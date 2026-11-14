//! Fills both screens with cycling colours, with gradients that show the
//! precision of each framebuffer format. A switches formats, START exits.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use citronnade::prelude::*;
use citronnade::time;

#[citronnade::main]
fn main(sys: System) -> Result<()> {
    let System { apt, gsp, hid, .. } = sys;
    let mut screens = gsp.screens()?;
    let mut input = hid.input()?;
    let mut format = 1; // Format::ALL[1] is BGR8, the default
    let mut frame: u32 = 0;
    log!("colors: A changes the pixel format, START exits");

    let (mut total, mut worst) = (0u64, 0u64);
    while apt.main_loop() {
        let start = time::micros();
        let down = input.scan().down();
        if down.contains(Buttons::START) {
            break;
        }
        if down.contains(Buttons::A) {
            format = (format + 1) % Format::ALL.len();
            let f = Format::ALL[format];
            screens.set_format(Screen::Top, f);
            screens.set_format(Screen::Bottom, f);
            log!("format: {:?}", f);
        }
        let hue = (frame * 2 % 360) as u16;
        let name = format!("{:?} ({} bytes/pixel)", Format::ALL[format], Format::ALL[format].bytes_per_pixel());

        let mut top = screens.top();
        top.clear(Color::hsv(hue, 170, 210));
        for x in 0..400i32 {
            let v = (x * 255 / 399) as u8;
            top.fill_rect(x, 168, 1, 24, Color::rgb(v, 0, 0));
            top.fill_rect(x, 192, 1, 24, Color::rgb(0, v, 0));
            top.fill_rect(x, 216, 1, 24, Color::rgb(0, 0, v));
        }
        top.fill_rect(0, 0, 400, 30, Color::rgba(0, 0, 0, 140));
        top.text(8, 8, &name, Color::WHITE, 2);

        let mut bottom = screens.bottom();
        bottom.clear(Color::hsv(hue + 180, 170, 210));
        for (i, c) in [Color::RED, Color::GREEN, Color::BLUE, Color::YELLOW, Color::CYAN, Color::MAGENTA].iter().enumerate() {
            bottom.fill_rect(10 + i as i32 * 50, 60, 44, 44, *c);
        }
        bottom.text(10, 130, &format!("frame {frame}"), Color::WHITE, 2);
        bottom.text(10, 220, "A: next format   START: exit", Color::WHITE, 1);

        let elapsed = time::micros() - start;
        worst = worst.max(elapsed);
        total += elapsed;
        screens.present();
        frame += 1;
        if frame == 60 {
            log!("presented 60 frames");
        }
    }
    log!("bye: drawing took {} us per frame on average, {} us at worst", total / frame.max(1) as u64, worst);
    Ok(())
}
