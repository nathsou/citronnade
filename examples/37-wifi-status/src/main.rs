//! The network connection, refreshed twice a second: whether the console
//! is connected, the access point's name and security, the signal strength
//! in bars, and the console's IP address. START exits.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use citronnade::net::Network;
use citronnade::prelude::*;

const SECURITY: [&str; 8] = ["open", "WEP 40-bit", "WEP 104-bit", "WEP 128-bit", "WPA TKIP", "WPA2 TKIP", "WPA AES", "WPA2 AES"];

#[citronnade::main]
fn main(sys: System) -> Result<()> {
    let System { apt, gsp, hid, .. } = sys;
    let mut screens = gsp.screens()?;
    let mut input = hid.input()?;
    let net = match Network::new() {
        Ok(n) => n,
        Err(e) => {
            log!("wifi-status: could not start the network: {}", e);
            return Ok(());
        }
    };
    log!("wifi-status: network up");

    let mut frame = 0u32;
    let mut lines: [alloc::string::String; 5] = Default::default();
    while apt.main_loop() {
        if input.scan().down().contains(Buttons::START) {
            break;
        }
        if frame % 30 == 0 {
            let w = net.wifi();
            let ip = net.local_ip();
            lines[0] = format!("connected   {}", if w.connected { "yes" } else { "no" });
            lines[1] = format!("access point {}", w.ssid.as_deref().unwrap_or("-"));
            lines[2] = format!("security    {}", w.security.map_or("-", |s| SECURITY.get(s as usize).copied().unwrap_or("?")));
            lines[3] = format!("signal      {} of 3 bars", w.bars);
            lines[4] = match ip {
                Ok(ip) => format!("address     {}", ip),
                Err(e) => format!("address     ({})", e),
            };
            if frame == 0 {
                log!("wifi: status {}, connected {}, ssid {:?}, security {:?}, {} bars", w.status, w.connected, w.ssid, w.security, w.bars);
                log!("local address {}", lines[4].trim_start_matches("address").trim());
            }
        }
        let mut top = screens.top();
        top.clear(Color::rgb(14, 20, 34));
        top.text(16, 16, "Wi-Fi", Color::WHITE, 3);
        for (i, l) in lines.iter().enumerate() {
            top.text(16, 70 + i as i32 * 22, l, Color::rgb(200, 220, 240), 2);
        }
        // Signal bars.
        let bars = citronnade::net::wifi_bars();
        for b in 0..3 {
            let h = 12 + b * 10;
            let c = if (b as u8) < bars { Color::rgb(120, 230, 140) } else { Color::rgb(60, 60, 70) };
            top.fill_rect(330 + b * 16, 50 - h, 12, h as u32, c);
        }
        let mut bottom = screens.bottom();
        bottom.clear(Color::rgb(16, 16, 24));
        bottom.text(8, 8, "ac:u and soc:U", Color::WHITE, 2);
        bottom.text(8, 220, "START: exit", Color::WHITE, 1);
        screens.present();
        frame += 1;
    }
    drop(net);
    log!("bye: network stopped");
    Ok(())
}
