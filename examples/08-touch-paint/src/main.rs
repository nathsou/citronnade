//! Paint on the bottom screen with the stylus. Touch a swatch at the bottom
//! to change colour, L/R change the brush size, SELECT clears, START exits.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use citronnade::prelude::*;
use citronnade::Surface;

const PALETTE: [Color; 8] = [
    Color::rgb(20, 20, 24),
    Color::rgb(240, 240, 240),
    Color::rgb(230, 50, 50),
    Color::rgb(250, 160, 40),
    Color::rgb(250, 220, 50),
    Color::rgb(60, 190, 90),
    Color::rgb(50, 120, 230),
    Color::rgb(160, 80, 210),
];
const SWATCH_Y: i32 = 216;
const SWATCH_W: i32 = 40;

#[citronnade::main]
fn main(sys: System) -> Result<()> {
    let System { apt, gsp, hid, .. } = sys;
    let mut screens = gsp.screens()?;
    let mut input = hid.input()?;
    let mut paper = Surface::new(320, screens.format(Screen::Bottom));
    paper.canvas().clear(Color::rgb(250, 248, 240));
    let mut colour = 6usize;
    let mut brush = 4i32;
    let mut last: Option<(i32, i32)> = None;
    let mut stroke_start = (0, 0);
    let mut points = 0;
    let mut strokes = 0;
    log!("touch-paint: draw with the stylus (L/R: brush, SELECT: clear, START: exit)");

    while apt.main_loop() {
        let i = input.scan();
        let down = i.down();
        if down.contains(Buttons::START) {
            break;
        }
        if down.contains(Buttons::SELECT) {
            paper.canvas().clear(Color::rgb(250, 248, 240));
            log!("cleared");
        }
        if down.contains(Buttons::R) {
            brush = (brush + 2).min(20);
            log!("brush: {brush}");
        }
        if down.contains(Buttons::L) {
            brush = (brush - 2).max(1);
            log!("brush: {brush}");
        }
        match i.touch() {
            Some((x, y)) if (y as i32) >= SWATCH_Y => {
                if last.is_none() {
                    let pick = (x as i32 / SWATCH_W).clamp(0, 7) as usize;
                    if pick != colour {
                        colour = pick;
                        log!("colour: {:?}", PALETTE[colour]);
                    }
                }
                last = Some((-1, -1));
            }
            Some((x, y)) => {
                let (x, y) = (x as i32, y as i32);
                let mut canvas = paper.canvas();
                match last {
                    Some((lx, ly)) if lx >= 0 => canvas.thick_line(lx, ly, x, y, brush, PALETTE[colour]),
                    _ => {
                        canvas.fill_circle(x, y, brush, PALETTE[colour]);
                        stroke_start = (x, y);
                        points = 0;
                    }
                }
                points += 1;
                last = Some((x, y));
            }
            None => {
                if let Some((lx, ly)) = last {
                    if lx >= 0 {
                        strokes += 1;
                        log!("stroke {strokes}: from {:?} to ({lx}, {ly}), {points} samples", stroke_start);
                    }
                }
                last = None;
            }
        }

        let mut bottom = screens.bottom();
        bottom.draw_surface(&paper);
        for (k, c) in PALETTE.iter().enumerate() {
            let x = k as i32 * SWATCH_W;
            bottom.fill_rect(x, SWATCH_Y, SWATCH_W as u32, 24, *c);
            if k == colour {
                bottom.rect(x + 1, SWATCH_Y + 1, SWATCH_W as u32 - 2, 22, Color::rgb(255, 0, 255));
                bottom.rect(x + 2, SWATCH_Y + 2, SWATCH_W as u32 - 4, 20, Color::WHITE);
            }
        }

        let mut top = screens.top();
        top.clear(Color::rgb(40, 44, 60));
        top.text(12, 12, "Touch Paint", Color::WHITE, 3);
        top.text(12, 56, "Draw on the bottom screen.", Color::WHITE, 1);
        top.text(12, 70, "Swatches: colour   L/R: brush size", Color::WHITE, 1);
        top.text(12, 84, "SELECT: clear      START: exit", Color::WHITE, 1);
        top.fill_circle(330, 120, brush, PALETTE[colour]);
        top.circle(330, 120, 24, Color::GRAY);
        top.text(12, 200, &format!("{strokes} strokes, brush {brush}"), Color::YELLOW, 2);
        screens.present();
    }
    log!("bye");
    Ok(())
}
