//! A two-octave keyboard on the bottom screen. A touched key plays a looping
//! single-cycle wave on its own channel until released; A, B, X and Y hold
//! notes of a chord at the same time (polyphony). The time from touch to
//! the DSP reporting the channel as playing is shown as the latency.
//! L/R change the timbre, START exits.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use alloc::sync::Arc;
use alloc::vec::Vec;

use citronnade::audio::{Audio, Interpolation, SampleFormat, WaveBuf};
use citronnade::math::{exp, sin};
use citronnade::prelude::*;
use citronnade::time::Instant;

const CYCLE: usize = 64;
const WHITE_KEYS: usize = 14;
const NAMES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
const WHITE_SEMITONES: [i32; 7] = [0, 2, 4, 5, 7, 9, 11];
const BLACK_AFTER: [bool; 7] = [true, true, false, true, true, true, false];

fn freq(semitone: i32) -> f32 {
    // Semitones from C4 (261.63 Hz).
    261.63 * exp(semitone as f32 / 12.0 * core::f32::consts::LN_2)
}

fn name(semitone: i32) -> alloc::string::String {
    format!("{}{}", NAMES[semitone.rem_euclid(12) as usize], 4 + semitone.div_euclid(12))
}

fn timbre(kind: usize) -> Arc<WaveBuf> {
    let mut b = WaveBuf::new(CYCLE, 1);
    for (i, s) in b.samples_mut().iter_mut().enumerate() {
        let t = i as f32 / CYCLE as f32;
        let v = match kind {
            0 => sin(t * core::f32::consts::TAU) * 0.8,
            1 => 0.6 * sin(t * core::f32::consts::TAU) + 0.3 * sin(2.0 * t * core::f32::consts::TAU) + 0.15 * sin(3.0 * t * core::f32::consts::TAU),
            2 => (if t < 0.5 { 4.0 * t - 1.0 } else { 3.0 - 4.0 * t }) * 0.8,
            _ => if t < 0.25 { 0.5 } else { -0.5 },
        };
        *s = (v * 30000.0) as i16;
    }
    b.set_looping(true);
    Arc::new(b)
}

fn key_at(x: u16, y: u16) -> Option<i32> {
    let w = 320.0 / WHITE_KEYS as f32;
    let white = ((x as f32 / w) as usize).min(WHITE_KEYS - 1);
    let (octave, k) = (white / 7, white % 7);
    if y < 150 {
        // Black keys sit over the boundary between white keys.
        let frac = x as f32 / w - white as f32;
        if frac > 0.7 && BLACK_AFTER[k] && white + 1 < WHITE_KEYS {
            return Some(octave as i32 * 12 + WHITE_SEMITONES[k] + 1);
        }
        if frac < 0.3 && k > 0 && BLACK_AFTER[k - 1] {
            return Some(octave as i32 * 12 + WHITE_SEMITONES[k] - 1);
        }
    }
    if y >= 100 { Some(octave as i32 * 12 + WHITE_SEMITONES[k]) } else { None }
}

