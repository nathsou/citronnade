//! The cameras: the inner camera, either outer camera, or both outer
//! cameras as a stereo pair. Frames (400x240, RGB565 or YUYV) arrive by
//! DMA; [`Cameras::poll`] returns as soon as a new one is complete, without
//! waiting, so it fits a frame loop. [`Y2r`] converts YUYV frames with the
//! hardware converter.

use alloc::vec::Vec;

use citronnade_rt::svc;
use citronnade_services::{cam, y2r, Handle};

use crate::Result;

/// Which cameras to use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CameraSelect {
    /// The camera facing the player.
    Inner,
    /// One of the outer cameras.
    OuterRight,
    OuterLeft,
    /// Both outer cameras together (a stereo pair; frames come in pairs).
    OuterStereo,
}

impl CameraSelect {
    fn select(self) -> u32 {
        match self {
            CameraSelect::Inner => cam::select::IN1,
            CameraSelect::OuterRight => cam::select::OUT1,
            CameraSelect::OuterLeft => cam::select::OUT2,
            CameraSelect::OuterStereo => cam::select::OUT1_OUT2,
        }
    }

    fn ports(self) -> &'static [u32] {
        match self {
            CameraSelect::OuterLeft => &[cam::port::CAM2],
            CameraSelect::OuterStereo => &[cam::port::CAM1, cam::port::CAM2],
            _ => &[cam::port::CAM1],
        }
    }
}

/// Pixel formats.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PixelFormat {
    Rgb565,
    /// Packed Y0 U Y1 V (two pixels in four bytes).
    Yuyv,
}

pub const WIDTH: u32 = 400;
pub const HEIGHT: u32 = 240;
const FRAME_BYTES: usize = (WIDTH * HEIGHT * 2) as usize;

struct Port {
    port: u32,
    buffer: Vec<u8>,
    /// The completed frame (copied out of the DMA buffer).
    frame: Vec<u8>,
    receive: Option<Handle>,
    error: Handle,
    frames: u32,
}

/// Open cameras, capturing.
pub struct Cameras {
    which: CameraSelect,
    format: PixelFormat,
    ports: Vec<Port>,
    transfer: u32,
    errors: u32,
    reported_frames: u32,
}

impl Cameras {
    /// Opens and starts `which`, 400x240 at 30 frames per second.
    pub fn open(which: CameraSelect, format: PixelFormat) -> Result<Cameras> {
        cam::init()?;
        let mut c = Cameras { which, format, ports: Vec::new(), transfer: 0, errors: 0, reported_frames: 0 };
        if let Err(e) = c.start() {
            drop(c);
            return Err(e);
        }
        Ok(c)
    }

    fn start(&mut self) -> Result<()> {
        let sel = self.which.select();
        let out = match self.format {
            PixelFormat::Rgb565 => cam::OUTPUT_RGB_565,
            PixelFormat::Yuyv => cam::OUTPUT_YUV_422,
        };
        cam::set_size(sel, cam::size::CTR_TOP_LCD, cam::CONTEXT_A)?;
        cam::set_output_format(sel, out, cam::CONTEXT_A)?;
        cam::set_frame_rate(sel, cam::frame_rate::FPS_30)?;
        cam::set_noise_filter(sel, true)?;
        cam::set_auto_exposure(sel, true)?;
        cam::set_auto_white_balance(sel, true)?;
        for &p in self.which.ports() {
            cam::set_trimming(p, false)?;
        }
        self.transfer = cam::max_bytes(WIDTH as u16, HEIGHT as u16)?;
        cam::set_transfer_bytes(cam::port::BOTH, self.transfer, WIDTH as u16, HEIGHT as u16)?;
        cam::activate(sel)?;
        for &p in self.which.ports() {
            let error = cam::buffer_error_event(p)?;
            self.ports.push(Port {
                port: p,
                buffer: alloc::vec![0; FRAME_BYTES],
                frame: alloc::vec![0; FRAME_BYTES],
                receive: None,
                error,
                frames: 0,
            });
        }
        cam::clear_buffer(cam::port::BOTH)?;
        if self.which == CameraSelect::OuterStereo {
            cam::synchronize_vsync_timing(cam::select::OUT1, cam::select::OUT2)?;
        }
        for &p in self.which.ports() {
            cam::start_capture(p)?;
        }
        Ok(())
    }

    fn arm(transfer: u32, p: &mut Port) {
        if p.receive.is_none() {
            p.receive = unsafe { cam::set_receiving(p.buffer.as_mut_ptr(), p.port, FRAME_BYTES as u32, transfer as u16) }.ok();
        }
    }

