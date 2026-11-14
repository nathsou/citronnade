//! cam:u: the three cameras (two outer, one inner). Images arrive by DMA
//! into the application's memory; an event signals each complete frame.

use core::sync::atomic::{AtomicU32, Ordering};

use citronnade_rt::svc;

use crate::ipc::Request;
use crate::{srv, Handle, Result};

/// Which cameras a call configures.
pub mod select {
    pub const OUT1: u32 = 1;
    pub const IN1: u32 = 2;
    pub const OUT2: u32 = 4;
    pub const IN1_OUT1: u32 = 3;
    pub const OUT1_OUT2: u32 = 5;
    pub const IN1_OUT2: u32 = 6;
    pub const ALL: u32 = 7;
}

/// Capture ports: port 1 carries OUT1 or IN1, port 2 carries OUT2.
pub mod port {
    pub const CAM1: u32 = 1;
    pub const CAM2: u32 = 2;
    pub const BOTH: u32 = 3;
}

/// Image sizes.
pub mod size {
    pub const VGA: u32 = 0; // 640x480
    pub const QVGA: u32 = 1; // 320x240
    pub const QQVGA: u32 = 2; // 160x120
    pub const CIF: u32 = 3; // 352x288
    pub const QCIF: u32 = 4; // 176x144
    pub const DS_LCD: u32 = 5; // 256x192
    pub const DS_LCD_X4: u32 = 6; // 512x384
    pub const CTR_TOP_LCD: u32 = 7; // 400x240
}

/// Frame rates.
pub mod frame_rate {
    pub const FPS_15: u32 = 0;
    pub const FPS_10: u32 = 3;
    pub const FPS_5: u32 = 5;
    pub const FPS_20: u32 = 6;
    pub const FPS_30: u32 = 8;
    pub const FPS_30_TO_10: u32 = 12;
}

pub const OUTPUT_YUV_422: u32 = 0;
pub const OUTPUT_RGB_565: u32 = 1;
pub const CONTEXT_A: u32 = 1;
pub const CONTEXT_BOTH: u32 = 3;
pub const SHUTTER_NORMAL: u32 = 0;
pub const SHUTTER_MOVIE: u32 = 1;
pub const SHUTTER_MOVIE_END: u32 = 2;

static SESSION: AtomicU32 = AtomicU32::new(0);
static REFS: AtomicU32 = AtomicU32::new(0);

fn session() -> Handle {
    SESSION.load(Ordering::Acquire)
}

fn call(r: Request) -> Result<crate::ipc::Reply> {
    unsafe { r.send(session()) }
}

/// Opens the service and powers the camera driver.
pub fn init() -> Result<()> {
    if REFS.fetch_add(1, Ordering::AcqRel) > 0 {
        return Ok(());
    }
    let r = (|| -> Result<()> {
        SESSION.store(srv::get_service("cam:u")?.into_raw(), Ordering::Release);
        call(Request::new(0x39)).map(|_| ())
    })();
    if r.is_err() {
        REFS.store(1, Ordering::Release);
        exit();
    }
    r
}

pub fn exit() {
    if REFS.fetch_sub(1, Ordering::AcqRel) != 1 {
        return;
    }
    let s = SESSION.swap(0, Ordering::AcqRel);
    if s != 0 {
        let _ = unsafe { Request::new(0x3A).send(s) };
        let _ = svc::close_handle(s);
    }
}

pub fn start_capture(port: u32) -> Result<()> {
    call(Request::new(0x1).word(port)).map(|_| ())
}

pub fn stop_capture(port: u32) -> Result<()> {
    call(Request::new(0x2).word(port)).map(|_| ())
}

pub fn is_busy(port: u32) -> Result<bool> {
    call(Request::new(0x3).word(port)).map(|r| r.word(2) & 0xFF != 0)
}

pub fn clear_buffer(port: u32) -> Result<()> {
    call(Request::new(0x4).word(port)).map(|_| ())
}

pub fn vsync_event(port: u32) -> Result<Handle> {
    call(Request::new(0x5).word(port)).map(|r| r.word(3))
}

pub fn buffer_error_event(port: u32) -> Result<Handle> {
    call(Request::new(0x6).word(port)).map(|r| r.word(3))
}

/// Starts receiving one frame of `size` bytes into `dst` (in chunks of
/// `transfer_unit` bytes); returns the event signalled when it is complete.
///
/// # Safety
/// `dst` must stay valid and untouched until the event fires.
pub unsafe fn set_receiving(dst: *mut u8, port: u32, size: u32, transfer_unit: u16) -> Result<Handle> {
    call(Request::new(0x7).word(dst as u32).word(port).word(size).word(transfer_unit as u32).shared_handles(&[0xFFFF_8001]))
        .map(|r| r.word(3))
}

pub fn is_finished_receiving(port: u32) -> Result<bool> {
    call(Request::new(0x8).word(port)).map(|r| r.word(2) & 0xFF != 0)
}

pub fn set_transfer_bytes(port: u32, bytes: u32, width: u16, height: u16) -> Result<()> {
    call(Request::new(0xB).word(port).word(bytes).word(width as u32).word(height as u32)).map(|_| ())
}

/// The largest transfer unit for an image size.
pub fn max_bytes(width: u16, height: u16) -> Result<u32> {
    call(Request::new(0xD).word(width as u32).word(height as u32)).map(|r| r.word(2))
}

pub fn set_trimming(port: u32, trimming: bool) -> Result<()> {
    call(Request::new(0xE).word(port).word(trimming as u32)).map(|_| ())
}

/// Powers on the selected cameras (0: all off).
pub fn activate(select: u32) -> Result<()> {
    call(Request::new(0x13).word(select)).map(|_| ())
}

pub fn set_auto_exposure(select: u32, on: bool) -> Result<()> {
    call(Request::new(0x19).word(select).word(on as u32)).map(|_| ())
}

pub fn set_auto_white_balance(select: u32, on: bool) -> Result<()> {
    call(Request::new(0x1B).word(select).word(on as u32)).map(|_| ())
}

/// 0 none, 1 horizontal, 2 vertical, 3 both.
pub fn flip_image(select: u32, flip: u32, context: u32) -> Result<()> {
    call(Request::new(0x1D).word(select).word(flip).word(context)).map(|_| ())
}

pub fn set_size(select: u32, size: u32, context: u32) -> Result<()> {
    call(Request::new(0x1F).word(select).word(size).word(context)).map(|_| ())
}

pub fn set_frame_rate(select: u32, rate: u32) -> Result<()> {
    call(Request::new(0x20).word(select).word(rate)).map(|_| ())
}

pub fn set_output_format(select: u32, format: u32, context: u32) -> Result<()> {
    call(Request::new(0x25).word(select).word(format).word(context)).map(|_| ())
}

pub fn set_noise_filter(select: u32, on: bool) -> Result<()> {
    call(Request::new(0x28).word(select).word(on as u32)).map(|_| ())
}

/// Starts two cameras' frames together (for stereo pairs).
pub fn synchronize_vsync_timing(select1: u32, select2: u32) -> Result<()> {
    call(Request::new(0x29).word(select1).word(select2)).map(|_| ())
}

pub fn play_shutter_sound(kind: u32) -> Result<()> {
    call(Request::new(0x38).word(kind)).map(|_| ())
}
