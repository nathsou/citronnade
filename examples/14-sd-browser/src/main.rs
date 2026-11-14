//! Browses the SD card. Up/Down (or touch) select, A opens a directory or
//! shows a text file, B goes up, X appends a line to a log file on the card,
//! START exits.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use citronnade::fs::{self, DirEntry};
use citronnade::prelude::*;

const LOG_FILE: &str = "/citronnade-sd-browser.log";
const ROWS: usize = 26;

fn listing(path: &str) -> Vec<DirEntry> {
    let mut entries = fs::read_dir(path).unwrap_or_default();
    entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then(a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    entries
}

fn join(dir: &str, name: &str) -> String {
    if dir.ends_with('/') { format!("{dir}{name}") } else { format!("{dir}/{name}") }
}

fn parent(dir: &str) -> String {
    match dir.trim_end_matches('/').rfind('/') {
        Some(0) | None => String::from("/"),
        Some(i) => String::from(&dir[..i]),
    }
}

fn self_check() -> Result<()> {
    log!("free space: {} MiB of {} MiB", fs::free_space()? >> 20, fs::total_space()? >> 20);
    let probe = "/citronnade-test-probe.txt";
    fs::write(probe, b"probe contents")?;
    let back = fs::read(probe)?;
    fs::remove_file(probe)?;
    if back == b"probe contents" {
        log!("wrote and read back {} bytes", back.len());
    } else {
        log!("error: read back {:?}", back);
    }
    if fs::exists("/citronnade-test") {
        log!("listing /citronnade-test");
        for e in listing("/citronnade-test") {
            log!("  {}{} ({} bytes)", e.name, if e.is_dir { "/" } else { "" }, e.size);
        }
        if let Ok(text) = fs::read_to_string("/citronnade-test/notes.txt") {
            log!("notes.txt starts with: {}", text.lines().next().unwrap_or(""));
        }
    }
    Ok(())
}

#[citronnade::main]
fn main(sys: System) -> Result<()> {
    let System { apt, gsp, hid, .. } = sys;
    let mut screens = gsp.screens()?;
    let mut input = hid.input()?;
    input.set_repeat(18, 4);
    if let Err(e) = self_check() {
        log!("error in the self check: {e:?}");
    }
    let mut dir = String::from("/");
    let mut entries = listing(&dir);
    let mut selected = 0usize;
    let mut scroll = 0usize;
    let mut viewing: Option<(String, Vec<String>)> = None;
    let mut status = String::from("A: open  B: up  X: append to log  START: exit");

    while apt.main_loop() {
        let i = input.scan();
        let (down, rep) = (i.down(), i.repeated());
        if down.contains(Buttons::START) {
            break;
        }
        if down.contains(Buttons::X) {
            let line = format!("visited {} at {} ms\n", dir, citronnade::time::millis());
            match fs::append(LOG_FILE, line.as_bytes()) {
                Ok(()) => {
                    log!("appended to {LOG_FILE}");
                    status = format!("appended to {LOG_FILE}");
                }
                Err(e) => status = format!("error {:#x}", e.0),
            }
        }
        if viewing.is_some() {
            if down.intersects(Buttons::B | Buttons::A) {
                viewing = None;
            }
        } else {
            if rep.intersects(Buttons::DOWN) && selected + 1 < entries.len() {
                selected += 1;
            }
            if rep.intersects(Buttons::UP) && selected > 0 {
                selected -= 1;
            }
            if let Some((_, y)) = i.touch() {
                let row = y as usize / 9;
                if (1..=ROWS).contains(&row) && scroll + row - 1 < entries.len() {
                    selected = scroll + row - 1;
                }
            }
            if down.contains(Buttons::B) && dir != "/" {
                dir = parent(&dir);
                entries = listing(&dir);
                selected = 0;
            }
            if down.contains(Buttons::A) {
                if let Some(e) = entries.get(selected).cloned() {
                    let path = join(&dir, &e.name);
                    if e.is_dir {
                        dir = path;
                        entries = listing(&dir);
                        selected = 0;
                    } else if e.size <= 64 * 1024 {
                        let text = fs::read_to_string(&path).unwrap_or_default();
                        viewing = Some((path, text.lines().take(24).map(String::from).collect()));
                    } else {
                        status = format!("{} is too large to show", e.name);
                    }
                }
            }
        }
        if selected < scroll {
            scroll = selected;
        }
        if selected >= scroll + ROWS {
            scroll = selected + 1 - ROWS;
        }

        let mut bottom = screens.bottom();
        bottom.clear(Color::rgb(16, 18, 24));
        bottom.fill_rect(0, 0, 320, 9, Color::rgb(40, 60, 110));
        bottom.text(2, 1, &dir, Color::WHITE, 1);
        for (row, e) in entries.iter().enumerate().skip(scroll).take(ROWS) {
            let y = 9 + (row - scroll) as i32 * 9;
            if row == selected {
                bottom.fill_rect(0, y, 320, 9, Color::rgb(60, 80, 140));
            }
            let name = if e.is_dir { format!("{}/", e.name) } else { e.name.clone() };
            bottom.text(4, y + 1, &name, if e.is_dir { Color::YELLOW } else { Color::WHITE }, 1);
            if !e.is_dir {
                bottom.text(250, y + 1, &format!("{:>8}", e.size), Color::GRAY, 1);
            }
        }

        let mut top = screens.top();
        top.clear(Color::rgb(24, 28, 40));
        match &viewing {
            Some((path, lines)) => {
                top.text(8, 6, path, Color::YELLOW, 1);
                for (k, l) in lines.iter().enumerate() {
                    top.text(8, 22 + k as i32 * 9, l, Color::WHITE, 1);
                }
            }
            None => {
                top.text(8, 8, "SD card", Color::WHITE, 3);
                top.text(8, 40, &format!("{} entries in {}", entries.len(), dir), Color::WHITE, 1);
            }
        }
        top.text(8, 226, &status, Color::GRAY, 1);
        screens.present();
    }
    Ok(())
}
