//! A program that panics on purpose.
//!
//! The panic handler logs the location and message, runs the exit hooks and
//! returns to the Homebrew Launcher instead of hanging the console. From
//! v0.2, the message is also shown on screen.
#![no_std]
#![no_main]

use citronnade::log;

fn on_exit() {
    log!("exit hook ran after the panic");
}

#[citronnade::main]
fn main() {
    citronnade::at_exit(on_exit);
    let values = [1, 2, 3];
    // black_box keeps the compiler from rejecting the out-of-bounds index.
    let index = core::hint::black_box(7);
    log!("reading values[{index}]...");
    let v = values[index];
    log!("unreachable: {v}");
}
