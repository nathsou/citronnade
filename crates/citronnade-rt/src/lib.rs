//! The runtime of a Citronnade program: startup, exit, memory, system calls
//! and logging. Most programs use it through the `citronnade` crate.
#![cfg_attr(not(test), no_std)]

#[cfg(target_arch = "arm")]
extern crate alloc;

pub mod heap;
pub mod sync;

#[cfg(target_arch = "arm")]
pub mod args;
#[cfg(target_arch = "arm")]
pub mod env;
#[cfg(target_arch = "arm")]
pub mod log;
pub mod math;
#[cfg(target_arch = "arm")]
pub mod memory;
#[cfg(target_arch = "arm")]
mod panic;
#[cfg(target_arch = "arm")]
mod start;
#[cfg(target_arch = "arm")]
pub mod svc;
#[cfg(target_arch = "arm")]
pub mod thread;
#[cfg(target_arch = "arm")]
pub mod tls;

#[cfg(target_arch = "arm")]
pub use panic::{panicking, set_panic_hook, PanicHook};
#[cfg(target_arch = "arm")]
pub use start::{at_exit, exit, set_exit_action, set_return_to_loader, DEFAULT_STACK_SIZE};
