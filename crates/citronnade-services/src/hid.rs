//! hid:USER: buttons, Circle Pad, touch screen and motion sensors, read from
//! shared memory that the HID module updates continuously.
//!
//! Shared memory (words): pad section at 0 (latest index at word 4, entries
//! of 4 words from word 10: buttons, ..., circle pad), touch at 42, the
//! accelerometer at 66 and the gyroscope at 86, each a ring of samples with
//! its latest index at offset 4.

use core::sync::atomic::{AtomicU32, Ordering};

use citronnade_rt::{memory, svc};

use crate::ipc::{Request, Session};
use crate::{srv, Handle, Result, ResultCode};

static SESSION: AtomicU32 = AtomicU32::new(0);
static MEMORY_HANDLE: AtomicU32 = AtomicU32::new(0);
static MEMORY: AtomicU32 = AtomicU32::new(0);
static EVENTS: [AtomicU32; 5] = [const { AtomicU32::new(0) }; 5];
static REFS: AtomicU32 = AtomicU32::new(0);

fn session() -> Handle {
    SESSION.load(Ordering::Acquire)
}

pub fn init() -> Result<()> {
    if REFS.fetch_add(1, Ordering::AcqRel) > 0 {
        return Ok(());
    }
    let result = (|| -> Result<()> {
        let s: Session = srv::get_service("hid:USER").or_else(|_| srv::get_service("hid:SPVR"))?;
        SESSION.store(s.into_raw(), Ordering::Release);
        // GetIPCHandles: shared memory, then pad 0, pad 1, accelerometer,
        // gyroscope and debug pad events.
        let reply = unsafe { Request::new(0xA).send(session()) }?;
        let mem = reply.word(3);
        MEMORY_HANDLE.store(mem, Ordering::Release);
        for (i, e) in EVENTS.iter().enumerate() {
            e.store(reply.word(4 + i), Ordering::Release);
        }
        let addr = memory::find_mappable(0x1000).ok_or(ResultCode(0xD86007F3))?;
        unsafe { svc::map_memory_block(mem, addr, svc::perm::READ, svc::perm::DONT_CARE) }?;
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
    let addr = MEMORY.swap(0, Ordering::AcqRel);
    let mem = MEMORY_HANDLE.swap(0, Ordering::AcqRel);
    if addr != 0 {
        unsafe {
            let _ = svc::unmap_memory_block(mem, addr);
        }
    }
    for h in core::iter::once(mem).chain(EVENTS.iter().map(|e| e.swap(0, Ordering::AcqRel))) {
        if h != 0 {
            let _ = svc::close_handle(h);
        }
    }
    let s = SESSION.swap(0, Ordering::AcqRel);
    if s != 0 {
        let _ = svc::close_handle(s);
    }
}

/// One reading of every input HID reports.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RawInput {
    /// Button bits (see the `citronnade::input::Buttons` constants).
    pub buttons: u32,
    /// Circle Pad position, roughly -156..156 on each axis.
    pub circle: (i16, i16),
    /// Touch position in bottom-screen pixels, while touched.
    pub touch: Option<(u16, u16)>,
    /// Accelerometer (raw units).
    pub accel: (i16, i16, i16),
    /// Gyroscope angular rates (raw units).
    pub gyro: (i16, i16, i16),
}

fn word(i: usize) -> u32 {
    unsafe { core::ptr::read_volatile((MEMORY.load(Ordering::Acquire) as *const u32).add(i)) }
}

fn half(word_index: usize, half: usize) -> i16 {
    (word(word_index) >> (16 * half)) as i16
}

/// Whether a section's ring holds valid data for entry `index`.
fn section_valid(base: usize, index: u32) -> bool {
    if index != 0 {
        return true;
    }
    let tick0 = word(base) as u64 | (word(base + 1) as u64) << 32;
    let tick1 = word(base + 2) as u64 | (word(base + 3) as u64) << 32;
    !(tick0 == tick1 || (tick0 as i64) < 0 || (tick1 as i64) < 0)
}

/// Reads the latest inputs.
pub fn read() -> RawInput {
    let mut out = RawInput::default();
    if MEMORY.load(Ordering::Acquire) == 0 {
        return out;
    }
    let id = word(4).min(7);
    if section_valid(0, id) {
        let at = 10 + id as usize * 4;
        out.buttons = word(at);
        out.circle = (half(at + 3, 0), half(at + 3, 1));
    }
    let id = word(42 + 4).min(7);
    if section_valid(42, id) {
        let at = 42 + 8 + id as usize * 2;
        if word(at + 1) & 1 != 0 {
            out.touch = Some((word(at) as u16, (word(at) >> 16) as u16));
        }
    }
    let id = word(66 + 4).min(7);
    if section_valid(66, id) {
        // 6-byte samples starting at word 66 + 8.
        let byte = (66 + 8) * 4 + id as usize * 6;
        out.accel = (read_i16(byte), read_i16(byte + 2), read_i16(byte + 4));
    }
    let id = word(86 + 4).min(31);
    if section_valid(86, id) {
        let byte = (86 + 8) * 4 + id as usize * 6;
        out.gyro = (read_i16(byte), read_i16(byte + 2), read_i16(byte + 4));
    }
    out
}

fn read_i16(byte: usize) -> i16 {
    unsafe { core::ptr::read_volatile((MEMORY.load(Ordering::Acquire) as *const u8).add(byte) as *const i16) }
}

/// The pad update event (signalled about every 4 ms).
pub fn pad_event() -> Handle {
    EVENTS[0].load(Ordering::Acquire)
}

pub fn enable_accelerometer() -> Result<()> {
    unsafe { Request::new(0x11).send(session()) }.map(|_| ())
}

pub fn disable_accelerometer() -> Result<()> {
    unsafe { Request::new(0x12).send(session()) }.map(|_| ())
}

pub fn enable_gyroscope() -> Result<()> {
    unsafe { Request::new(0x13).send(session()) }.map(|_| ())
}

pub fn disable_gyroscope() -> Result<()> {
    unsafe { Request::new(0x14).send(session()) }.map(|_| ())
}

/// Degrees per second for one raw gyroscope unit.
pub fn gyroscope_raw_to_dps() -> Result<f32> {
    let reply = unsafe { Request::new(0x15).send(session()) }?;
    Ok(f32::from_bits(reply.word(2)))
}

/// The volume slider, 0 to 63.
pub fn sound_volume() -> Result<u8> {
    let reply = unsafe { Request::new(0x17).send(session()) }?;
    Ok(reply.word(2) as u8)
}
