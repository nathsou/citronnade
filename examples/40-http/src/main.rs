//! HTTP through the system's http:C service (which also does HTTPS and
//! honours the console's proxy settings). The URL comes from
//! `sd:/citronnade/http.txt`, or `http://<PC>:7780/status.json` with the PC
//! in `sd:/citronnade/net.txt` (`tools/net/hostservers.py` serves it), or
//! example.com. The response shows on screen; a small JSON reader picks
//! fields out of it. A POST then sends a few bytes back. START exits.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use citronnade::net::{Http, Network};
use citronnade::prelude::*;

/// The text of `"key": "value"` or `"key": [ ... ]` in a flat JSON object
/// (just enough for this example).
fn json_field<'a>(json: &'a str, key: &str) -> Option<&'a str> {
    let at = json.find(&format!("\"{}\"", key))? + key.len() + 2;
    let rest = json[at..].trim_start().strip_prefix(':')?.trim_start();
    let end = match rest.chars().next()? {
        '[' => rest.find(']')? + 1,
        '"' => rest[1..].find('"')? + 2,
        _ => rest.find([',', '}']).unwrap_or(rest.len()),
    };
    Some(rest[..end].trim())
}

fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    for raw in text.lines() {
        let mut line = String::new();
        for ch in raw.chars() {
            if line.len() >= width {
                lines.push(core::mem::take(&mut line));
            }
            line.push(ch);
        }
        lines.push(line);
    }
    lines
}

#[citronnade::main]
fn main(sys: System) -> Result<()> {
    let System { apt, gsp, hid, .. } = sys;
    let mut screens = gsp.screens()?;
    let mut input = hid.input()?;
    let _net = Network::new()?;
    let http = Http::new()?;

    let url = citronnade::fs::read_to_string("/citronnade/http.txt")
        .ok()
        .map(|s| String::from(s.trim()))
        .or_else(|| {
            citronnade::fs::read_to_string("/citronnade/net.txt").ok().map(|pc| format!("http://{}:7780/status.json", pc.trim()))
        })
        .unwrap_or_else(|| String::from("http://example.com/"));
    log!("http: fetching {}", url);

    let mut lines: Vec<String> = Vec::new();
    match http.get(&url, 16 * 1024) {
        Ok(r) => {
            log!("http: {} {} ({} bytes)", r.status, r.content_type, r.body.len());
            let text = r.text();
            lines.push(format!("{} {}, {} bytes", r.status, r.content_type, r.body.len()));
            if let Some(lemons) = json_field(&text, "lemons") {
                let list: Vec<&str> = lemons.trim_matches(['[', ']']).split(',').map(|s| s.trim().trim_matches('"')).collect();
                log!("lemons: {}", list.join(", "));
            }
            if let Some(t) = json_field(&text, "temperature") {
                log!("temperature: {}", t);
            }
            lines.extend(wrap(&text, 64));
        }
        Err(e) => {
            log!("http: request failed: {:?}", e);
            lines.push(format!("request failed: {:?}", e));
        }
    }
    // A POST: the host's server answers with the size it received.
    if url.contains(":7780/") {
        let post_url = url.replace("status.json", "upload");
        match http.post(&post_url, "text/plain", b"zest and juice", 1024) {
            Ok(r) => log!("post: {} {}", r.status, r.text()),
            Err(e) => log!("post: failed: {:?}", e),
        }
    }

    let mut scroll = 0usize;
    while apt.main_loop() {
        let state = input.scan();
        if state.down().contains(Buttons::START) {
            break;
        }
        if state.down().contains(Buttons::DPAD_DOWN) && scroll + 16 < lines.len() {
            scroll += 1;
        }
        if state.down().contains(Buttons::DPAD_UP) && scroll > 0 {
            scroll -= 1;
        }
        let mut top = screens.top();
        top.clear(Color::rgb(14, 20, 34));
        top.text(8, 6, &url, Color::rgb(255, 220, 120), 1);
        for (i, l) in lines.iter().skip(scroll).take(17).enumerate() {
            top.text(8, 22 + i as i32 * 12, l, Color::rgb(200, 220, 240), 1);
        }
        let mut b = screens.bottom();
        b.clear(Color::rgb(16, 16, 24));
        b.text(8, 8, "HTTP", Color::WHITE, 2);
        b.text(8, 36, "URL from sd:/citronnade/http.txt", Color::rgb(200, 210, 230), 1);
        b.text(8, 206, "Up/Down: scroll", Color::WHITE, 1);
        b.text(8, 220, "START: exit", Color::WHITE, 1);
        screens.present();
    }
    drop(http);
    log!("bye");
    Ok(())
}
