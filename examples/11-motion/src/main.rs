//! Tilt the console to roll a ball (accelerometer); the gyroscope's rates
//! are shown as bars. START exits.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use citronnade::prelude::*;

#[citronnade::main]
fn main(sys: System) -> Result<()> {
    let System { apt, gsp, hid, .. } = sys;
    let mut screens = gsp.screens()?;
    let mut input = hid.input()?;
    input.enable_motion()?;
    log!("motion: sensors enabled");
    let (mut x, mut y, mut vx, mut vy) = (200.0f32, 120.0f32, 0.0f32, 0.0f32);
    let mut frame = 0u32;

    while apt.main_loop() {
        let i = input.scan();
        if i.down().contains(Buttons::START) {
            break;
        }
        let (ax, ay, az) = i.accelerometer();
        let (gx, gy, gz) = i.gyroscope();
        if frame % 60 == 0 {
            log!("accelerometer: ({ax}, {ay}, {az})  gyroscope: ({gx:.1}, {gy:.1}, {gz:.1}) deg/s");
        }
        // About 512 units per g; screen x follows the console's x axis.
        vx = (vx + ax as f32 / 512.0 * 0.6) * 0.98;
        vy = (vy - az as f32 / 512.0 * 0.6) * 0.98;
        x += vx;
        y += vy;
        if !(14.0..=386.0).contains(&x) {
            vx = -vx * 0.6;
            x = x.clamp(14.0, 386.0);
        }
        if !(14.0..=226.0).contains(&y) {
            vy = -vy * 0.6;
            y = y.clamp(14.0, 226.0);
        }

        let mut top = screens.top();
        top.clear(Color::rgb(30, 70, 50));
        for k in 0..10 {
            top.rect(k * 40, 0, 40, 240, Color::rgb(36, 80, 58));
        }
        top.fill_circle(x as i32 + 3, y as i32 + 4, 13, Color::rgba(0, 0, 0, 90));
        top.fill_circle(x as i32, y as i32, 13, Color::rgb(240, 200, 60));
        top.fill_circle(x as i32 - 4, y as i32 - 4, 4, Color::rgb(255, 245, 200));

        let mut bottom = screens.bottom();
        bottom.clear(Color::rgb(16, 18, 24));
        bottom.text(10, 10, "Accelerometer", Color::WHITE, 2);
        bottom.text(10, 34, &format!("x {ax:5}  y {ay:5}  z {az:5}"), Color::WHITE, 1);
        bottom.text(10, 60, "Gyroscope (deg/s)", Color::WHITE, 2);
        for (k, (name, v)) in [("x", gx), ("y", gy), ("z", gz)].iter().enumerate() {
            let yy = 86 + k as i32 * 24;
            bottom.text(10, yy + 4, name, Color::WHITE, 1);
            bottom.fill_rect(160, yy, 1, 16, Color::GRAY);
            let w = (v.abs() / 2.0).min(140.0) as u32;
            let x0 = if *v < 0.0 { 160 - w as i32 } else { 160 };
            bottom.fill_rect(x0, yy, w, 16, Color::CYAN);
        }
        bottom.text(10, 220, "Tilt to roll the ball.  START: exit", Color::GRAY, 1);
        screens.present();
        frame += 1;
    }
    log!("bye");
    Ok(())
}
