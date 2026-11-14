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

pub mod console;
pub mod font;
pub mod gfx;
pub mod input;
pub mod math;
mod panic_screen;
pub mod system;

pub use citronnade_macros::main;
pub use citronnade_rt::{at_exit, exit, log, set_panic_hook, PanicHook};
pub use console::Console;
pub use gfx::{Canvas, Color, Format, Image, Screen, Screens, TopMode};
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

    /// Sleeps the current thread.
    pub fn sleep(duration: core::time::Duration) {
        svc::sleep_thread(duration.as_nanos().min(i64::MAX as u128) as i64);
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
