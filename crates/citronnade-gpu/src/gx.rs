//! The transfer engines, driven through GSP's command queue: memory fills,
//! display transfers, texture copies and command list execution. The
//! `*_sync` functions wait for the operation's interrupt.

use citronnade_services::gsp::{self, Interrupt};
use citronnade_rt::svc;

use crate::Result;

/// Queues a raw GX command, waiting for room in GSP's queue.
pub fn submit(command: [u32; 8]) -> Result<()> {
    for _ in 0..100_000 {
        match gsp::submit_gx_command(&command) {
            Ok(()) => return Ok(()),
            Err(e) if e.0 == 0xFFFF_FFFE => svc::sleep_thread(100_000),
            Err(e) => return Err(e),
        }
    }
    Err(citronnade_services::ResultCode(0xFFFF_FFFE))
}

fn run(command: [u32; 8], interrupts: &[Interrupt]) -> Result<()> {
    for &i in interrupts {
        gsp::clear_interrupt(i);
    }
    submit(command)?;
    for &i in interrupts {
        gsp::wait_for(i, false);
    }
    Ok(())
}

/// Pixel formats of display transfers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum TransferFormat {
    Rgba8 = 0,
    Rgb8 = 1,
    Rgb565 = 2,
    Rgb5A1 = 3,
    Rgba4 = 4,
}

/// Display transfer flags.
pub mod flags {
    /// Flip the image vertically.
    pub const FLIP_VERTICAL: u32 = 1 << 0;
    /// Linear input, tiled output (for making textures); the default is
    /// tiled input (a render target), linear output (a framebuffer).
    pub const OUT_TILED: u32 = 1 << 1;
    /// Copy without format conversion.
    pub const RAW_COPY: u32 = 1 << 3;
    /// Use 32x32 tiles instead of 8x8.
    pub const BLOCK_32: u32 = 1 << 16;
    pub const fn input_format(f: super::TransferFormat) -> u32 {
        (f as u32) << 8
    }
    pub const fn output_format(f: super::TransferFormat) -> u32 {
        (f as u32) << 12
    }
    /// Downscale 2x horizontally (antialiasing).
    pub const SCALE_X: u32 = 1 << 24;
    /// Downscale 2x in both directions.
    pub const SCALE_XY: u32 = 2 << 24;
}

/// A transfer's dimensions word.
pub const fn dim(width: u32, height: u32) -> u32 {
    (height << 16) | width
}

/// Memory fill widths.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FillWidth {
    Bits16 = 0,
    Bits24 = 1,
    Bits32 = 2,
}

/// One memory fill: fills `[start, end)` with `value`.
#[derive(Clone, Copy, Debug)]
pub struct Fill {
    pub start: *mut u8,
    pub end: *mut u8,
    pub value: u32,
    pub width: FillWidth,
}

/// Fills one or two memory ranges (VRAM or linear memory) and waits.
pub fn memory_fill_sync(first: Fill, second: Option<Fill>) -> Result<()> {
    let ctrl = |f: &Fill| 1 | (f.width as u32) << 8;
    let (b1, v1, e1, c1) = match &second {
        Some(f) => (f.start as u32, f.value, f.end as u32, ctrl(f)),
        None => (0, 0, 0, 0),
    };
    let command = [0x0100_0102, first.start as u32, first.value, first.end as u32, b1, v1, e1, ctrl(&first) | c1 << 16];
    // With both buffers set, GSP raises PSC0 once both are done.
    run(command, &[Interrupt::Psc0])
}

/// Converts an image between formats and layouts (render target to
/// framebuffer, typically) and waits.
pub fn display_transfer_sync(src: *const u8, src_dim: u32, dst: *mut u8, dst_dim: u32, flags: u32) -> Result<()> {
    run([0x0100_0103, src as u32, dst as u32, src_dim, dst_dim, flags, 0, 0], &[Interrupt::Ppf])
}

/// Copies `size` bytes, optionally with gaps: `in_dim`/`out_dim` are
/// `(gap << 16) | line width` in 16-byte units. Waits.
pub fn texture_copy_sync(src: *const u8, dst: *mut u8, size: u32, in_dim: u32, out_dim: u32, flags: u32) -> Result<()> {
    run([0x0100_0104, src as u32, dst as u32, size, in_dim, out_dim, flags | flags::RAW_COPY, 0], &[Interrupt::Ppf])
}

/// Runs a command list (from [`crate::CommandList::finish`]) and waits for
/// the GPU to finish it.
pub fn process_command_list_sync(list: *const u32, bytes: usize) -> Result<()> {
    run([0x0100_0101, list as u32, bytes as u32, 0, 0, 0, 0, 0], &[Interrupt::P3d])
}
