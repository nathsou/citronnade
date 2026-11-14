//! Plays sounds loaded from WAV files in the RomFS. A, B, X and Y play the
//! chime, drum, chord and pluck, each on the next of 8 channels, panned by
//! the Circle Pad. L makes the pluck loop (until R stops everything).
//! START exits.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

use citronnade::audio::{Audio, SampleFormat, WaveBuf};
use citronnade::prelude::*;
use citronnade::romfs::RomFs;

struct Sound {
    name: &'static str,
    rate: u32,
    wave: Arc<WaveBuf>,
    looped: Arc<WaveBuf>,
}

/// Reads 16-bit PCM mono or stereo samples and the rate from a WAV file.
fn parse_wav(data: &[u8]) -> Option<(u32, u16, Vec<i16>)> {
    if data.get(0..4)? != b"RIFF" || data.get(8..12)? != b"WAVE" {
        return None;
    }
    let (mut rate, mut channels, mut samples) = (0, 0, None);
    let mut pos = 12;
    while pos + 8 <= data.len() {
        let id = &data[pos..pos + 4];
        let len = u32::from_le_bytes(data[pos + 4..pos + 8].try_into().ok()?) as usize;
        let body = data.get(pos + 8..pos + 8 + len)?;
        match id {
            b"fmt " => {
                channels = u16::from_le_bytes([body[2], body[3]]);
                rate = u32::from_le_bytes(body[4..8].try_into().ok()?);
                if u16::from_le_bytes([body[14], body[15]]) != 16 {
                    return None;
                }
            }
            b"data" => samples = Some(body.chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]])).collect()),
            _ => {}
        }
        pos += 8 + len + (len & 1);
    }
    Some((rate, channels, samples?))
}

#[citronnade::main]
fn main(sys: System) -> Result<()> {
    let System { apt, gsp, hid, .. } = sys;
    let mut screens = gsp.screens()?;
    let mut input = hid.input()?;
    let mut romfs = RomFs::mount().map_err(|e| {
        log!("no RomFS: {e}");
        citronnade::ResultCode(0xC8804464)
    })?;
    let mut sounds = Vec::new();
    for name in ["chime", "drum", "chord", "pluck"] {
        let data = romfs.read(&format!("/sounds/{name}.wav")).unwrap_or_default();
        match parse_wav(&data) {
            Some((rate, channels, samples)) => {
                log!("loaded {name}.wav: {} samples at {rate} Hz", samples.len());
                let wave = Arc::new(WaveBuf::from_samples(&samples, channels as u8));
                let mut looped = WaveBuf::from_samples(&samples, channels as u8);
                looped.set_looping(true);
                sounds.push(Sound { name, rate, wave, looped: Arc::new(looped) });
            }
            None => log!("could not read {name}.wav"),
        }
    }
    let audio = match Audio::new() {
        Ok(a) => Some(a),
        Err(e) => {
            log!("no audio: {e}");
            None
        }
    };
    let mut next_channel = 0usize;
    let mut looping = false;
    let mut last = String::from("-");

    while apt.main_loop() {
        let i = input.scan();
        let down = i.down();
        if down.contains(Buttons::START) {
            break;
        }
        if down.contains(Buttons::L) {
            looping = !looping;
            log!("looping: {looping}");
        }
        let pan = i.circle_pad().0 as f32 / 156.0;
        let pressed = [Buttons::A, Buttons::B, Buttons::X, Buttons::Y];
        for (k, b) in pressed.iter().enumerate() {
            if down.contains(*b) {
                if let (Some(a), Some(s)) = (&audio, sounds.get(k)) {
                    let ch = a.channel(next_channel);
                    ch.reset();
                    ch.set_format(SampleFormat::Pcm16Mono);
                    ch.set_sample_rate(s.rate as f32);
                    ch.set_volume_pan(0.9, pan);
                    let wave = if looping && s.name == "pluck" { &s.looped } else { &s.wave };
                    ch.queue(wave);
                    log!("playing {} on channel {next_channel}, pan {:.2}", s.name, pan);
                    last = format!("{} (channel {next_channel})", s.name);
                    next_channel = (next_channel + 1) % 8;
                }
            }
        }
        if down.contains(Buttons::R) {
            if let Some(a) = &audio {
                for c in 0..8 {
                    a.channel(c).reset();
                }
                log!("stopped");
            }
        }

        let mut top = screens.top();
        top.clear(Color::rgb(20, 24, 36));
        top.text(10, 10, "Samples", Color::WHITE, 3);
        for (k, (b, name)) in ["A", "B", "X", "Y"].iter().zip(["chime", "drum", "chord", "pluck"]).enumerate() {
            top.text(20, 60 + k as i32 * 20, &format!("{b}: {name}"), Color::WHITE, 2);
        }
        top.text(20, 150, &format!("last: {last}"), Color::YELLOW, 1);
        top.text(20, 165, &format!("pluck loops: {looping} (L)   R: stop all"), Color::WHITE, 1);
        top.fill_rect(100, 200, 200, 8, Color::DARK_GRAY);
        top.fill_rect(196 + (pan * 96.0) as i32, 196, 8, 16, Color::CYAN);
        top.text(170, 215, "pan", Color::GRAY, 1);

        let mut bottom = screens.bottom();
        bottom.clear(Color::rgb(16, 18, 24));
        if let Some(a) = &audio {
            for c in 0..8 {
                let on = a.channel(c).is_playing();
                bottom.fill_rect(10 + c as i32 * 38, 20, 32, 32, if on { Color::GREEN } else { Color::DARK_GRAY });
                bottom.text(20 + c as i32 * 38, 58, &format!("{c}"), Color::WHITE, 1);
            }
        } else {
            bottom.text(10, 20, "No DSP firmware (dump it with DSP1)", Color::YELLOW, 1);
        }
        bottom.text(10, 220, "START: exit", Color::GRAY, 1);
        screens.present();
    }
    drop(audio);
    log!("bye");
    Ok(())
}
