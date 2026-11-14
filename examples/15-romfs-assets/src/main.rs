//! Loads assets from the RomFS that `cargo citronnade build` packs into the
//! 3DSX (the `romfs/` directory next to this file): a text file, a CSV file
//! and two PNG images, decoded on the console. START exits.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use citronnade::formats::png;
use citronnade::prelude::*;
use citronnade::romfs::{DirEntry, RomFs};

fn list(romfs: &RomFs, dir: &str, depth: usize, out: &mut Vec<String>) {
    for e in romfs.read_dir(dir).unwrap_or_default() {
        match e {
            DirEntry::Dir(name) => {
                out.push(format!("{}{name}/", "  ".repeat(depth)));
                list(romfs, &format!("{dir}/{name}"), depth + 1, out);
            }
            DirEntry::File(name, size) => out.push(format!("{}{name} ({size} bytes)", "  ".repeat(depth))),
        }
    }
}

#[citronnade::main]
fn main(sys: System) -> Result<()> {
    let System { apt, gsp, hid, .. } = sys;
    let mut screens = gsp.screens()?;
    let mut input = hid.input()?;
    let mut romfs = match RomFs::mount() {
        Ok(r) => r,
        Err(e) => panic!("could not mount the RomFS: {e}"),
    };
    log!("mounted RomFS");
    let mut tree = Vec::new();
    list(&romfs, "", 0, &mut tree);
    for l in &tree {
        log!("{l}");
    }
    let hello = romfs.read_to_string("/hello.txt").unwrap_or_default();
    log!("{}", hello.lines().next().unwrap_or(""));
    let csv = romfs.read_to_string("/data/squares.csv").unwrap_or_default();
    let sum: u64 = csv.lines().skip(1).filter_map(|l| l.split(',').nth(1)?.parse::<u64>().ok()).sum();
    log!("sum of squares: {sum}");
    let mut images = Vec::new();
    for name in ["slice.png", "badge.png"] {
        let data = romfs.read(&format!("/images/{name}")).unwrap_or_default();
        match png::decode(&data) {
            Ok(img) => {
                log!("{name}: {}x{}", img.width, img.height);
                images.push(img);
            }
            Err(e) => log!("{name}: {e}"),
        }
    }

    let mut frame = 0i32;
    while apt.main_loop() {
        if input.scan().down().contains(Buttons::START) {
            break;
        }
        let mut top = screens.top();
        top.vertical_gradient(0, 0, 400, 240, Color::rgb(30, 40, 70), Color::rgb(70, 50, 90));
        top.text(10, 10, "Assets from the RomFS", Color::WHITE, 2);
        for (k, line) in hello.lines().enumerate() {
            top.text(10, 40 + k as i32 * 10, line, Color::rgb(220, 230, 255), 1);
        }
        if let Some(img) = images.first() {
            let bob = ((frame % 120) - 60).abs() / 4;
            top.blit(60, 90 + bob, img.width, img.height, &img.rgba);
        }
        if let Some(img) = images.get(1) {
            for k in 0..5 {
                top.blit(170 + k * 40, 120, img.width, img.height, &img.rgba);
            }
        }
        top.text(10, 220, &format!("sum of squares from the CSV: {sum}"), Color::YELLOW, 1);

        let mut bottom = screens.bottom();
        bottom.clear(Color::rgb(16, 18, 24));
        bottom.text(8, 6, "romfs:/", Color::YELLOW, 1);
        for (k, l) in tree.iter().enumerate() {
            bottom.text(8, 18 + k as i32 * 10, l, Color::WHITE, 1);
        }
        bottom.text(8, 226, "START: exit", Color::GRAY, 1);
        screens.present();
        frame += 1;
    }
    log!("bye");
    Ok(())
}
