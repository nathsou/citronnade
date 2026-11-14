//! Command-line arguments passed by the loader.
//!
//! The Homebrew Launcher passes the path of the 3DSX as the first argument;
//! 3dslink can pass more. When a program arrives over 3dslink, the launcher
//! also appends the sending PC's address as a last argument,
//! `XXXXXXXX_3DSLINK_` (the address's `s_addr` in hex); like libctru, it is
//! taken out of [`args`] and offered by [`link_host`].

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};

static DATA: AtomicPtr<&'static str> = AtomicPtr::new(core::ptr::null_mut());
static COUNT: AtomicUsize = AtomicUsize::new(0);
static LINK_HOST: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);

/// Decodes the launcher's `XXXXXXXX_3DSLINK_` argument.
fn parse_link_host(arg: &str) -> Option<[u8; 4]> {
    let b = arg.as_bytes();
    if b.len() != 17 || &b[8..] != b"_3DSLINK_" {
        return None;
    }
    // s_addr holds the address's bytes in network order; read as a
    // little-endian number it prints reversed.
    u32::from_str_radix(arg.get(..8)?, 16).ok().map(u32::to_le_bytes)
}

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
        if args.len() > 1 {
            if let Some(ip) = args.last().and_then(|a| parse_link_host(a)) {
                LINK_HOST.store(u32::from_le_bytes(ip), Ordering::Release);
                args.pop();
            }
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

/// The address of the PC that sent this program with 3dslink, if it did.
pub fn link_host() -> Option<[u8; 4]> {
    let v = LINK_HOST.load(Ordering::Acquire);
    (v != 0).then(|| v.to_le_bytes())
}

#[cfg(test)]
mod tests {
    #[test]
    fn link_host_argument() {
        // 192.168.1.20 has s_addr 0x1401A8C0 on a little-endian machine.
        assert_eq!(super::parse_link_host("1401a8c0_3DSLINK_"), Some([192, 168, 1, 20]));
        assert_eq!(super::parse_link_host("3dslink:/c0a80114"), None);
        assert_eq!(super::parse_link_host("1401a8c0_3DSLINKX"), None);
    }
}
