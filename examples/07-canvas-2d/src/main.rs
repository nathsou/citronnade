//! Software 2D drawing: sprites (drawn procedurally at startup) bounce
//! around with alpha blending, over a frame-time graph. A adds sprites,
//! B removes them, START exits.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use alloc::vec::Vec;

use citronnade::prelude::*;
use citronnade::math::{atan2, rem_euclid, sqrt};
use citronnade::{time, Image};

/// A citrus slice: rind, pith and eight segments.
fn slice(size: u32, hue: u16) -> Image {
    let mut img = Image::new(size, size);
    let c = (size as f32 - 1.0) / 2.0;
    for y in 0..size {
        for x in 0..size {
            let (dx, dy) = (x as f32 - c, y as f32 - c);
            let r = sqrt(dx * dx + dy * dy) / c;
            let seg = rem_euclid(atan2(dy, dx) / (core::f32::consts::PI / 4.0), 1.0);
            let color = if r > 1.0 {
                Color::TRANSPARENT
            } else if r > 0.88 {
                Color::hsv(hue, 230, 230)
            } else if r > 0.78 {
                Color::rgb(255, 248, 225)
            } else if r < 0.12 || !(0.08..=0.92).contains(&seg) {
                Color::rgb(255, 245, 210)
            } else {
                Color::hsv(hue, 150, 255).with_alpha(220)
            };
            img.set(x, y, color);
        }
    }
    img
}

/// A soft glowing dot.
fn glow(size: u32, color: Color) -> Image {
    let mut img = Image::new(size, size);
    let c = (size as f32 - 1.0) / 2.0;
    for y in 0..size {
        for x in 0..size {
            let (dx, dy) = (x as f32 - c, y as f32 - c);
            let r = sqrt(dx * dx + dy * dy) / c;
            let a = if r >= 1.0 { 0.0 } else { (1.0 - r) * (1.0 - r) };
            img.set(x, y, color.with_alpha((a * 255.0) as u8));
        }
    }
    img
}

struct Sprite {
    x: i32,
    y: i32,
    dx: i32,
    dy: i32,
    kind: usize,
}

struct Rng(u32);

impl Rng {
    fn next(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        self.0
    }
    fn range(&mut self, lo: i32, hi: i32) -> i32 {
        lo + (self.next() % (hi - lo) as u32) as i32
    }
}

#[citronnade::main]
fn main(sys: System) -> Result<()> {
    let System { apt, gsp, hid, .. } = sys;
    let mut screens = gsp.screens()?;
    let mut input = hid.input()?;
    let images = [
        slice(28, 50),
        slice(28, 30),
        slice(28, 100),
        glow(24, Color::rgb(255, 200, 80)),
        glow(24, Color::rgb(120, 200, 255)),
    ];
    let mut rng = Rng(0x1234_5678);
    let spawn = |rng: &mut Rng| Sprite {
        x: rng.range(0, 372),
        y: rng.range(0, 212),
        dx: rng.range(1, 4) * if rng.next() & 1 == 0 { 1 } else { -1 },
        dy: rng.range(1, 4) * if rng.next() & 1 == 0 { 1 } else { -1 },
        kind: (rng.next() % 5) as usize,
    };
    let mut sprites: Vec<Sprite> = (0..24).map(|_| spawn(&mut rng)).collect();
    let mut times = [0u32; 200];
    let mut frame = 0usize;
    let mut total_us: u64 = 0;
    log!("canvas-2d: {} sprites (A: more, B: fewer, START: exit)", sprites.len());

    while apt.main_loop() {
        let start = time::micros();
        let down = input.scan().down();
        if down.contains(Buttons::START) {
            break;
        }
        if down.contains(Buttons::A) {
            for _ in 0..8 {
                sprites.push(spawn(&mut rng));
            }
            log!("sprites: {}", sprites.len());
        }
        if down.contains(Buttons::B) {
            let n = sprites.len().saturating_sub(8);
            sprites.truncate(n);
            log!("sprites: {}", sprites.len());
        }
        for s in &mut sprites {
            s.x += s.dx;
            s.y += s.dy;
            if s.x < 0 || s.x > 372 {
                s.dx = -s.dx;
            }
            if s.y < 0 || s.y > 212 {
                s.dy = -s.dy;
            }
        }

        let mut top = screens.top();
        top.vertical_gradient(0, 0, 400, 240, Color::rgb(20, 30, 60), Color::rgb(20, 70, 140));
        for s in &sprites {
            let img = &images[s.kind];
            top.blit(s.x, s.y, img.width, img.height, &img.rgba);
        }
        // Frame-time graph: one bar per frame, 16.7 ms marked.
        top.fill_rect(0, 200, 200, 40, Color::rgba(0, 0, 0, 150));
        top.fill_rect(0, 200 + 40 - 17, 200, 1, Color::rgba(255, 255, 255, 120));
        for (i, &t) in times.iter().enumerate() {
            let h = (t / 1000).min(40);
            let c = if t > 16_700 { Color::RED } else { Color::GREEN };
            top.fill_rect(i as i32, 240 - h as i32, 1, h, c);
        }

        let elapsed = (time::micros() - start) as u32;
        times.copy_within(1.., 0);
        times[199] = elapsed;
        total_us += elapsed as u64;
        frame += 1;

        let mut bottom = screens.bottom();
        bottom.clear(Color::rgb(16, 18, 24));
        bottom.text(10, 10, &format!("{} sprites", sprites.len()), Color::WHITE, 2);
        bottom.text(10, 40, &format!("drawing: {:.1} ms", elapsed as f32 / 1000.0), Color::WHITE, 2);
        bottom.text(10, 200, "A: more   B: fewer   START: exit", Color::GRAY, 1);
        screens.present();
    }
    log!("average frame (drawing) {} us over {} frames", total_us / frame.max(1) as u64, frame);
    Ok(())
}
