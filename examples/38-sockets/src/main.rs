//! Talks to the PC: run `python3 tools/net/hostservers.py` there and write
//! the PC's address into `sd:/citronnade/net.txt`. The example connects to
//! the PC's TCP echo server (port 7777), sends a numbered message every few
//! frames and measures the round trip; it also sends UDP datagrams to port
//! 7778 and, with A, broadcasts one to the whole network to discover the PC
//! without knowing its address. Sockets are non-blocking, so the frame loop
//! never waits. START exits.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use citronnade::net::{Ipv4Addr, Network, SocketAddr, TcpStream, UdpSocket};
use citronnade::prelude::*;
use citronnade::time::Instant;

const TCP_PORT: u16 = 7777;
const UDP_PORT: u16 = 7778;

fn pc_address() -> Option<Ipv4Addr> {
    citronnade::fs::read_to_string("/citronnade/net.txt").ok()?.trim().parse().ok()
}

#[citronnade::main]
fn main(sys: System) -> Result<()> {
    let System { apt, gsp, hid, .. } = sys;
    let mut screens = gsp.screens()?;
    let mut input = hid.input()?;
    let mut console: Vec<String> = Vec::new();
    let say = |console: &mut Vec<String>, s: String| {
        log!("{}", s);
        console.push(s);
        if console.len() > 12 {
            console.remove(0);
        }
    };

    let net = Network::new()?;
    let pc = pc_address();
    match pc {
        Some(ip) => say(&mut console, format!("sockets: PC at {} (from net.txt)", ip)),
        None => say(&mut console, String::from("sockets: no sd:/citronnade/net.txt; A still broadcasts")),
    }

    let mut tcp = pc.and_then(|ip| match TcpStream::connect_timeout(SocketAddr::new(ip, TCP_PORT), 2000) {
        Ok(s) => {
            let _ = s.set_nonblocking(true);
            Some(s)
        }
        Err(e) => {
            log!("tcp: cannot connect: {}", e);
            None
        }
    });
    if tcp.is_some() {
        say(&mut console, format!("tcp: connected to port {}", TCP_PORT));
    }
    let udp = UdpSocket::bind(0)?;
    let _ = udp.set_nonblocking(true);

    let (mut sent, mut pending, mut echoes, mut udp_echoes) = (0u32, None::<(String, Instant)>, 0u32, 0u32);
    let mut received = String::new();
    let mut last_rtt = 0u64;
    let mut frame = 0u32;
    while apt.main_loop() {
        let down = input.scan().down();
        if down.contains(Buttons::START) {
            break;
        }
        // TCP: one message in flight at a time.
        if let Some(s) = tcp.as_mut() {
            if pending.is_none() && frame % 20 == 5 {
                sent += 1;
                let msg = format!("message {} from the 3DS\n", sent);
                if s.write_all(msg.as_bytes()).is_ok() {
                    pending = Some((msg, Instant::now()));
                }
            }
            let mut buf = [0u8; 256];
            match s.read(&mut buf) {
                Ok(0) => {
                    say(&mut console, String::from("tcp: the PC closed the connection"));
                    tcp = None;
                }
                Ok(n) => received.push_str(&String::from_utf8_lossy(&buf[..n])),
                Err(e) if e.would_block() => {}
                Err(e) => say(&mut console, format!("tcp: {}", e)),
            }
            if let Some((msg, at)) = &pending {
                if received.len() >= msg.len() {
                    last_rtt = at.elapsed().as_micros() as u64;
                    let ok = received.starts_with(msg.as_str());
                    received.drain(..msg.len());
                    echoes += 1;
                    if echoes == 1 || !ok {
                        say(&mut console, format!("tcp echo {} ({} us)", if ok { "ok" } else { "mismatch" }, last_rtt));
                    }
                    pending = None;
                }
            }
        }
        // UDP: a datagram to the PC now and then, a broadcast on A.
        if let Some(ip) = pc {
            if frame % 40 == 10 {
                let _ = udp.send_to(format!("ping {}", frame).as_bytes(), SocketAddr::new(ip, UDP_PORT));
            }
        }
        if down.contains(Buttons::A) {
            match udp.send_to(b"who is there?", SocketAddr::new(Ipv4Addr::BROADCAST, UDP_PORT)) {
                Ok(_) => say(&mut console, String::from("udp: broadcast sent")),
                Err(e) => say(&mut console, format!("udp: broadcast failed: {}", e)),
            }
        }
        let mut dgram = [0u8; 256];
        while let Ok((n, from)) = udp.recv_from(&mut dgram) {
            udp_echoes += 1;
            let text = String::from_utf8_lossy(&dgram[..n]).into_owned();
            if udp_echoes == 1 {
                say(&mut console, format!("udp echo ok from {}: {}", from, text));
            }
        }

        let mut top = screens.top();
        top.clear(Color::rgb(14, 20, 34));
        top.text(12, 10, "Sockets", Color::WHITE, 2);
        top.text(12, 40, &format!("TCP: {} sent, {} echoed, last round trip {} us", sent, echoes, last_rtt), Color::rgb(200, 220, 240), 1);
        top.text(12, 54, &format!("UDP: {} replies", udp_echoes), Color::rgb(200, 220, 240), 1);
        for (i, l) in console.iter().enumerate() {
            top.text(12, 76 + i as i32 * 12, l, Color::rgb(170, 190, 210), 1);
        }
        let mut b = screens.bottom();
        b.clear(Color::rgb(16, 16, 24));
        b.text(8, 8, "PC side:", Color::WHITE, 1);
        b.text(8, 22, "python3 tools/net/hostservers.py", Color::rgb(200, 210, 230), 1);
        b.text(8, 36, "and its address in sd:/citronnade/net.txt", Color::rgb(200, 210, 230), 1);
        b.text(8, 206, "A: UDP broadcast", Color::WHITE, 1);
        b.text(8, 220, "START: exit", Color::WHITE, 1);
        screens.present();
        frame += 1;
    }
    drop(tcp);
    drop(udp);
    drop(net);
    log!("bye: {} TCP echoes, {} UDP replies", echoes, udp_echoes);
    Ok(())
}
