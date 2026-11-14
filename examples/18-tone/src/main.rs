//! A synthesiser: a stream of wave buffers refilled as the DSP finishes
//! them. A changes the waveform, the Circle Pad bends the pitch, Up/Down
//! change the volume, START exits.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use alloc::sync::Arc;
use alloc::vec::Vec;

use citronnade::audio::{Audio, SampleFormat, WaveBuf, OUTPUT_RATE};
use citronnade::math::{exp, sin};
use citronnade::prelude::*;

const NAMES: [&str; 4] = ["sine", "square", "sawtooth", "noise"];
const BUFFER_FRAMES: usize = 1024;

struct Osc {
    phase: f32,
    noise: u32,
}

impl Osc {
    fn sample(&mut self, wave: usize, freq: f32) -> f32 {
        self.phase += freq / OUTPUT_RATE;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
        }
        match wave {
            0 => sin(self.phase * core::f32::consts::TAU),
            1 => if self.phase < 0.5 { 0.6 } else { -0.6 },
            2 => 0.8 * (2.0 * self.phase - 1.0),
            _ => {
                self.noise ^= self.noise << 13;
                self.noise ^= self.noise >> 17;
                self.noise ^= self.noise << 5;
                (self.noise as i32 as f32 / i32::MAX as f32) * 0.5
            }
        }
    }
}

#[citronnade::main]
fn main(sys: System) -> Result<()> {
    let System { apt, gsp, hid, .. } = sys;
    let mut screens = gsp.screens()?;
    let mut input = hid.input()?;
    let audio = match Audio::new() {
        Ok(a) => {
            log!("tone: audio started");
            Some(a)
        }
        Err(e) => {
            log!("tone: no audio: {e}");
            None
        }
    };
    let mut buffers: Vec<Arc<WaveBuf>> = (0..4).map(|_| Arc::new(WaveBuf::new(BUFFER_FRAMES, 1))).collect();
    if let Some(a) = &audio {
        let ch = a.channel(0);
        ch.set_format(SampleFormat::Pcm16Mono);
        ch.set_sample_rate(OUTPUT_RATE);
        ch.set_volume(0.8, 0.8);
    }
    let mut osc = Osc { phase: 0.0, noise: 0x9E37_79B9 };
    let mut wave = 0usize;
    let mut volume = 0.6f32;
    let mut frame = 0u32;
    let mut scope = [0i16; 400];

    while apt.main_loop() {
        let i = input.scan();
        let down = i.down();
        if down.contains(Buttons::START) {
            break;
        }
        if down.contains(Buttons::A) {
            wave = (wave + 1) % NAMES.len();
            log!("waveform: {}", NAMES[wave]);
        }
        if i.held().intersects(Buttons::DPAD_UP) {
            volume = (volume + 0.01).min(1.0);
        }
        if i.held().intersects(Buttons::DPAD_DOWN) {
            volume = (volume - 0.01).max(0.0);
        }
        // Pitch: 440 Hz at rest, an octave up or down at the Circle Pad's ends.
        let bend = i.circle_pad().1 as f32 / 156.0;
        let freq = 440.0 * exp(bend * core::f32::consts::LN_2);

        if let Some(a) = &audio {
            let ch = a.channel(0);
            for buf in buffers.iter_mut() {
                if let Some(b) = Arc::get_mut(buf) {
                    if b.is_free() {
                        for s in b.samples_mut().iter_mut() {
                            *s = (osc.sample(wave, freq) * volume * 32767.0) as i16;
                        }
                        let n = scope.len().min(BUFFER_FRAMES);
                        scope[..n].copy_from_slice(&b.samples()[..n]);
                        ch.queue(buf);
                    }
                }
            }
            if frame % 60 == 59 {
                log!("audio frames: {}, dropped: {}", a.frame_count(), a.dropped_frames());
            }
        }

        let mut top = screens.top();
        top.clear(Color::rgb(14, 20, 30));
        top.fill_rect(0, 120, 400, 1, Color::rgb(60, 70, 90));
        for x in 1..400 {
            let y0 = 120 - scope[x - 1] as i32 / 400;
            let y1 = 120 - scope[x] as i32 / 400;
            top.line(x as i32 - 1, y0, x as i32, y1, Color::rgb(120, 230, 160));
        }
        top.text(10, 10, &format!("{} {:.0} Hz", NAMES[wave], freq), Color::WHITE, 2);

        let mut bottom = screens.bottom();
        bottom.clear(Color::rgb(16, 18, 24));
        match &audio {
            Some(_) => bottom.text(10, 10, "Playing", Color::GREEN, 2),
            None => bottom.text(10, 10, "No DSP firmware: dump it with DSP1", Color::YELLOW, 1),
        };
        bottom.fill_rect(10, 50, 300, 14, Color::DARK_GRAY);
        bottom.fill_rect(10, 50, (300.0 * volume) as u32, 14, Color::CYAN);
        bottom.text(10, 70, "volume (Up/Down)", Color::WHITE, 1);
        bottom.text(10, 200, "A: waveform  Circle Pad: pitch", Color::GRAY, 1);
        bottom.text(10, 220, "START: exit", Color::GRAY, 1);
        screens.present();
        frame += 1;
    }
    drop(audio);
    log!("bye");
    Ok(())
}
