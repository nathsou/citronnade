//! A live preview. The two outer cameras form a stereo pair shown in 3D on
//! the top screen (raise the 3D slider); A switches to the inner camera, B
//! to the inner camera through the hardware YUV converter. X swaps the eyes
//! in case the pair looks inside out. At startup the converter turns a
//! colour-bar test pattern into RGB to check itself. START exits.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use alloc::vec::Vec;
use citronnade::camera::{CameraSelect, Cameras, PixelFormat, Y2r, HEIGHT, WIDTH};
use citronnade::prelude::*;
use citronnade::time::Instant;

/// YUV (BT.601, full range) of 8 colour bars and their RGB.
const BARS: [((u8, u8, u8), (u8, u8, u8)); 8] = [
    ((255, 128, 128), (255, 255, 255)),
    ((226, 0, 149), (255, 255, 0)),
    ((179, 171, 0), (0, 255, 255)),
    ((150, 44, 21), (0, 255, 0)),
    ((105, 212, 235), (255, 0, 255)),
    ((76, 85, 255), (255, 0, 0)),
    ((29, 255, 107), (0, 0, 255)),
    ((0, 128, 128), (0, 0, 0)),
];

/// Converts colour bars with the hardware and compares.
fn y2r_self_test(y2r: &mut Y2r) -> core::result::Result<(), alloc::string::String> {
    let (w, h) = (64u16, 8u16);
    let mut yuyv = Vec::with_capacity(w as usize * h as usize * 2);
    for _ in 0..h {
        for x in (0..w).step_by(2) {
            let ((y, u, v), _) = BARS[(x / 8) as usize];
            yuyv.extend_from_slice(&[y, u, y, v]);
        }
    }
    let mut rgb = alloc::vec![0u8; w as usize * h as usize * 2];
    y2r.yuyv_to_rgb565(&yuyv, w, h, &mut rgb).map_err(|e| format!("conversion failed: {:?}", e))?;
    for (i, &(_, (r, g, b))) in BARS.iter().enumerate() {
        let at = 2 * (4 * w as usize + i * 8 + 4);
        let v = u16::from_le_bytes([rgb[at], rgb[at + 1]]);
        let got = (((v >> 11) << 3) as u8, (((v >> 5) & 0x3F) << 2) as u8, ((v & 0x1F) << 3) as u8);
        let close = |a: u8, b: u8| a.abs_diff(b) <= 24;
        if !(close(got.0, r) && close(got.1, g) && close(got.2, b)) {
            return Err(format!("bar {} is {:?}, expected {:?}", i, got, (r, g, b)));
        }
    }
    Ok(())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Stereo,
    Inner,
    InnerYuv,
}

#[citronnade::main]
fn main(sys: System) -> Result<()> {
    let System { apt, gsp, hid, .. } = sys;
    let mut screens = gsp.screens_with(Format::Rgb565, Format::Bgr8)?;
    let mut input = hid.input()?;
    let mut y2r = Y2r::new()?;
    match y2r_self_test(&mut y2r) {
        Ok(()) => log!("camera: y2r self-test ok"),
        Err(e) => log!("camera: y2r self-test failed: {}", e),
    }

    let mut mode = Mode::Stereo;
    let mut cams = Cameras::open(CameraSelect::OuterStereo, PixelFormat::Rgb565)?;
    log!("camera: outer stereo pair");
    screens.set_top_mode(TopMode::Stereo);
    let mut rgb = alloc::vec![0u8; (WIDTH * HEIGHT * 2) as usize];
    let mut swap = false;
    let mut shown = 0u32;
    let mut since = Instant::now();
    let (mut fps, mut counted) = (0.0f32, 0u32);
    let mut frame = 0u32;
    while apt.main_loop() {
        let down = input.scan().down();
        if down.contains(Buttons::START) {
            break;
        }
        let next = if down.contains(Buttons::A) {
            Some(Mode::Inner)
        } else if down.contains(Buttons::B) {
            Some(Mode::InnerYuv)
        } else if down.contains(Buttons::Y) {
            Some(Mode::Stereo)
        } else {
            None
        };
        if down.contains(Buttons::X) {
            swap = !swap;
        }
        if let Some(m) = next.filter(|m| *m != mode) {
            drop(cams);
            cams = match m {
                Mode::Stereo => Cameras::open(CameraSelect::OuterStereo, PixelFormat::Rgb565)?,
                Mode::Inner => Cameras::open(CameraSelect::Inner, PixelFormat::Rgb565)?,
                Mode::InnerYuv => Cameras::open(CameraSelect::Inner, PixelFormat::Yuyv)?,
            };
            screens.set_top_mode(if m == Mode::Stereo { TopMode::Stereo } else { TopMode::Mono });
            log!("camera: {}", match m {
                Mode::Stereo => "outer stereo pair",
                Mode::Inner => "inner",
                Mode::InnerYuv => "inner, YUV through y2r",
            });
            mode = m;
            shown = 0;
        }
        if cams.poll() {
            shown += 1;
            counted += 1;
            if shown == 10 {
                log!("frames arriving ({} so far, {} buffer errors)", cams.frames(0), cams.errors());
            }
        }
        if since.elapsed().as_millis() >= 1000 {
            fps = counted as f32 * 1000.0 / since.elapsed().as_millis() as f32;
            counted = 0;
            since = Instant::now();
        }
        if frame == 140 && shown == 0 {
            log!("no frames from the cameras");
        }

        match mode {
            Mode::Stereo => {
                let (mut left, mut right) = screens.top_stereo();
                let (l, r) = if swap { (1, 0) } else { (0, 1) };
                left.draw_rgb565(0, 0, WIDTH, HEIGHT, cams.frame(l));
                right.draw_rgb565(0, 0, WIDTH, HEIGHT, cams.frame(r));
            }
            Mode::Inner => screens.top().draw_rgb565(0, 0, WIDTH, HEIGHT, cams.frame(0)),
            Mode::InnerYuv => {
                if y2r.yuyv_to_rgb565(cams.frame(0), WIDTH as u16, HEIGHT as u16, &mut rgb).is_ok() {
                    screens.top().draw_rgb565(0, 0, WIDTH, HEIGHT, &rgb);
                }
            }
        }
        let mut b = screens.bottom();
        b.clear(Color::rgb(16, 16, 24));
        b.text(8, 8, "Camera", Color::WHITE, 2);
        let name = match mode {
            Mode::Stereo => "outer cameras, stereo (3D slider)",
            Mode::Inner => "inner camera, RGB565",
            Mode::InnerYuv => "inner camera, YUYV -> y2r -> RGB565",
        };
        b.text(8, 36, name, Color::rgb(200, 210, 230), 1);
        b.text(8, 50, &format!("{:.1} frames/s, {} errors", fps, cams.errors()), Color::rgb(200, 210, 230), 1);
        b.text(8, 178, "Y: outer stereo  A: inner  B: inner + y2r", Color::WHITE, 1);
        b.text(8, 192, &format!("X: swap eyes ({})", if swap { "swapped" } else { "normal" }), Color::WHITE, 1);
        b.text(8, 220, "START: exit", Color::WHITE, 1);
        screens.present();
        frame += 1;
    }
    drop(cams);
    log!("bye: {} frames shown in the last mode", shown);
    Ok(())
}
