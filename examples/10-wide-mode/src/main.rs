//! Wide mode: the top screen as 800x240, with pixels half as wide (all
//! models but the original 2DS). Fine stripes and two columns of text show
//! the doubled horizontal resolution. START exits.
#![no_std]
#![no_main]

use citronnade::info;
use citronnade::prelude::*;

#[citronnade::main]
fn main(sys: System) -> Result<()> {
    let System { apt, gsp, hid, .. } = sys;
    let mut screens = gsp.screens()?;
    let mut input = hid.input()?;
    let model = info::model().unwrap_or(info::Model::Old3ds);
    let wide = model.has_wide_mode();
    if wide {
        screens.set_top_mode(TopMode::Wide);
    }
    log!("wide-mode: {} ({})", model.name(), if wide { "800x240" } else { "not supported, 400x240" });

    while apt.main_loop() {
        if input.scan().down().contains(Buttons::START) {
            break;
        }
        let mut top = screens.top();
        let w = top.width() as i32;
        top.clear(Color::rgb(20, 24, 34));
        // One-pixel stripes: distinct columns only in wide mode.
        for x in 0..w {
            let c = if x % 2 == 0 { Color::rgb(240, 200, 60) } else { Color::rgb(40, 60, 160) };
            top.fill_rect(x, 0, 1, 40, c);
        }
        top.text(8, 50, "Wide mode: 800 columns.", Color::WHITE, 2);
        top.text(8, 80, "Each glyph here is as wide as a column of the normal mode.", Color::WHITE, 1);
        top.text(w / 2 + 8, 110, "This text starts in the right half.", Color::CYAN, 1);
        for i in 0..8 {
            top.fill_circle(60 + i * (w - 120) / 7, 180, 30, Color::hsv((i * 45) as u16, 180, 230));
        }

        let mut bottom = screens.bottom();
        bottom.clear(Color::rgb(16, 18, 24));
        bottom.text(10, 10, model.name(), Color::WHITE, 1);
        bottom.text(10, 30, if wide { "Wide mode is on." } else { "Wide mode is not available." }, Color::YELLOW, 1);
        bottom.text(10, 220, "START: exit", Color::GRAY, 1);
        screens.present();
    }
    log!("bye");
    Ok(())
}
