//! The system's software keyboard, in four configurations: A asks for a
//! name (two buttons, a hint), Y for a 4-digit PIN (numeric, hidden), X for
//! a note (several lines, starting from the previous note), B for a word
//! without digits. What was typed shows on the top screen. START exits.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use citronnade::applets::{Keyboard, KeyboardButton, KeyboardKind, ValidInput};
use citronnade::prelude::*;

#[citronnade::main]
fn main(sys: System) -> Result<()> {
    let System { apt, gsp, hid, .. } = sys;
    let mut screens = gsp.screens()?;
    let mut input = hid.input()?;
    log!("software-keyboard: A name, Y PIN, X note, B word");

    let mut name = String::from("(none yet)");
    let mut pin = String::from("-");
    let mut note = String::from("Lemons are yellow.");
    let mut word = String::from("-");
    let mut last = String::new();
    let mut frame = 0u32;
    let mut returned_at = None;
    while apt.main_loop() {
        let down = input.scan().down();
        if down.contains(Buttons::START) {
            break;
        }
        let describe = |what: &str, r: &citronnade::applets::KeyboardResult| -> String {
            match r.button {
                Some(KeyboardButton::Right) => format!("{}: confirmed \"{}\"", what, r.text),
                Some(KeyboardButton::Left) => format!("{}: cancelled", what),
                Some(KeyboardButton::Middle) => format!("{}: middle button", what),
                None => format!("{}: closed (result {})", what, r.result),
            }
        };
        if down.contains(Buttons::A) {
            let r = Keyboard::new(KeyboardKind::Normal, 2, 24)
                .hint("Your name")
                .button(0, "Cancel", false)
                .button(2, "OK", true)
                .validation(ValidInput::NotEmptyNotBlank, 0)
                .show();
            last = describe("name", &r);
            if r.button == Some(KeyboardButton::Right) {
                name = r.text;
            }
            returned_at = Some(frame);
        }
        if down.contains(Buttons::Y) {
            let r = Keyboard::new(KeyboardKind::Numpad, 2, 4)
                .hint("4-digit PIN")
                .password(1)
                .validation(ValidInput::FixedLength, 4)
                .show();
            last = describe("PIN", &r);
            if r.button == Some(KeyboardButton::Right) {
                pin = "*".repeat(r.text.len());
            }
            returned_at = Some(frame);
        }
        if down.contains(Buttons::X) {
            let r = Keyboard::new(KeyboardKind::Normal, 2, 200).multiline(true).initial_text(&note).show();
            last = describe("note", &r);
            if r.button == Some(KeyboardButton::Right) {
                note = r.text;
            }
            returned_at = Some(frame);
        }
        if down.contains(Buttons::B) {
            let r = Keyboard::new(KeyboardKind::Qwerty, 1, 16).hint("No digits").filter(1).show();
            last = describe("word", &r);
            if r.button.is_some() {
                word = r.text;
            }
            returned_at = Some(frame);
        }
        if returned_at.is_some_and(|f| f != frame) {
            log!("{}", last);
            returned_at = None;
        }
        // Drawing after the applet: if this shows, the screens came back.
        if frame == 60 {
            log!("screens back after the keyboard: frame {}", frame);
        }

        let mut top = screens.top();
        top.clear(Color::rgb(20, 24, 40));
        top.text(12, 10, "Software keyboard", Color::WHITE, 2);
        top.text(12, 50, &format!("name: {}", name), Color::rgb(255, 230, 120), 2);
        top.text(12, 80, &format!("PIN:  {}", pin), Color::rgb(200, 220, 240), 2);
        top.text(12, 110, &format!("word: {}", word), Color::rgb(200, 220, 240), 2);
        top.text(12, 140, "note:", Color::rgb(200, 220, 240), 2);
        for (i, l) in note.lines().take(4).enumerate() {
            top.text(80, 140 + i as i32 * 18, l, Color::rgb(200, 220, 240), 1);
        }
        top.text(12, 222, &last, Color::rgb(150, 160, 180), 1);
        let mut b = screens.bottom();
        b.clear(Color::rgb(16, 16, 24));
        for (i, l) in ["A: name", "Y: PIN (numbers, hidden)", "X: note (several lines)", "B: word (no digits)"].iter().enumerate() {
            b.text(12, 20 + i as i32 * 18, l, Color::WHITE, 1);
        }
        b.text(12, 220, "START: exit", Color::WHITE, 1);
        screens.present();
        frame += 1;
    }
    log!("bye: name {}", name);
    Ok(())
}
