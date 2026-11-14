//! Nintendo 3DS homebrew in Rust, without devkitPro.
//!
//! A program marks its entry point with [`main`]:
//!
//! ```ignore
//! #![no_std]
//! #![no_main]
//!
//! use citronnade::log;
//!
//! #[citronnade::main]
//! fn main() {
//!     log!("Hello from the 3DS!");
//! }
//! ```
#![no_std]

extern crate alloc;

pub use citronnade_macros::main;
pub use citronnade_rt::{at_exit, exit, log, set_panic_hook, PanicHook};

/// Raw system calls.
pub use citronnade_rt::svc;
/// The heaps: usage statistics and linear (GPU/DSP-visible) memory.
pub use citronnade_rt::heap;
/// What the loader passed: arguments, homebrew detection, service handles.
pub mod env {
    pub use citronnade_rt::args::{args, link_host};
    pub use citronnade_rt::env::*;
}

pub use citronnade_rt::svc::ResultCode;

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
