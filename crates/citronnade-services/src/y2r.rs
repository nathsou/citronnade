//! y2r:u: the YUV to RGB converter (for camera and video frames), with
//! optional rotation and output tiled for the GPU.

use core::sync::atomic::{AtomicU32, Ordering};

use citronnade_rt::svc;

use crate::ipc::Request;
use crate::{srv, Handle, Result};

/// Input formats.
pub mod input {
    pub const YUV422_INDIVIDUAL_8: u8 = 0;
    pub const YUV420_INDIVIDUAL_8: u8 = 1;
    pub const YUV422_INDIVIDUAL_16: u8 = 2;
    pub const YUV420_INDIVIDUAL_16: u8 = 3;
    /// Packed Y0 U Y1 V (what the cameras send).
    pub const YUYV: u8 = 4;
}

/// Output formats.
pub mod output {
    pub const RGBA8: u8 = 0;
    pub const RGB8: u8 = 1;
    pub const RGB5551: u8 = 2;
    pub const RGB565: u8 = 3;
}

/// Conversion settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Params {
    pub input_format: u8,
    pub output_format: u8,
    /// 0 none, 1 90° clockwise, 2 180°, 3 270°.
    pub rotation: u8,
    /// 0 rows, 1 8x8 blocks (the GPU's texture layout).
    pub block_alignment: u8,
    pub width: u16,
    pub height: u16,
    /// 0 BT.601, 1 BT.709, 2 BT.601 scaled, 3 BT.709 scaled.
    pub coefficient: u8,
    pub alpha: u16,
}

static SESSION: AtomicU32 = AtomicU32::new(0);
static REFS: AtomicU32 = AtomicU32::new(0);

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
    let r = (|| -> Result<()> {
        SESSION.store(srv::get_service("y2r:u")?.into_raw(), Ordering::Release);
        call(Request::new(0x2B)).map(|_| ())
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
        let _ = unsafe { Request::new(0x2C).send(s) };
        let _ = svc::close_handle(s);
    }
}

pub fn set_params(p: &Params) -> Result<()> {
    let w1 = p.input_format as u32 | (p.output_format as u32) << 8 | (p.rotation as u32) << 16 | (p.block_alignment as u32) << 24;
    let w2 = p.width as u32 | (p.height as u32) << 16;
    let w3 = p.coefficient as u32 | (p.alpha as u32) << 16;
    call(Request::new(0x29).word(w1).word(w2).word(w3).word(0).word(0).word(0).word(0)).map(|_| ())
}

pub fn set_transfer_end_interrupt(on: bool) -> Result<()> {
    call(Request::new(0xD).word(on as u32)).map(|_| ())
}

pub fn transfer_end_event() -> Result<Handle> {
    call(Request::new(0xF)).map(|r| r.word(3))
}

/// The YUYV source image.
///
/// # Safety
/// `src` must stay valid until the conversion ends.
pub unsafe fn set_sending_yuyv(src: *const u8, size: u32, transfer_unit: u16, gap: u16) -> Result<()> {
    call(Request::new(0x13).word(src as u32).word(size).word(transfer_unit as u32).word(gap as u32).shared_handles(&[0xFFFF_8001]))
        .map(|_| ())
}

/// Where the RGB image goes.
///
/// # Safety
/// `dst` must stay valid until the conversion ends.
pub unsafe fn set_receiving(dst: *mut u8, size: u32, transfer_unit: u16, gap: u16) -> Result<()> {
    call(Request::new(0x18).word(dst as u32).word(size).word(transfer_unit as u32).word(gap as u32).shared_handles(&[0xFFFF_8001]))
        .map(|_| ())
}

pub fn start_conversion() -> Result<()> {
    call(Request::new(0x26)).map(|_| ())
}

pub fn stop_conversion() -> Result<()> {
    call(Request::new(0x27)).map(|_| ())
}

pub fn is_busy() -> Result<bool> {
    call(Request::new(0x28)).map(|r| r.word(2) & 0xFF != 0)
}

pub fn is_done_receiving() -> Result<bool> {
    call(Request::new(0x19)).map(|r| r.word(2) & 0xFF != 0)
}
