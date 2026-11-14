//! Command-line arguments passed by the loader.
//!
//! The Homebrew Launcher passes the path of the 3DSX as the first argument;
//! 3dslink can pass more, and appends the host's address as the last one
//! (`3dslink:/` followed by the address) when it wants output sent back.

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};

static DATA: AtomicPtr<&'static str> = AtomicPtr::new(core::ptr::null_mut());
static COUNT: AtomicUsize = AtomicUsize::new(0);

/// Copies the loader's argument list to the heap. Called once at startup.
pub(crate) fn init() {
    let list = crate::env::arg_list();
    if list.is_null() {
        return;
    }
    unsafe {
        let argc = (list as *const u32).read_unaligned() as usize;
        let mut p = list.add(4);
        let mut args: Vec<&'static str> = Vec::with_capacity(argc);
        for _ in 0..argc {
            let mut len = 0;
            while *p.add(len) != 0 {
                len += 1;
            }
            let bytes: &'static [u8] = Box::leak(core::slice::from_raw_parts(p, len).into());
            args.push(core::str::from_utf8(bytes).unwrap_or(""));
            p = p.add(len + 1);
        }
        let args = Box::leak(args.into_boxed_slice());
        COUNT.store(args.len(), Ordering::Relaxed);
        DATA.store(args.as_mut_ptr(), Ordering::Release);
    }
}

/// The program's arguments.
pub fn args() -> &'static [&'static str] {
    let data = DATA.load(Ordering::Acquire);
    if data.is_null() {
        return &[];
    }
    unsafe { core::slice::from_raw_parts(data, COUNT.load(Ordering::Relaxed)) }
}

/// The host address 3dslink appended, if any (`3dslink:/` + an IPv4 address
/// as 8 hex digits, as the Homebrew Launcher passes it).
pub fn link_host() -> Option<[u8; 4]> {
    let last = *args().last()?;
    let hex = last.strip_prefix("3dslink:/")?;
    if hex.len() != 8 {
        return None;
    }
    let v = u32::from_str_radix(hex, 16).ok()?;
    Some(v.to_be_bytes())
}
