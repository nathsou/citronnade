//! Shows every button and the Circle Pad live, and logs presses and
//! releases. START exits.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use citronnade::prelude::*;

const LAYOUT: [(Buttons, &str, i32, i32); 14] = [
    (Buttons::L, "L", 20, 20),
    (Buttons::ZL, "ZL", 70, 20),
    (Buttons::ZR, "ZR", 290, 20),
    (Buttons::R, "R", 340, 20),
    (Buttons::DPAD_UP, "UP", 70, 80),
    (Buttons::DPAD_LEFT, "LEFT", 30, 115),
    (Buttons::DPAD_RIGHT, "RIGHT", 110, 115),
    (Buttons::DPAD_DOWN, "DOWN", 70, 150),
    (Buttons::X, "X", 310, 80),
    (Buttons::Y, "Y", 270, 115),
    (Buttons::A, "A", 350, 115),
    (Buttons::B, "B", 310, 150),
    (Buttons::SELECT, "SELECT", 150, 205),
    (Buttons::START, "START", 210, 205),
];

#[citronnade::main]
fn main(sys: System) -> Result<()> {
    let System { apt, gsp, hid, .. } = sys;
    let mut screens = gsp.screens()?;
    let mut input = hid.input()?;
    let mut last_circle = (0i16, 0i16);
    log!("buttons: press anything, START exits");

    while apt.main_loop() {
        let i = input.scan();
        let (down, up, held) = (i.down(), i.up(), i.held());
        let circle = i.circle_pad();
        if !down.is_empty() {
            log!("down: {:?}", down);
        }
        if !up.is_empty() {
            log!("up: {:?}", up);
        }
        if (circle.0 - last_circle.0).abs() > 20 || (circle.1 - last_circle.1).abs() > 20 {
            log!("circle pad: ({}, {})", circle.0, circle.1);
            last_circle = circle;
        }
        if down.contains(Buttons::START) {
            break;
        }

        let mut top = screens.top();
        top.clear(Color::rgb(24, 26, 34));
        for (button, name, x, y) in LAYOUT {
            let on = held.intersects(button);
            let (w, h) = (if name.len() > 2 { 52 } else { 34 }, 26);
            top.fill_rect(x, y, w, h, if on { Color::YELLOW } else { Color::DARK_GRAY });
            top.rect(x, y, w, h, Color::GRAY);
            let tw = Canvas::text_size(name, 1).0 as i32;
            top.text(x + (w as i32 - tw) / 2, y + 9, name, if on { Color::BLACK } else { Color::WHITE }, 1);
        }
        // The Circle Pad.
        let (cx, cy) = (200, 125);
        top.circle(cx, cy, 40, Color::GRAY);
        let px = cx + circle.0 as i32 * 36 / 156;
        let py = cy - circle.1 as i32 * 36 / 156;
        top.fill_circle(px, py, 10, if held.intersects(Buttons::CPAD_UP | Buttons::CPAD_DOWN | Buttons::CPAD_LEFT | Buttons::CPAD_RIGHT) { Color::CYAN } else { Color::WHITE });

        let mut bottom = screens.bottom();
        bottom.clear(Color::rgb(16, 18, 24));
        bottom.text(10, 10, "Held:", Color::WHITE, 2);
        let mut y = 34;
        for name in held.names() {
            bottom.text(16, y, name, Color::YELLOW, 2);
            y += 18;
        }
        bottom.text(10, 200, &format!("Circle Pad: {:4} {:4}", circle.0, circle.1), Color::WHITE, 1);
        bottom.text(10, 220, "START: exit", Color::GRAY, 1);
        screens.present();
    }
    log!("bye");
    Ok(())
}
