//! dsp::DSP: loads the DSP firmware ("component") and gives access to the
//! DSP's pipes, semaphores and shared memory. The NDSP audio driver in the
//! `citronnade` crate is built on it.
//!
//! The firmware is not part of the system's memory: programs load it from
//! the user's own dump (`/3ds/dspfirm.cdc` on the SD card). When the
//! console sleeps or another applet takes over, APT asks us to unload it;
//! we reload it on wake-up.

use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use citronnade_rt::svc;
use citronnade_rt::sync::Mutex;

use crate::apt::{self, DspHandlers};
use crate::ipc::Request;
use crate::{srv, Handle, Result};

static SESSION: AtomicU32 = AtomicU32::new(0);
static REFS: AtomicU32 = AtomicU32::new(0);
static LOADED: AtomicBool = AtomicBool::new(false);
static SLEEPING: AtomicBool = AtomicBool::new(false);
static COMPONENT: Mutex<Option<(Vec<u8>, u16, u16)>> = Mutex::new(None);
static HOOK: Mutex<Option<fn(Hook)>> = Mutex::new(None);

/// What the audio driver is told around sleep and applet switches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hook {
    Sleep,
    Wakeup,
    Cancel,
}

/// DSP interrupt types.
pub const INTERRUPT_PIPE: u32 = 2;

fn session() -> Handle {
    SESSION.load(Ordering::Acquire)
}

fn call(r: Request) -> Result<crate::ipc::Reply> {
    unsafe { r.send(session()) }
}

pub fn init() -> Result<()> {
    if REFS.fetch_add(1, Ordering::AcqRel) > 0 {
        return Ok(());
    }
    match srv::get_service("dsp::DSP") {
        Ok(s) => SESSION.store(s.into_raw(), Ordering::Release),
        Err(e) => {
            REFS.fetch_sub(1, Ordering::AcqRel);
            return Err(e);
        }
    }
    // Unload whatever a previous program left.
    LOADED.store(true, Ordering::Release);
    let _ = unload_component();
    SLEEPING.store(false, Ordering::Release);
    apt::set_dsp_handlers(Some(DspHandlers { sleep: on_sleep, wakeup: on_wakeup, cancel: on_cancel }));
    Ok(())
}

pub fn exit() {
    if REFS.fetch_sub(1, Ordering::AcqRel) != 1 {
        return;
    }
    apt::set_dsp_handlers(None);
    let _ = unload_component();
    *COMPONENT.lock() = None;
    let s = SESSION.swap(0, Ordering::AcqRel);
    if s != 0 {
        let _ = svc::close_handle(s);
    }
}

/// Sets the function called on sleep, wake-up and cancellation.
pub fn set_hook(hook: Option<fn(Hook)>) {
    *HOOK.lock() = hook;
}

fn call_hook(h: Hook) {
    let hook = *HOOK.lock();
    if let Some(f) = hook {
        f(h);
    }
}

fn on_sleep() -> bool {
    if !LOADED.load(Ordering::Acquire) || SLEEPING.load(Ordering::Acquire) {
        return false;
    }
    call_hook(Hook::Sleep);
    let _ = unload_component();
    SLEEPING.store(true, Ordering::Release);
    true
}

fn on_wakeup() {
    if !SLEEPING.load(Ordering::Acquire) {
        return;
    }
    let component = COMPONENT.lock().as_ref().map(|(b, p, d)| (b.as_ptr(), b.len(), *p, *d));
    if let Some((ptr, len, p, d)) = component {
        let data = unsafe { core::slice::from_raw_parts(ptr, len) };
        if load_raw(data, p, d).is_err() {
            svc::break_(svc::break_reason::PANIC);
        }
    }
    call_hook(Hook::Wakeup);
    SLEEPING.store(false, Ordering::Release);
}

fn on_cancel() {
    if SLEEPING.load(Ordering::Acquire) {
        call_hook(Hook::Cancel);
    }
}

fn load_raw(component: &[u8], prog_mask: u16, data_mask: u16) -> Result<bool> {
    let r = call(
        Request::new(0x11)
            .word(component.len() as u32)
            .word(prog_mask as u32)
            .word(data_mask as u32)
            .read_buffer(component.as_ptr(), component.len()),
    )?;
    let loaded = r.word(2) & 0xFF != 0;
    LOADED.store(loaded, Ordering::Release);
    Ok(loaded)
}

/// Loads the DSP firmware (kept for reloading after sleep).
pub fn load_component(component: Vec<u8>, prog_mask: u16, data_mask: u16) -> Result<bool> {
    let loaded = load_raw(&component, prog_mask, data_mask)?;
    *COMPONENT.lock() = Some((component, prog_mask, data_mask));
    Ok(loaded)
}

pub fn unload_component() -> Result<()> {
    if !LOADED.swap(false, Ordering::AcqRel) {
        return Ok(());
    }
    call(Request::new(0x12)).map(|_| ())
}

pub fn is_component_loaded() -> bool {
    LOADED.load(Ordering::Acquire)
}

pub fn recv_data(register: u32) -> Result<u16> {
    call(Request::new(0x1).word(register)).map(|r| r.word(2) as u16)
}

pub fn recv_data_is_ready(register: u32) -> Result<bool> {
    call(Request::new(0x2).word(register)).map(|r| r.word(2) & 0xFF != 0)
}

pub fn set_semaphore(value: u16) -> Result<()> {
    call(Request::new(0x7).word(value as u32)).map(|_| ())
}

/// Converts a DSP data RAM address to an address in our address space.
pub fn convert_process_address_from_dsp_dram(dsp: u32) -> Result<u32> {
    call(Request::new(0xC).word(dsp)).map(|r| r.word(2))
}

pub fn write_process_pipe(channel: u32, data: &[u8]) -> Result<()> {
    call(Request::new(0xD).word(channel).word(data.len() as u32).static_buffer(data.as_ptr(), data.len(), 1)).map(|_| ())
}

/// Reads from a pipe; returns the bytes read.
pub fn read_pipe_if_possible(channel: u32, peer: u32, buf: &mut [u8]) -> Result<usize> {
    let mut rx = [(buf.as_mut_ptr(), buf.len())];
    let r = unsafe {
        Request::new(0x10).word(channel).word(peer).word(buf.len() as u32).send_receiving(session(), &mut rx)
    }?;
    Ok((r.word(2) & 0xFFFF) as usize)
}

pub fn flush_data_cache(addr: *const u8, size: usize) -> Result<()> {
    call(Request::new(0x13).word(addr as u32).word(size as u32).shared_handles(&[svc::CURRENT_PROCESS])).map(|_| ())
}

pub fn invalidate_data_cache(addr: *const u8, size: usize) -> Result<()> {
    call(Request::new(0x14).word(addr as u32).word(size as u32).shared_handles(&[svc::CURRENT_PROCESS])).map(|_| ())
}

/// Registers (or with handle 0, unregisters) an event for DSP interrupts.
pub fn register_interrupt_events(event: Handle, interrupt: u32, channel: u32) -> Result<()> {
    call(Request::new(0x15).word(interrupt).word(channel).shared_handles(&[event])).map(|_| ())
}

pub fn get_semaphore_handle() -> Result<Handle> {
    call(Request::new(0x16)).map(|r| r.word(3))
}

pub fn set_semaphore_mask(mask: u16) -> Result<()> {
    call(Request::new(0x17).word(mask as u32)).map(|_| ())
}

pub fn headphones_inserted() -> Result<bool> {
    call(Request::new(0x1F)).map(|r| r.word(2) & 0xFF != 0)
}
