//! What the loader tells the program: the 3DSX header words it patches.

use crate::svc::{self, Handle};

unsafe extern "C" {
    static __service_ptr: *const ServiceList;
    static __apt_appid: u32;
    static __system_arglist: *const u8;
    static __system_runflags: u32;
}

#[repr(C)]
struct ServiceList {
    count: u32,
    entries: [ServiceEntry; 0],
}

#[repr(C)]
struct ServiceEntry {
    name: [u8; 8],
    handle: Handle,
}

fn service_list() -> *const ServiceList {
    unsafe { core::ptr::read_volatile(&raw const __service_ptr) }
}

/// Whether a homebrew loader started the program (it then passes a service
/// list, possibly empty, and expects the program to return to it on exit).
pub fn is_homebrew() -> bool {
    !service_list().is_null()
}

/// The applet ID the program runs as (0x300 for applications).
pub fn apt_app_id() -> u32 {
    unsafe { core::ptr::read_volatile(&raw const __apt_appid) }
}

/// Flags the loader sets to signal restrictions.
pub fn run_flags() -> u32 {
    unsafe { core::ptr::read_volatile(&raw const __system_runflags) }
}

/// Run flag: APT is not available (some exploit entry points).
pub const RUNFLAG_APT_WORKAROUND: u32 = 1 << 0;
/// Run flag: APT may be used but the program must not reinitialise it.
pub const RUNFLAG_APT_REINIT: u32 = 1 << 1;
/// Run flag: the program was started from the Home Menu as a "chainloaded" title.
pub const RUNFLAG_APT_CHAINLOAD: u32 = 1 << 2;

pub(crate) fn arg_list() -> *const u8 {
    unsafe { core::ptr::read_volatile(&raw const __system_arglist) }
}

/// A service handle the loader passed for `name`, if any. Loaders may hand
/// over handles to services the program could not otherwise open.
pub fn get_handle(name: &str) -> Option<Handle> {
    let list = service_list();
    if list.is_null() {
        return None;
    }
    let name = name.as_bytes();
    unsafe {
        let count = (*list).count as usize;
        let entries = (&raw const (*list).entries).cast::<ServiceEntry>();
        for i in 0..count {
            let entry = &*entries.add(i);
            let len = entry.name.iter().position(|&b| b == 0).unwrap_or(8);
            if &entry.name[..len] == name {
                return Some(entry.handle);
            }
        }
    }
    None
}

/// Closes the handles the loader passed. Called on exit.
pub(crate) fn destroy_handles() {
    let list = service_list() as *mut ServiceList;
    if list.is_null() {
        return;
    }
    unsafe {
        let count = (*list).count as usize;
        let entries = (&raw const (*list).entries).cast::<ServiceEntry>();
        for i in 0..count {
            let _ = svc::close_handle((*entries.add(i)).handle);
        }
        (*list).count = 0;
    }
}
