//! The smallest GPU program: two render targets in VRAM, cleared by the
//! GPU's memory fill engine and copied to the screens by display transfers
//! (which also untile and convert the pixels). The bottom screen then gets
//! CPU-drawn text on top. START exits.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use citronnade::gpu::{self, ColorFormat, Gpu, RenderTarget};
use citronnade::prelude::*;
use citronnade::time;

#[citronnade::main]
fn main(sys: System) -> Result<()> {
    let System { apt, gsp, hid, .. } = sys;
    let mut screens = gsp.screens()?;
    let mut input = hid.input()?;
    let mut gpu = Gpu::new()?;
    let top = RenderTarget::new(400, 240, ColorFormat::Rgba8, None)?;
    let bottom = RenderTarget::new(320, 240, ColorFormat::Rgb565, None)?;
    log!("gpu-clear: VRAM free {} KiB after two targets", gpu::vram::free_bytes() / 1024);

    let mut frame: u32 = 0;
    let (mut clear_us, mut transfer_us) = (0u64, 0u64);
    while apt.main_loop() {
        if input.scan().down().contains(Buttons::START) {
            break;
        }
        let a = Color::hsv((frame * 3 % 360) as u16, 200, 230);
        let b = Color::hsv(((frame * 3 + 180) % 360) as u16, 160, 120);

        let t0 = time::micros();
        gpu.clear(&top, gpu::rgba(a.r, a.g, a.b, 255), 0)?;
        gpu.clear(&bottom, gpu::rgba(b.r, b.g, b.b, 255), 0)?;
        let t1 = time::micros();
        screens.show_target(&mut gpu, &top, Screen::Top)?;
        screens.show_target(&mut gpu, &bottom, Screen::Bottom)?;
        let t2 = time::micros();
        clear_us += t1 - t0;
        transfer_us += t2 - t1;

        if frame == 30 {
            // The CPU sees what the GPU wrote (the screens use BGR8).
            let got = screens.top().get(200, 120);
            let ok = got.r.abs_diff(a.r) <= 1 && got.g.abs_diff(a.g) <= 1 && got.b.abs_diff(a.b) <= 1;
            log!("readback top {} ({:?} vs {:?})", if ok { "ok" } else { "mismatch" }, (got.r, got.g, got.b), (a.r, a.g, a.b));
            let got = screens.bottom().get(10, 10);
            // RGB565 keeps 5, 6 and 5 bits.
            let ok = got.r.abs_diff(b.r) <= 8 && got.g.abs_diff(b.g) <= 4 && got.b.abs_diff(b.b) <= 8;
            log!("readback bottom {} ({:?} vs {:?})", if ok { "ok" } else { "mismatch" }, (got.r, got.g, got.b), (b.r, b.g, b.b));
        }

        let mut c = screens.bottom();
        c.text(10, 10, "GPU memory fill + display transfer", Color::WHITE, 1);
        c.text(10, 24, &format!("frame {frame}"), Color::WHITE, 2);
        c.text(10, 220, "START: exit", Color::WHITE, 1);
        screens.present();
        frame += 1;
    }
    let n = frame.max(1) as u64;
    log!("bye: clear {} us, transfer {} us per frame", clear_us / n, transfer_us / n);
    Ok(())
}
