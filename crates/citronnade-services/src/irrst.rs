//! ir:rst: the New 3DS's C-Stick and ZL/ZR (and the Circle Pad Pro on Old
//! 3DS), read from shared memory like HID.

use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use citronnade_rt::{env, memory, svc};

use crate::ipc::Request;
use crate::{srv, Handle, Result, ResultCode};

static SESSION: AtomicU32 = AtomicU32::new(0);
static MEMORY_HANDLE: AtomicU32 = AtomicU32::new(0);
static EVENT: AtomicU32 = AtomicU32::new(0);
static MEMORY: AtomicU32 = AtomicU32::new(0);
static INITIALIZED: AtomicBool = AtomicBool::new(false);
static REFS: AtomicU32 = AtomicU32::new(0);

fn session() -> Handle {
    SESSION.load(Ordering::Acquire)
}

pub fn init() -> Result<()> {
    if REFS.fetch_add(1, Ordering::AcqRel) > 0 {
        return Ok(());
    }
    let result = (|| -> Result<()> {
        SESSION.store(srv::get_service("ir:rst")?.into_raw(), Ordering::Release);
        let reply = unsafe { Request::new(0x1).send(session()) }?;
        MEMORY_HANDLE.store(reply.word(3), Ordering::Release);
        EVENT.store(reply.word(4), Ordering::Release);
        if env::get_handle("ir:rst").is_none() {
            unsafe { Request::new(0x2).word(10).word(0).send(session()) }?;
            INITIALIZED.store(true, Ordering::Release);
        }
        let addr = memory::find_mappable(0x1000).ok_or(ResultCode(0xD86007F3))?;
        unsafe { svc::map_memory_block(reply.word(3), addr, svc::perm::READ, svc::perm::DONT_CARE) }?;
        MEMORY.store(addr, Ordering::Release);
        Ok(())
    })();
    if result.is_err() {
        exit();
    }
    result
}

pub fn exit() {
    if REFS.fetch_sub(1, Ordering::AcqRel) != 1 {
        return;
    }
    if INITIALIZED.swap(false, Ordering::AcqRel) {
        let _ = unsafe { Request::new(0x3).send(session()) };
    }
    let addr = MEMORY.swap(0, Ordering::AcqRel);
    let mem = MEMORY_HANDLE.swap(0, Ordering::AcqRel);
    if addr != 0 {
        unsafe {
            let _ = svc::unmap_memory_block(mem, addr);
        }
    }
    for h in [mem, EVENT.swap(0, Ordering::AcqRel), SESSION.swap(0, Ordering::AcqRel)] {
        if h != 0 {
            let _ = svc::close_handle(h);
        }
    }
}

pub fn is_active() -> bool {
    MEMORY.load(Ordering::Acquire) != 0
}

fn word(i: usize) -> u32 {
    unsafe { core::ptr::read_volatile((MEMORY.load(Ordering::Acquire) as *const u32).add(i)) }
}

/// Button bits (ZL, ZR, C-Stick directions) and the C-Stick position.
pub fn read() -> (u32, (i16, i16)) {
    if !is_active() {
        return (0, (0, 0));
    }
    let id = word(4).min(7);
    if id == 0 {
        let t0 = word(0) as u64 | (word(1) as u64) << 32;
        let t1 = word(2) as u64 | (word(3) as u64) << 32;
        if t0 == t1 || (t0 as i64) < 0 || (t1 as i64) < 0 {
            return (0, (0, 0));
        }
    }
    let at = 6 + id as usize * 4;
    let stick = word(at + 3);
    (word(at), (stick as i16, (stick >> 16) as i16))
}
