//! The New 3DS's extra controls: ZL, ZR and the C-Stick (read through
//! ir:rst). On an Old 3DS they stay released unless a Circle Pad Pro is
//! attached. START exits.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use citronnade::info;
use citronnade::prelude::*;

#[citronnade::main]
fn main(sys: System) -> Result<()> {
    let System { apt, gsp, hid, .. } = sys;
    let mut screens = gsp.screens()?;
    let mut input = hid.input()?;
    input.enable_new3ds_controls()?;
    let model = info::model().unwrap_or(info::Model::Old3ds);
    log!("new3ds-inputs: model {} (New: {}, APT says New: {})", model.name(), model.is_new(), apt.is_new_3ds());
    let extra = Buttons::ZL | Buttons::ZR | Buttons::CSTICK_UP | Buttons::CSTICK_DOWN | Buttons::CSTICK_LEFT | Buttons::CSTICK_RIGHT;

    while apt.main_loop() {
        let i = input.scan();
        let down = i.down() & extra;
        if !down.is_empty() {
            log!("down: {:?}", down);
        }
        if i.down().contains(Buttons::START) {
            break;
        }
        let held = i.held();
        let stick = i.c_stick();

        let mut top = screens.top();
        top.clear(Color::rgb(24, 26, 34));
        for (b, name, x) in [(Buttons::ZL, "ZL", 40), (Buttons::ZR, "ZR", 300)] {
            let on = held.contains(b);
            top.fill_rect(x, 20, 60, 30, if on { Color::YELLOW } else { Color::DARK_GRAY });
            top.text(x + 22, 31, name, if on { Color::BLACK } else { Color::WHITE }, 1);
        }
        top.circle(200, 140, 50, Color::GRAY);
        top.fill_circle(200 + stick.0 as i32 * 44 / 156, 140 - stick.1 as i32 * 44 / 156, 10, Color::CYAN);
        top.text(140, 205, &format!("C-Stick {:4} {:4}", stick.0, stick.1), Color::WHITE, 1);

        let mut bottom = screens.bottom();
        bottom.clear(Color::rgb(16, 18, 24));
        bottom.text(10, 10, model.name(), Color::WHITE, 1);
        bottom.text(10, 30, if model.is_new() { "Has ZL, ZR and the C-Stick." } else { "Old model: needs a Circle Pad Pro." }, Color::YELLOW, 1);
        bottom.text(10, 220, "START: exit", Color::GRAY, 1);
        screens.present();
    }
    log!("bye");
    Ok(())
}
