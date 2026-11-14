//! The raw layer between programs and the system: IPC requests and the
//! system services, each a thin, typed wrapper over its commands.
//!
//! Service behaviour (initialisation order, the APT applet lifecycle, GSP's
//! shared memory protocol) follows libctru, the reference implementation
//! (zlib licence), reimplemented here in Rust.
#![no_std]

extern crate alloc;

pub mod apt;
pub mod cfg;
pub mod gsp;
pub mod hid;
pub mod ipc;
pub mod irrst;
pub mod ptm;
pub mod srv;

pub use citronnade_rt::svc::{Handle, ResultCode};

/// The result of a service call.
pub type Result<T> = core::result::Result<T, ResultCode>;
