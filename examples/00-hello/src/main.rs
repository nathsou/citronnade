//! The smallest Citronnade program: it logs a few lines and exits back to
//! the Homebrew Launcher.
//!
//! The output goes to the debugger (`svcOutputDebugString`): Luma3DS's GDB
//! stub on hardware, or the emulator's log.
#![no_std]
#![no_main]

use citronnade::{env, heap, log, time};

#[citronnade::main]
fn main() {
    log!("Hello from Citronnade!");
    log!("started by a homebrew loader: {}", env::is_homebrew());
    for (i, arg) in env::args().iter().enumerate() {
        log!("argument {i}: {arg}");
    }
    let app = heap::app_stats();
    let linear = heap::linear_stats();
    log!("application heap: {} KiB at {:#010x}", app.size / 1024, app.base);
    log!("linear heap: {} KiB at {:#010x}", linear.size / 1024, linear.base);
    log!("uptime: {} ms", time::millis());
    log!("goodbye");
}
