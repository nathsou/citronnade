//! The microphone: 16-bit samples recorded into a ring shared with the mic
//! module.

use alloc::vec::Vec;
use core::alloc::Layout;

use citronnade_services::mic as raw;

pub use raw::Rate;

use crate::Result;

/// An open microphone, sampling continuously.
pub struct Microphone {
    buffer: *mut u8,
    size: usize,
    rate: Rate,
    read_pos: usize,
}

impl Microphone {
    /// Starts sampling signed 16-bit audio at `rate` into a ring of
    /// `ring_bytes` (rounded up to whole pages, 4 bytes of which hold the
    /// write position).
    pub fn new(rate: Rate, ring_bytes: usize) -> Result<Microphone> {
        let size = (ring_bytes + 0xFFF) & !0xFFF;
        let layout = Layout::from_size_align(size, 0x1000).unwrap();
        let buffer = unsafe { alloc::alloc::alloc_zeroed(layout) };
        if buffer.is_null() {
            return Err(crate::ResultCode(0xD86007F3));
        }
        if let Err(e) = unsafe { raw::init(buffer, size) } {
            unsafe { alloc::alloc::dealloc(buffer, layout) };
            return Err(e);
        }
        let data = (size - 4) as u32;
        let _ = raw::set_gain(0x2B);
        if let Err(e) = raw::start_sampling(raw::Encoding::Pcm16Signed, rate, 0, data, true) {
            raw::exit();
            unsafe { alloc::alloc::dealloc(buffer, layout) };
            return Err(e);
        }
        Ok(Microphone { buffer, size, rate, read_pos: 0 })
    }

    pub fn rate(&self) -> Rate {
        self.rate
    }

    fn write_pos(&self) -> usize {
        let p = unsafe { (self.buffer.add(self.size - 4) as *const u32).read_volatile() } as usize;
        p.min(self.size - 4) & !1
    }

    /// Appends the samples recorded since the last call.
    pub fn read(&mut self, out: &mut Vec<i16>) -> usize {
        let end = self.write_pos();
        let data = self.size - 4;
        let mut n = 0;
        while self.read_pos != end {
            let s = unsafe { (self.buffer.add(self.read_pos) as *const i16).read_volatile() };
            out.push(s);
            n += 1;
            self.read_pos = (self.read_pos + 2) % (data & !1);
        }
        n
    }

    /// The loudest of the last `count` samples (0 to 32767).
    pub fn level(&self, count: usize) -> u16 {
        let data = (self.size - 4) & !1;
        let end = self.write_pos();
        let mut peak = 0u16;
        for k in 1..=count.min(data / 2) {
            let pos = (end + data - 2 * k) % data;
            let s = unsafe { (self.buffer.add(pos) as *const i16).read_volatile() };
            peak = peak.max(s.unsigned_abs());
        }
        peak
    }

    pub fn set_gain(&self, gain: u8) -> Result<()> {
        raw::set_gain(gain)
    }
}

impl Drop for Microphone {
    fn drop(&mut self) {
        let _ = raw::stop_sampling();
        raw::exit();
        unsafe { alloc::alloc::dealloc(self.buffer, Layout::from_size_align(self.size, 0x1000).unwrap()) };
    }
}
