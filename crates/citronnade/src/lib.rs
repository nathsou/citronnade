//! Nintendo 3DS homebrew in Rust, without devkitPro.
//!
//! ```ignore
//! #![no_std]
//! #![no_main]
//!
//! use citronnade::prelude::*;
//!
//! #[citronnade::main]
//! fn main(sys: System) -> Result<()> {
//!     let System { apt, gsp, hid, .. } = sys;
//!     let mut screens = gsp.screens()?;
//!     let mut input = hid.input()?;
//!     while apt.main_loop() {
//!         if input.scan().down().contains(Buttons::START) {
//!             break;
//!         }
//!         screens.top().clear(Color::hex(0x2040A0));
//!         screens.present();
//!     }
//!     Ok(())
//! }
//! ```
#![no_std]

extern crate alloc;

pub mod applets;
pub mod audio;
pub mod camera;
pub mod console;
pub mod font;
pub mod fs;
pub mod gfx;
pub mod gpu;
pub mod info;
pub mod input;
pub mod math;
pub mod net;
pub mod mic;
mod panic_screen;
pub mod romfs;
pub mod system;

pub use citronnade_macros::main;
pub use citronnade_rt::{at_exit, exit, log, set_panic_hook, PanicHook};
pub use console::Console;
pub use gfx::{slider_3d, Canvas, Color, Format, Image, Screen, Screens, Surface, TopMode};
pub use input::{Buttons, Input};
pub use system::{Apt, Flow, Gsp, Hid, Hook, System};

/// Raw system calls.
pub use citronnade_rt::svc;
/// The heaps: usage statistics and linear (GPU/DSP-visible) memory.
pub use citronnade_rt::heap;
/// Threads.
pub use citronnade_rt::thread;
/// Locks and events.
pub use citronnade_rt::sync;
/// Address space helpers.
pub use citronnade_rt::memory;
/// The raw service layer (IPC and each system service).
pub use citronnade_services as services;
/// File formats: PNG, zlib, RomFS, SMDH, 3DSX.
pub use citronnade_formats as formats;

pub use citronnade_rt::svc::ResultCode;

/// The result of a system call or service request.
pub type Result<T> = core::result::Result<T, ResultCode>;

/// What the loader passed: arguments, homebrew detection, service handles.
pub mod env {
    pub use citronnade_rt::args::{args, link_host};
    pub use citronnade_rt::env::*;
}

/// The names most programs need.
pub mod prelude {
    pub use crate::{log, print, println};
    pub use crate::{Apt, Buttons, Canvas, Color, Console, Flow, Format, Input, Result, Screen, Screens, System, TopMode};
}

/// Time since boot.
pub mod time {
    use citronnade_rt::svc;

    pub use svc::TICKS_PER_SECOND;

    /// System ticks since boot.
    #[inline]
    pub fn ticks() -> u64 {
        svc::get_system_tick()
    }

    /// Microseconds since boot.
    pub fn micros() -> u64 {
        (svc::get_system_tick() as u128 * 1_000_000 / TICKS_PER_SECOND as u128) as u64
    }

    /// Milliseconds since boot.
    pub fn millis() -> u64 {
        micros() / 1000
    }

    /// A point in time, for measuring durations (from the system tick).
    #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct Instant(u64);

    impl Instant {
        pub fn now() -> Instant {
            Instant(ticks())
        }

        pub fn elapsed(&self) -> core::time::Duration {
            Instant::now().duration_since(*self)
        }

        pub fn duration_since(&self, earlier: Instant) -> core::time::Duration {
            let t = self.0.saturating_sub(earlier.0) as u128;
            core::time::Duration::from_nanos((t * 1_000_000_000 / TICKS_PER_SECOND as u128) as u64)
        }

        pub fn ticks(&self) -> u64 {
            self.0
        }
    }

    impl core::ops::Add<core::time::Duration> for Instant {
        type Output = Instant;
        fn add(self, d: core::time::Duration) -> Instant {
            Instant(self.0 + (d.as_nanos() * TICKS_PER_SECOND as u128 / 1_000_000_000) as u64)
        }
    }

    /// Sleeps the current thread.
    pub fn sleep(duration: core::time::Duration) {
        svc::sleep_thread(duration.as_nanos().min(i64::MAX as u128) as i64);
    }

    /// A calendar date and time (the console's clock, local time).
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct DateTime {
        pub year: u16,
        pub month: u8,
        pub day: u8,
        pub hour: u8,
        pub minute: u8,
        pub second: u8,
        pub millisecond: u16,
        /// 0 = Monday.
        pub weekday: u8,
    }

    /// Milliseconds since 1 January 1900 on the console's clock.
    pub fn clock_millis() -> u64 {
        use citronnade_rt::memory::{read_config_u32, SHARED_CONFIG};
        let read64 = |o: u32| read_config_u32(SHARED_CONFIG + o) as u64 | (read_config_u32(SHARED_CONFIG + o + 4) as u64) << 32;
        // The time reference is double buffered: re-read until stable.
        let (mut value_ms, mut value_tick, mut hz, mut drift);
        loop {
            let counter = read_config_u32(SHARED_CONFIG);
            let base = 0x20 + 0x20 * (counter & 1);
            value_ms = read64(base);
            value_tick = read64(base + 8);
            hz = read64(base + 16) as i64;
            drift = read64(base + 24) as i64;
            if read_config_u32(SHARED_CONFIG) == counter {
                break;
            }
        }
        let hz = if hz > 0 { hz } else { TICKS_PER_SECOND as i64 };
        let elapsed_ticks = ticks() as i64 - value_tick as i64;
        let mut elapsed_ms = elapsed_ticks * 1000 / hz;
        const HOUR: i64 = 3_600_000;
        if drift != 0 && elapsed_ms < HOUR {
            elapsed_ms += drift * (HOUR - elapsed_ms) / HOUR;
        }
        (value_ms as i64 + elapsed_ms) as u64
    }

    /// The console's current date and time.
    pub fn now() -> DateTime {
        let ms = clock_millis();
        let days = ms / 86_400_000;
        let in_day = ms % 86_400_000;
        // Days since 1900-01-01 to a civil date (1900-01-01 was a Monday).
        // 1900-01-01 is 25,567 days before 1970-01-01; the civil-from-days
        // algorithm counts from 0000-03-01, 719,468 days before 1970.
        let z = days as i64 - 25_567 + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let y = yoe + era * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = doy - (153 * mp + 2) / 5 + 1;
        let m = if mp < 10 { mp + 3 } else { mp - 9 };
        let y = if m <= 2 { y + 1 } else { y };
        DateTime {
            year: y as u16,
            month: m as u8,
            day: d as u8,
            hour: (in_day / 3_600_000) as u8,
            minute: (in_day / 60_000 % 60) as u8,
            second: (in_day / 1000 % 60) as u8,
            millisecond: (in_day % 1000) as u16,
            weekday: (days % 7) as u8,
        }
    }
}

#[doc(hidden)]
pub mod __private {
    use core::fmt::Debug;

    /// What `main` may return.
    pub trait Termination {
        fn report(self) -> i32;
    }

    impl Termination for () {
        fn report(self) -> i32 {
            0
        }
    }

    impl Termination for i32 {
        fn report(self) -> i32 {
            self
        }
    }

    impl<T: Termination, E: Debug> Termination for Result<T, E> {
        fn report(self) -> i32 {
            match self {
                Ok(v) => v.report(),
                Err(e) => {
                    crate::log!("main returned an error: {:?}", e);
                    1
                }
            }
        }
    }
}
