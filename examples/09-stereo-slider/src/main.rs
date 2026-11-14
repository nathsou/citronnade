//! Stereoscopic 3D: a scene of layers (sky, far hills, near hills, trees, a
//! floating slice of citrus) drawn once per eye, each layer shifted by its
//! depth times the 3D slider. Slider down (or a 2DS) gives a flat picture.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use citronnade::math::{sin, sqrt};
use citronnade::prelude::*;
use citronnade::{info, slider_3d};

/// Maximum separation between the eyes' images, in pixels, at full slider.
const MAX_SHIFT: f32 = 12.0;

/// Parallax per layer: negative is behind the screen, positive in front.
const DEPTHS: [f32; 5] = [-1.0, -0.6, -0.25, 0.1, 0.8];

fn hills(canvas: &mut Canvas<'_>, shift: i32, base: i32, amp: f32, freq: f32, phase: f32, color: Color) {
    for x in -16..416 {
        let y = base + (amp * sin(x as f32 * freq + phase)) as i32;
        canvas.fill_rect(x + shift, y, 1, (240 - y).max(0) as u32, color);
    }
}

fn tree(canvas: &mut Canvas<'_>, x: i32, y: i32, size: i32) {
    canvas.fill_rect(x - 2, y, 5, (size / 2) as u32, Color::rgb(110, 70, 40));
    for row in 0..size {
        let half = row * 2 / 3;
        canvas.fill_rect(x - half, y - size + row, (2 * half + 1) as u32, 1, Color::rgb(40, 140 - row as u8, 70));
    }
}

fn slice(canvas: &mut Canvas<'_>, cx: i32, cy: i32, r: i32) {
    for dy in -r..=r {
        for dx in -r..=r {
            let d = sqrt((dx * dx + dy * dy) as f32) / r as f32;
            if d > 1.0 {
                continue;
            }
            let c = if d > 0.86 {
                Color::rgb(250, 200, 30)
            } else if d > 0.76 {
                Color::rgb(255, 250, 230)
            } else if dx.abs() < 2 || dy.abs() < 2 || (dx - dy).abs() < 2 || (dx + dy).abs() < 2 {
                Color::rgb(255, 245, 210)
            } else {
                Color::rgb(255, 230, 90)
            };
            canvas.set(cx + dx, cy + dy, c);
        }
    }
}

fn draw_eye(canvas: &mut Canvas<'_>, eye: f32, slider: f32, t: f32) {
    let shift = |layer: usize| (eye * DEPTHS[layer] * slider * MAX_SHIFT / 2.0) as i32;
    canvas.vertical_gradient(0, 0, 400, 240, Color::rgb(90, 150, 230), Color::rgb(200, 225, 250));
    hills(canvas, shift(0), 120, 18.0, 0.015, 0.0, Color::rgb(120, 140, 190));
    hills(canvas, shift(1), 150, 14.0, 0.03, 1.5, Color::rgb(90, 160, 110));
    hills(canvas, shift(2), 185, 10.0, 0.05, 3.0, Color::rgb(60, 130, 70));
    for (i, x) in [40, 120, 210, 300, 370].iter().enumerate() {
        tree(canvas, x + shift(3), 205 + (i as i32 % 2) * 8, 34 + (i as i32 % 3) * 6);
    }
    let bob = (sin(t * 2.0) * 6.0) as i32;
    slice(canvas, 200 + shift(4), 90 + bob, 34);
    canvas.text(150 + shift(4), 140 + bob, "3D", Color::rgb(60, 40, 10), 4);
}

#[citronnade::main]
fn main(sys: System) -> Result<()> {
    let System { apt, gsp, hid, .. } = sys;
    let mut screens = gsp.screens()?;
    let mut input = hid.input()?;
    let model = info::model().unwrap_or(info::Model::Old3ds);
    if model.has_stereo() {
        screens.set_top_mode(TopMode::Stereo);
    }
    log!("stereo-slider: {} ({})", model.name(), if model.has_stereo() { "stereo" } else { "no 3D screen" });
    let mut last_slider = -1.0f32;
    let mut frame = 0u32;

    while apt.main_loop() {
        if input.scan().down().contains(Buttons::START) {
            break;
        }
        let slider = slider_3d();
        if (slider - last_slider).abs() > 0.02 {
            log!("slider: {:.2}", slider);
            last_slider = slider;
        }
        let t = frame as f32 / 60.0;
        if screens.top_mode() == TopMode::Stereo {
            let (mut left, mut right) = screens.top_stereo();
            draw_eye(&mut left, -1.0, slider, t);
            draw_eye(&mut right, 1.0, slider, t);
        } else {
            draw_eye(&mut screens.top(), 0.0, 0.0, t);
        }

        let mut bottom = screens.bottom();
        bottom.clear(Color::rgb(16, 18, 24));
        bottom.text(10, 10, "Stereoscopic 3D", Color::WHITE, 2);
        bottom.text(10, 40, "Move the 3D slider to change the depth.", Color::WHITE, 1);
        bottom.fill_rect(10, 70, 300, 16, Color::DARK_GRAY);
        bottom.fill_rect(10, 70, (300.0 * slider) as u32, 16, Color::CYAN);
        bottom.text(10, 94, &format!("slider {:.2}", slider), Color::WHITE, 2);
        if !model.has_stereo() {
            bottom.text(10, 130, "This model has no 3D screen.", Color::YELLOW, 1);
        }
        bottom.text(10, 220, "START: exit", Color::GRAY, 1);
        screens.present();
        frame += 1;
    }
    log!("bye");
    Ok(())
}