    /// Checks for new frames without waiting. Returns true when every
    /// camera has delivered a new frame since the last true (both images of
    /// a stereo pair).
    pub fn poll(&mut self) -> bool {
        let transfer = self.transfer;
        let mut restart = false;
        for p in self.ports.iter_mut() {
            Cameras::arm(transfer, p);
            if svc::wait_synchronization(p.error, 0).is_ok() {
                restart = true;
            }
            if let Some(ev) = p.receive {
                if svc::wait_synchronization(ev, 0).is_ok() {
                    let _ = svc::close_handle(ev);
                    p.receive = None;
                    core::mem::swap(&mut p.buffer, &mut p.frame);
                    p.frames += 1;
                    Cameras::arm(transfer, p);
                }
            }
        }
        if restart {
            // A buffer overrun: start over.
            self.errors += 1;
            for p in self.ports.iter_mut() {
                if let Some(ev) = p.receive.take() {
                    let _ = svc::close_handle(ev);
                }
            }
            let _ = cam::clear_buffer(cam::port::BOTH);
            for p in self.ports.iter() {
                let _ = cam::start_capture(p.port);
            }
        }
        let min = self.ports.iter().map(|p| p.frames).min().unwrap_or(0);
        let fresh = min > 0 && self.ports.iter().all(|p| p.frames == min) && min != self.last_reported();
        if fresh {
            self.set_reported(min);
        }
        fresh
    }

    fn last_reported(&self) -> u32 {
        self.reported_frames
    }

    fn set_reported(&mut self, frames: u32) {
        self.reported_frames = frames;
    }

    /// The latest complete frame of camera `i` (0, or 1 for the second
    /// camera of a stereo pair; the left eye is camera 0's).
    pub fn frame(&self, i: usize) -> &[u8] {
        &self.ports[i.min(self.ports.len() - 1)].frame
    }

    /// Frames received from camera `i`.
    pub fn frames(&self, i: usize) -> u32 {
        self.ports.get(i).map_or(0, |p| p.frames)
    }

    /// Buffer overruns recovered from.
    pub fn errors(&self) -> u32 {
        self.errors
    }

    pub fn format(&self) -> PixelFormat {
        self.format
    }

    /// Plays the shutter sound (0 photo, 1 movie start, 2 movie end).
    pub fn shutter(&self, kind: u32) {
        let _ = cam::play_shutter_sound(kind);
    }
}

impl Drop for Cameras {
    fn drop(&mut self) {
        for p in &self.ports {
            let _ = cam::stop_capture(p.port);
        }
        for p in self.ports.iter_mut() {
            if let Some(ev) = p.receive.take() {
                let _ = svc::close_handle(ev);
            }
            let _ = svc::close_handle(p.error);
        }
        let _ = cam::activate(0);
        cam::exit();
    }
}

/// The hardware YUV to RGB converter.
pub struct Y2r {
    event: Handle,
}

impl Y2r {
    pub fn new() -> Result<Y2r> {
        y2r::init()?;
        let r = (|| -> Result<Handle> {
            y2r::set_transfer_end_interrupt(true)?;
            y2r::transfer_end_event()
        })();
        match r {
            Ok(event) => Ok(Y2r { event }),
            Err(e) => {
                y2r::exit();
                Err(e)
            }
        }
    }

    /// Converts a YUYV image (`width` x `height`, rows from the top) to
    /// RGB565 into `out` (2 bytes per pixel), waiting for the converter.
    pub fn yuyv_to_rgb565(&mut self, yuyv: &[u8], width: u16, height: u16, out: &mut [u8]) -> Result<()> {
        let pixels = width as u32 * height as u32;
        assert!(yuyv.len() >= (pixels * 2) as usize && out.len() >= (pixels * 2) as usize);
        y2r::set_params(&y2r::Params {
            input_format: y2r::input::YUYV,
            output_format: y2r::output::RGB565,
            rotation: 0,
            block_alignment: 0,
            width,
            height,
            coefficient: 0,
            alpha: 0xFF,
        })?;
        unsafe {
            y2r::set_sending_yuyv(yuyv.as_ptr(), pixels * 2, width * 2, 0)?;
            y2r::set_receiving(out.as_mut_ptr(), pixels * 2, width * 2 * 8, 0)?;
        }
        y2r::start_conversion()?;
        // A frame converts in a few milliseconds.
        let r = svc::wait_synchronization(self.event, 100_000_000);
        if r.is_err() {
            let _ = y2r::stop_conversion();
        }
        r
    }
}

impl Drop for Y2r {
    fn drop(&mut self) {
        let _ = y2r::stop_conversion();
        let _ = svc::close_handle(self.event);
        y2r::exit();
    }
}
