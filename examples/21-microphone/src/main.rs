//! The microphone: a live level meter and waveform. A records three
//! seconds, B plays the recording back, START exits.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use alloc::sync::Arc;
use alloc::vec::Vec;

use citronnade::audio::{Audio, SampleFormat, WaveBuf};
use citronnade::mic::{Microphone, Rate};
use citronnade::prelude::*;

#[citronnade::main]
fn main(sys: System) -> Result<()> {
    let System { apt, gsp, hid, .. } = sys;
    let mut screens = gsp.screens()?;
    let mut input = hid.input()?;
    let mut mic = Microphone::new(Rate::Hz16360, 0x30000)?;
    log!("microphone: sampling at {} Hz", mic.rate().hz());
    let audio = Audio::new().map_err(|e| log!("no audio: {e}")).ok();
    let target = 3 * mic.rate().hz() as usize;
    let mut fresh = Vec::new();
    let mut recording: Option<Vec<i16>> = None;
    let mut recorded: Option<Arc<WaveBuf>> = None;
    let mut wave = [0i16; 320];

    while apt.main_loop() {
        let down = input.scan().down();
        if down.contains(Buttons::START) {
            break;
        }
        fresh.clear();
        mic.read(&mut fresh);
        for (k, s) in fresh.iter().rev().take(320).enumerate() {
            wave[319 - k] = *s;
        }
        if down.contains(Buttons::A) && recording.is_none() {
            recording = Some(Vec::with_capacity(target));
            log!("recording 3 seconds");
        }
        if let Some(rec) = recording.as_mut() {
            rec.extend_from_slice(&fresh);
            if rec.len() >= target {
                rec.truncate(target);
                let peak = rec.iter().map(|s| s.unsigned_abs()).max().unwrap_or(0);
                log!("recorded {} samples, peak {peak}", rec.len());
                recorded = Some(Arc::new(WaveBuf::from_samples(rec, 1)));
                recording = None;
            }
        }
        if down.contains(Buttons::B) {
            if let (Some(a), Some(buf)) = (&audio, &recorded) {
                let ch = a.channel(0);
                ch.reset();
                ch.set_format(SampleFormat::Pcm16Mono);
                ch.set_sample_rate(mic.rate().hz() as f32);
                ch.set_volume(1.0, 1.0);
                ch.queue(buf);
                log!("playing back");
            }
        }
        let level = mic.level(800);

        let mut top = screens.top();
        top.clear(Color::rgb(14, 18, 26));
        top.text(10, 10, "Microphone", Color::WHITE, 3);
        top.fill_rect(10, 60, 380, 20, Color::DARK_GRAY);
        top.fill_rect(10, 60, (level as u32 * 380 / 32768).min(380), 20, if level > 24000 { Color::RED } else { Color::GREEN });
        for x in 1..320 {
            top.line(40 + x as i32 - 1, 160 - wave[x - 1] as i32 / 512, 40 + x as i32, 160 - wave[x] as i32 / 512, Color::CYAN);
        }
        let status = match (&recording, &recorded) {
            (Some(r), _) => format!("recording... {:.1} s", r.len() as f32 / mic.rate().hz() as f32),
            (None, Some(_)) => String::from("recorded 3 s: B plays it"),
            _ => String::from("A: record 3 seconds"),
        };
        top.text(10, 220, &status, Color::YELLOW, 1);

        let mut bottom = screens.bottom();
        bottom.clear(Color::rgb(16, 18, 24));
        bottom.text(10, 10, &format!("level {level}"), Color::WHITE, 2);
        bottom.text(10, 220, "A: record  B: play  START: exit", Color::GRAY, 1);
        screens.present();
    }
    drop(audio);
    log!("bye");
    Ok(())
}

use alloc::string::String;
