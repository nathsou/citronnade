//! A text console on the bottom screen. `println!` writes to it and `log!`
//! lines appear in it too. X panics: the panic screen shows the message
//! until START is pressed.
#![no_std]
#![no_main]

use citronnade::prelude::*;

#[citronnade::main]
fn main(sys: System) -> Result<()> {
    let System { apt, gsp, hid, .. } = sys;
    let mut screens = gsp.screens()?;
    let mut input = hid.input()?;
    let console = Console::bottom();
    println!("Hello from the console!");
    console.set_colors(Color::YELLOW, Color::rgb(16, 18, 24));
    println!("Colours change mid-text.");
    console.set_colors(Color::rgb(230, 230, 230), Color::rgb(16, 18, 24));
    for i in 0..40 {
        println!("line {i}: the console scrolls when it fills up");
    }
    log!("log lines show up here too");

    let mut frame = 0u32;
    while apt.main_loop() {
        let down = input.scan().down();
        if down.contains(Buttons::START) {
            break;
        }
        if down.contains(Buttons::X) {
            panic!("X was pressed (this is the panic screen)");
        }
        if frame % 60 == 0 {
            println!("{} seconds", frame / 60);
        }
        let mut top = screens.top();
        top.clear(Color::rgb(30, 40, 70));
        top.text(12, 12, "Console", Color::WHITE, 3);
        top.text(12, 50, "println! writes to the bottom screen.", Color::WHITE, 1);
        top.text(12, 64, "X: panic   START: exit", Color::YELLOW, 1);
        screens.present();
        frame += 1;
    }
    Ok(())
}
