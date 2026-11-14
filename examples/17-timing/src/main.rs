//! Timing: the system tick, how accurately the thread sleeps, and how evenly
//! frames are paced by VBlank (shown as a graph of frame intervals). START
//! exits.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use core::time::Duration;

use citronnade::prelude::*;
use citronnade::time::{self, Instant, TICKS_PER_SECOND};

#[citronnade::main]
fn main(sys: System) -> Result<()> {
    let System { apt, gsp, hid, .. } = sys;
    let mut screens = gsp.screens()?;
    let mut input = hid.input()?;
    log!("timing: tick rate {} Hz, uptime {} ms", TICKS_PER_SECOND, time::millis());

    let mut sleep_results = [(0u64, 0u64); 3];
    for (k, ms) in [1u64, 5, 16].iter().enumerate() {
        let mut total = 0u64;
        let mut worst = 0u64;
        for _ in 0..10 {
            let t = Instant::now();
            time::sleep(Duration::from_millis(*ms));
            let us = t.elapsed().as_micros() as u64;
            total += us;
            worst = worst.max(us);
        }
        sleep_results[k] = (total / 10, worst);
        log!("sleep {ms} ms: {} us on average, {} us at worst", total / 10, worst);
    }

    let mut intervals = [0u32; 200];
    let mut last = Instant::now();
    let mut frames = 0u32;
    let mut sum = 0u64;
    let (mut min, mut max) = (u32::MAX, 0u32);
    while apt.main_loop() {
        if input.scan().down().contains(Buttons::START) {
            break;
        }
        let now = Instant::now();
        let dt = now.duration_since(last).as_micros() as u32;
        last = now;
        intervals.copy_within(1.., 0);
        intervals[199] = dt;
        frames += 1;
        if frames > 1 && frames <= 121 {
            sum += dt as u64;
            min = min.min(dt);
            max = max.max(dt);
            if frames == 121 {
                log!("frame interval over 120 frames: {} us on average (min {min}, max {max})", sum / 120);
            }
        }

        let mut top = screens.top();
        top.clear(Color::rgb(20, 22, 30));
        top.text(10, 8, "Frame intervals (us)", Color::WHITE, 2);
        let base = 200;
        top.fill_rect(0, base - 100, 400, 1, Color::rgba(255, 255, 255, 60));
        for (x, &dt) in intervals.iter().enumerate() {
            let h = (dt / 200).min(180);
            top.fill_rect(x as i32 * 2, base - h as i32, 2, h, if dt > 17_500 { Color::RED } else { Color::GREEN });
        }
        top.text(10, 210, "line: 20 ms", Color::GRAY, 1);

        let mut bottom = screens.bottom();
        bottom.clear(Color::rgb(16, 18, 24));
        bottom.text(10, 10, &format!("uptime {:.3} s", time::micros() as f32 / 1e6), Color::WHITE, 2);
        for (k, ms) in [1, 5, 16].iter().enumerate() {
            let (avg, worst) = sleep_results[k];
            bottom.text(10, 50 + k as i32 * 14, &format!("sleep {ms:2} ms: avg {avg} us, worst {worst} us"), Color::WHITE, 1);
        }
        bottom.text(10, 220, "START: exit", Color::GRAY, 1);
        screens.present();
    }
    log!("bye");
    Ok(())
}
