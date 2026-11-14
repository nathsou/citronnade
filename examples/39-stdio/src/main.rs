//! Prints to the PC. Sent with `cargo citronnade run --server`, the output
//! goes back to the terminal that sent the program (3dslink passes the PC's
//! address in the arguments); otherwise to the address in
//! `sd:/citronnade/net.txt`, port 7779 (`tools/net/hostservers.py`).
//!
//! It then runs a few self-checks and prints a summary, the pattern
//! `cargo citronnade test` builds on: a program's checks print one line
//! each, and the PC reads them.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use citronnade::net::{stdio, Ipv4Addr, Network, SocketAddr};
use citronnade::prelude::*;

#[citronnade::main]
fn main(sys: System) -> Result<()> {
    let System { apt, .. } = sys;
    let net = Network::new()?;
    let target = match stdio::connect_link_host(&net) {
        Ok(Some(addr)) => Some(addr),
        _ => {
            let pc: Option<Ipv4Addr> = citronnade::fs::read_to_string("/citronnade/net.txt").ok().and_then(|t| t.trim().parse().ok());
            pc.map(|ip| SocketAddr::new(ip, 7779)).filter(|addr| stdio::connect(&net, *addr).is_ok())
        }
    };
    match target {
        Some(addr) => log!("stdio: output goes to {}", addr),
        None => log!("stdio: no PC to talk to; printing to the debugger only"),
    }

    println!("Hello from the 3DS!");
    let squares: Vec<String> = (1..=5).map(|i| format!("{}", i * i)).collect();
    println!("squares: {}", squares.join(" "));

    // Self-checks: each prints PASS or FAILED with its name.
    let checks: [(&str, fn() -> bool); 6] = [
        ("arithmetic", || 6 * 7 == 42),
        ("float maths", || (citronnade::math::sqrt(2.0) - 1.414_213_5).abs() < 1e-5),
        ("heap", || {
            let v: Vec<u32> = (0..1000).collect();
            v.iter().sum::<u32>() == 499_500
        }),
        ("formatting", || format!("{:>5}|{:<3}|{:x}", 42, "a", 255) == "   42|a  |ff"),
        ("time moves", || {
            let t = citronnade::time::ticks();
            citronnade::svc::sleep_thread(1_000_000);
            citronnade::time::ticks() > t
        }),
        ("address parsing", || "10.0.0.7:80".parse::<SocketAddr>() == Ok(SocketAddr::new(Ipv4Addr::new(10, 0, 0, 7), 80))),
    ];
    let mut passed = 0;
    for (name, check) in checks {
        let ok = check();
        passed += ok as usize;
        println!("{} {}", if ok { "PASS  " } else { "FAILED" }, name);
    }
    println!("self-check: {} of {} passed", passed, checks.len());

    for _ in 0..10 {
        if !apt.main_loop() {
            break;
        }
        citronnade::svc::sleep_thread(16_000_000);
    }
    println!("goodbye");
    drop(net);
    Ok(())
}
