//! mic:u: the microphone. Samples go into a buffer shared with the mic
//! module, used as a ring; its last 4 bytes hold the offset of the newest
//! sample.

use core::sync::atomic::{AtomicU32, Ordering};

use citronnade_rt::svc;

use crate::ipc::Request;
use crate::{srv, Handle, Result};

/// Sample encodings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum Encoding {
    Pcm8 = 0,
    Pcm16 = 1,
    Pcm8Signed = 2,
    Pcm16Signed = 3,
}

/// Sample rates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum Rate {
    Hz32730 = 0,
    Hz16360 = 1,
    Hz10910 = 2,
    Hz8180 = 3,
}

impl Rate {
    pub fn hz(self) -> u32 {
        match self {
            Rate::Hz32730 => 32730,
            Rate::Hz16360 => 16360,
            Rate::Hz10910 => 10910,
            Rate::Hz8180 => 8180,
        }
    }
}

static SESSION: AtomicU32 = AtomicU32::new(0);
static MEMORY: AtomicU32 = AtomicU32::new(0);

fn call(r: Request) -> Result<crate::ipc::Reply> {
    unsafe { r.send(SESSION.load(Ordering::Acquire)) }
}

/// Opens the microphone with `buffer` (page aligned, in the application
/// heap, a multiple of 0x1000 bytes) as the shared sample ring.
///
/// # Safety
/// The buffer must stay valid and unused by Rust code (other than reading
/// samples) until [`exit`].
pub unsafe fn init(buffer: *mut u8, size: usize) -> Result<()> {
    let s = srv::get_service("mic:u")?;
    SESSION.store(s.into_raw(), Ordering::Release);
    let result = (|| -> Result<()> {
        let block = unsafe {
            svc::create_memory_block(buffer as u32, size as u32, svc::perm::READ_WRITE, svc::perm::READ_WRITE)
        }?;
        MEMORY.store(block, Ordering::Release);
        call(Request::new(0x1).word(size as u32).shared_handles(&[block]))?;
        call(Request::new(0xA).word(1))?; // power on
        Ok(())
    })();
    if result.is_err() {
        exit();
    }
    result
}

pub fn exit() {
    let block = MEMORY.swap(0, Ordering::AcqRel);
    if block != 0 {
        let _ = call(Request::new(0x2));
        let _ = svc::close_handle(block);
    }
    let s = SESSION.swap(0, Ordering::AcqRel);
    if s != 0 {
        let _ = call(Request::new(0xA).word(0));
        let _ = svc::close_handle(s);
    }
}

pub fn start_sampling(encoding: Encoding, rate: Rate, offset: u32, size: u32, looping: bool) -> Result<()> {
    call(Request::new(0x3).word(encoding as u32).word(rate as u32).word(offset).word(size).word(looping as u32)).map(|_| ())
}

pub fn stop_sampling() -> Result<()> {
    call(Request::new(0x5)).map(|_| ())
}

pub fn is_sampling() -> Result<bool> {
    call(Request::new(0x6)).map(|r| r.word(2) & 0xFF != 0)
}

/// An event signalled when the ring fills (when not looping).
pub fn event_handle() -> Result<Handle> {
    call(Request::new(0x7)).map(|r| r.word(3))
}

/// Gain, 0-119 (the system's default is 0x2B).
pub fn set_gain(gain: u8) -> Result<()> {
    call(Request::new(0x8).word(gain as u32)).map(|_| ())
}

pub fn gain() -> Result<u8> {
    call(Request::new(0x9)).map(|r| r.word(2) as u8)
}

pub fn set_clamp(clamp: bool) -> Result<()> {
    call(Request::new(0xD).word(clamp as u32)).map(|_| ())
}

pub fn set_allow_shell_closed(allow: bool) -> Result<()> {
    call(Request::new(0xF).word(allow as u32)).map(|_| ())
}
