//! Four worker threads count primes in parts of a range, two on each core
//! when the system core is available. They report progress through atomics,
//! store results under a mutex, and the last one signals an event. The main
//! thread keeps drawing progress bars meanwhile. START exits.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

use citronnade::prelude::*;
use citronnade::sync::{Event, Mutex};
use citronnade::thread::{Builder, CORE_APP, CORE_SYS};
use citronnade::time::Instant;

const LIMIT: u32 = 200_000;
const WORKERS: usize = 4;

struct Shared {
    progress: [AtomicU32; WORKERS],
    results: Mutex<Vec<(usize, i32, u32, u64)>>,
    finished: AtomicUsize,
    done: Event,
}

fn is_prime(n: u32) -> bool {
    if n < 2 {
        return false;
    }
    if n % 2 == 0 {
        return n == 2;
    }
    let mut d = 3;
    while d * d <= n {
        if n % d == 0 {
            return false;
        }
        d += 2;
    }
    true
}

#[citronnade::main]
fn main(sys: System) -> Result<()> {
    let System { apt, gsp, hid, .. } = sys;
    let mut screens = gsp.screens()?;
    let mut input = hid.input()?;
    let sys_core = apt.set_app_cpu_time_limit(30).is_ok();
    log!("threads: system core time limit {}", if sys_core { "set to 30%" } else { "unavailable" });

    let shared = Arc::new(Shared {
        progress: [const { AtomicU32::new(0) }; WORKERS],
        results: Mutex::new(Vec::new()),
        finished: AtomicUsize::new(0),
        done: Event::sticky(),
    });
    let start = Instant::now();
    let mut handles = Vec::new();
    for w in 0..WORKERS {
        let shared = shared.clone();
        let core = if sys_core && w % 2 == 1 { CORE_SYS } else { CORE_APP };
        let work = move || {
            let t0 = Instant::now();
            let (lo, hi) = (w as u32 * LIMIT / WORKERS as u32, (w as u32 + 1) * LIMIT / WORKERS as u32);
            let mut count = 0;
            for n in lo..hi {
                if is_prime(n) {
                    count += 1;
                }
                if n % 1000 == 0 {
                    shared.progress[w].store((n - lo) * 100 / (hi - lo), Ordering::Relaxed);
                }
            }
            shared.progress[w].store(100, Ordering::Relaxed);
            shared.results.lock().push((w, core, count, t0.elapsed().as_micros() as u64));
            if shared.finished.fetch_add(1, Ordering::AcqRel) + 1 == WORKERS {
                shared.done.signal();
            }
        };
        // Below the main thread's priority, so drawing stays smooth.
        let builder = Builder { stack_size: 32 * 1024, priority: 0x31, core };
        let handle = match builder.spawn(work.clone()) {
            Ok(h) => h,
            Err(e) => {
                log!("worker {w}: core {core} refused ({e:?}), using the default core");
                Builder { core: -2, ..builder }.spawn(work)?
            }
        };
        handles.push(handle);
    }

    let mut reported = false;
    while apt.main_loop() {
        if input.scan().down().contains(Buttons::START) {
            break;
        }
        if !reported && shared.done.try_wait() {
            reported = true;
            let mut results = shared.results.lock().clone();
            results.sort();
            for (w, core, count, us) in &results {
                log!("worker {w} on core {core}: {count} primes in {us} us");
            }
            let total: u32 = results.iter().map(|r| r.2).sum();
            log!("total primes below {LIMIT}: {total} (in {} ms)", start.elapsed().as_millis());
        }
        let mut top = screens.top();
        top.clear(Color::rgb(24, 26, 40));
        top.text(10, 10, "Counting primes on worker threads", Color::WHITE, 2);
        for w in 0..WORKERS {
            let p = shared.progress[w].load(Ordering::Relaxed);
            let y = 50 + w as i32 * 40;
            top.text(10, y + 8, &format!("worker {w}"), Color::WHITE, 1);
            top.fill_rect(80, y, 300, 22, Color::DARK_GRAY);
            top.fill_rect(80, y, p * 3, 22, if p == 100 { Color::GREEN } else { Color::CYAN });
        }
        let mut bottom = screens.bottom();
        bottom.clear(Color::rgb(16, 18, 24));
        bottom.text(10, 10, if reported { "All workers finished." } else { "Working..." }, Color::WHITE, 2);
        bottom.text(10, 220, "START: exit", Color::GRAY, 1);
        screens.present();
    }
    shared.done.wait();
    for h in handles {
        h.join();
    }
    log!("bye");
    Ok(())
}
