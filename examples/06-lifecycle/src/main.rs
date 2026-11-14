//! A long-running program for checking the application lifecycle on
//! hardware: press HOME and come back, close the lid, press the power
//! button. Every APT event is logged and shown.
//!
//! Y toggles whether sleep is allowed, X whether HOME is allowed,
//! START exits.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use core::sync::atomic::{AtomicU32, Ordering};

use citronnade::prelude::*;
use citronnade::Hook;

static SUSPENDS: AtomicU32 = AtomicU32::new(0);
static RESTORES: AtomicU32 = AtomicU32::new(0);
static SLEEPS: AtomicU32 = AtomicU32::new(0);
static WAKEUPS: AtomicU32 = AtomicU32::new(0);

fn on_hook(hook: Hook) {
    log!("lifecycle event: {:?}", hook);
    match hook {
        Hook::Suspend => &SUSPENDS,
        Hook::Restore => &RESTORES,
        Hook::Sleep => &SLEEPS,
        Hook::Wakeup => &WAKEUPS,
        Hook::Exit => return,
    }
    .fetch_add(1, Ordering::Relaxed);
}

#[citronnade::main]
fn main(sys: System) -> Result<()> {
    let System { apt, gsp, hid, .. } = sys;
    apt.on(on_hook);
    let mut screens = gsp.screens()?;
    let mut input = hid.input()?;
    let mut sleep_allowed = true;
    let mut home_allowed = true;
    let mut frame: u64 = 0;
    log!("lifecycle: running (Y: sleep, X: HOME, START: exit)");

    while apt.main_loop() {
        let down = input.scan().down();
        if down.contains(Buttons::START) {
            break;
        }
        if down.contains(Buttons::Y) {
            sleep_allowed = !sleep_allowed;
            apt.set_sleep_allowed(sleep_allowed);
            log!("sleep allowed: {sleep_allowed}");
        }
        if down.contains(Buttons::X) {
            home_allowed = !home_allowed;
            apt.set_home_allowed(home_allowed);
            log!("HOME allowed: {home_allowed}");
        }
        frame += 1;
        if frame % 120 == 0 {
            log!("frame {frame}");
        }

        let mut top = screens.top();
        top.clear(Color::rgb(20, 50, 40));
        let pulse = (frame % 120) as i32;
        top.fill_circle(200, 120, 20 + pulse.min(120 - pulse) / 2, Color::rgb(90, 220, 140));
        top.text(10, 10, &format!("frame {frame}"), Color::WHITE, 2);

        let mut bottom = screens.bottom();
        bottom.clear(Color::rgb(16, 18, 24));
        let lines = [
            format!("HOME and back: {} / {}", SUSPENDS.load(Ordering::Relaxed), RESTORES.load(Ordering::Relaxed)),
            format!("sleep / wake:  {} / {}", SLEEPS.load(Ordering::Relaxed), WAKEUPS.load(Ordering::Relaxed)),
            format!("Y  sleep allowed: {sleep_allowed}"),
            format!("X  HOME allowed:  {home_allowed}"),
            format!("New 3DS: {}", apt.is_new_3ds()),
        ];
        for (i, l) in lines.iter().enumerate() {
            bottom.text(10, 12 + i as i32 * 20, l, Color::WHITE, 1);
        }
        bottom.text(10, 220, "START: exit", Color::GRAY, 1);
        screens.present();
    }
    log!("exiting after {frame} frames");
    Ok(())
}
