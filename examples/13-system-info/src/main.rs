//! Everything the system tells about itself: model, region, language, the
//! user's name, power, the volume slider, Wi-Fi, versions and the clock.
//! START exits.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use citronnade::prelude::*;
use citronnade::{info, slider_3d, time};

fn fmt_err<T: core::fmt::Debug>(r: Result<T>) -> String {
    match r {
        Ok(v) => format!("{v:?}"),
        Err(e) => format!("error {:#010x}", e.0),
    }
}

fn lines() -> Vec<String> {
    let now = time::now();
    let model = info::model();
    let (k1, k2, k3) = info::kernel_version();
    let (f1, f2, f3) = info::firmware_version();
    const DAYS: [&str; 7] = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];
    alloc::vec![
        format!("model: {}", model.map(|m| m.name()).unwrap_or("?")),
        format!("region: {}", info::region().unwrap_or("?")),
        format!("language: {}", info::language().unwrap_or("?")),
        format!("user: {}", info::username().unwrap_or_default()),
        format!("battery: {}", fmt_err(info::battery().map(|(l, c, a)| format!("{l}/5, charging {c}, adapter {a}")))),
        format!("volume: {}", fmt_err(info::volume())),
        format!("Wi-Fi strength: {}/3", info::wifi_strength()),
        format!("headphones: {}", info::headphones()),
        format!("3D slider: {:.2}", slider_3d()),
        format!("kernel: {k1}.{k2}.{k3}  firmware: {f1}.{f2}.{f3}"),
        format!(
            "time: {:04}-{:02}-{:02} {:02}:{:02}:{:02} ({})",
            now.year,
            now.month,
            now.day,
            now.hour,
            now.minute,
            now.second,
            DAYS[now.weekday as usize % 7]
        ),
        format!("uptime: {} s", time::millis() / 1000),
    ]
}

#[citronnade::main]
fn main(sys: System) -> Result<()> {
    let System { apt, gsp, hid, .. } = sys;
    let mut screens = gsp.screens()?;
    let mut input = hid.input()?;
    for l in lines() {
        log!("{l}");
    }
    let mut frame = 0u32;
    let mut cached = lines();
    while apt.main_loop() {
        if input.scan().down().contains(Buttons::START) {
            break;
        }
        if frame % 30 == 0 {
            cached = lines();
        }
        let mut top = screens.top();
        top.clear(Color::rgb(24, 30, 44));
        top.text(12, 10, "System information", Color::WHITE, 2);
        for (i, l) in cached.iter().enumerate() {
            top.text(12, 40 + i as i32 * 15, l, if i % 2 == 0 { Color::WHITE } else { Color::rgb(190, 210, 240) }, 1);
        }
        let mut bottom = screens.bottom();
        bottom.clear(Color::rgb(16, 18, 24));
        let now = time::now();
        bottom.text(40, 90, &format!("{:02}:{:02}:{:02}", now.hour, now.minute, now.second), Color::WHITE, 5);
        bottom.text(10, 220, "START: exit", Color::GRAY, 1);
        screens.present();
        frame += 1;
    }
    log!("bye");
    Ok(())
}
