//! The system's error display: A shows a message of our own (wrapped to
//! fit), B the system's text for a network error code. The application
//! waits while it is up and draws again afterwards. START exits.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use citronnade::applets::ErrorDisplay;
use citronnade::prelude::*;

#[citronnade::main]
fn main(sys: System) -> Result<()> {
    let System { apt, gsp, hid, .. } = sys;
    let mut screens = gsp.screens()?;
    let mut input = hid.input()?;
    log!("error-applet: A text message, B error code");
    let mut shown = 0;
    let mut last = String::from("nothing shown yet");
    while apt.main_loop() {
        let down = input.scan().down();
        if down.contains(Buttons::START) {
            break;
        }
        if down.contains(Buttons::A) {
            let r = ErrorDisplay::text(
                "The lemonade stand has run out of lemons. Squeeze some more and try again. \
                 (This message comes from the error-applet example.)",
            )
            .show();
            shown += 1;
            last = format!("text error closed (return code {})", r);
            log!("{}", last);
        }
        if down.contains(Buttons::B) {
            // A network error number, shown with the system's explanation.
            let r = ErrorDisplay::code(2003).show();
            shown += 1;
            last = format!("code error closed (return code {})", r);
            log!("{}", last);
        }
        let mut top = screens.top();
        top.clear(Color::rgb(40, 20, 24));
        top.text(12, 10, "Error display", Color::WHITE, 2);
        top.text(12, 60, &format!("shown {} times", shown), Color::rgb(255, 210, 200), 2);
        top.text(12, 100, &last, Color::rgb(220, 200, 200), 1);
        let mut b = screens.bottom();
        b.clear(Color::rgb(16, 16, 24));
        b.text(12, 20, "A: a message of our own", Color::WHITE, 1);
        b.text(12, 38, "B: a network error code", Color::WHITE, 1);
        b.text(12, 220, "START: exit", Color::WHITE, 1);
        screens.present();
    }
    log!("bye: {} errors shown", shown);
    Ok(())
}