#[citronnade::main]
fn main(sys: System) -> Result<()> {
    let System { apt, gsp, hid, .. } = sys;
    let mut screens = gsp.screens()?;
    let mut input = hid.input()?;
    let audio = match Audio::new() {
        Ok(a) => {
            log!("touch-synth: audio started");
            Some(a)
        }
        Err(e) => {
            log!("touch-synth: no audio: {e}");
            None
        }
    };
    let timbres: Vec<Arc<WaveBuf>> = (0..4).map(timbre).collect();
    let mut kind = 1usize;
    let mut touch_note: Option<i32> = None;
    let mut touch_started: Option<Instant> = None;
    let mut latency_ms = 0.0f32;
    let chord = [(Buttons::A, 0), (Buttons::B, 4), (Buttons::X, 7), (Buttons::Y, 12)];
    let mut chord_on = [false; 4];

    let start_voice = |audio: &Audio, ch: usize, semitone: i32, wave: &Arc<WaveBuf>| {
        let c = audio.channel(ch);
        c.reset();
        c.set_format(SampleFormat::Pcm16Mono);
        c.set_interpolation(Interpolation::Linear);
        c.set_sample_rate(freq(semitone) * CYCLE as f32);
        c.set_volume(0.5, 0.5);
        c.queue(wave);
    };

    while apt.main_loop() {
        let i = input.scan();
        let (down, held) = (i.down(), i.held());
        if down.contains(Buttons::START) {
            break;
        }
        if down.contains(Buttons::R) {
            kind = (kind + 1) % timbres.len();
        }
        if down.contains(Buttons::L) {
            kind = (kind + timbres.len() - 1) % timbres.len();
        }
        if let Some(a) = &audio {
            let now_note = i.touch().and_then(|(x, y)| key_at(x, y));
            if now_note != touch_note {
                match now_note {
                    Some(n) => {
                        start_voice(a, 0, n, &timbres[kind]);
                        touch_started = Some(Instant::now());
                        log!("note {} on", name(n));
                    }
                    None => a.channel(0).reset(),
                }
                touch_note = now_note;
            }
            if let Some(t) = touch_started {
                if a.channel(0).is_playing() {
                    latency_ms = t.elapsed().as_micros() as f32 / 1000.0;
                    log!("latency: {latency_ms:.1} ms");
                    touch_started = None;
                }
            }
            for (k, (b, semis)) in chord.iter().enumerate() {
                let on = held.contains(*b);
                if on && !chord_on[k] {
                    start_voice(a, 1 + k, *semis, &timbres[kind]);
                    if chord_on.iter().filter(|x| **x).count() >= 1 {
                        log!("chord: {} voices", chord_on.iter().filter(|x| **x).count() + 1);
                    }
                } else if !on && chord_on[k] {
                    a.channel(1 + k).reset();
                }
                chord_on[k] = on;
            }
        }

        let mut bottom = screens.bottom();
        bottom.clear(Color::rgb(30, 30, 36));
        let w = 320 / WHITE_KEYS as i32;
        for k in 0..WHITE_KEYS {
            let semis = (k / 7) as i32 * 12 + WHITE_SEMITONES[k % 7];
            let lit = touch_note == Some(semis);
            bottom.fill_rect(k as i32 * w + 1, 100, (w - 2) as u32, 138, if lit { Color::YELLOW } else { Color::rgb(240, 240, 235) });
        }
        for k in 0..WHITE_KEYS - 1 {
            if BLACK_AFTER[k % 7] {
                let semis = (k / 7) as i32 * 12 + WHITE_SEMITONES[k % 7] + 1;
                let lit = touch_note == Some(semis);
                bottom.fill_rect(k as i32 * w + w * 2 / 3, 100, (w * 2 / 3) as u32, 50, if lit { Color::YELLOW } else { Color::rgb(20, 20, 24) });
            }
        }
        bottom.text(10, 10, "Touch the keys", Color::WHITE, 2);
        bottom.text(10, 40, "A B X Y: chord   L/R: timbre", Color::GRAY, 1);

        let mut top = screens.top();
        top.clear(Color::rgb(16, 22, 34));
        top.text(10, 10, "Touch Synth", Color::WHITE, 3);
        let wave = &timbres[kind];
        for x in 0..CYCLE {
            let v = wave.samples()[x] as i32 / 400;
            top.fill_rect(80 + x as i32 * 4, 120 - v.max(0), 3, v.unsigned_abs().max(1), Color::CYAN);
        }
        top.text(10, 200, &format!("latency {latency_ms:.1} ms   note {}", touch_note.map(name).unwrap_or_default()), Color::WHITE, 1);
        if audio.is_none() {
            top.text(10, 220, "No DSP firmware: dump it with DSP1", Color::YELLOW, 1);
        }
        screens.present();
    }
    drop(audio);
    log!("bye");
    Ok(())
}
