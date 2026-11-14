//! Line-based logging to the debugger, plus optional extra sinks.
//!
//! [`log!`](crate::log!) formats without allocating and writes each line
//! with `svcOutputDebugString`, which Luma3DS's GDB stub and emulators
//! display. Other parts of the library can register sinks (an on-screen
//! console, a network socket) that receive the same lines.

use core::fmt::{self, Write};
use core::sync::atomic::{AtomicUsize, Ordering};

use crate::svc;

/// A function receiving each logged line (without its newline).
pub type Sink = fn(&str);

const MAX_SINKS: usize = 4;
static SINKS: [AtomicUsize; MAX_SINKS] = [const { AtomicUsize::new(0) }; MAX_SINKS];

/// Adds a sink. Returns false when all slots are taken.
pub fn add_sink(sink: Sink) -> bool {
    for slot in &SINKS {
        if slot.compare_exchange(0, sink as usize, Ordering::AcqRel, Ordering::Relaxed).is_ok() {
            return true;
        }
    }
    false
}

/// Removes a sink added with [`add_sink`].
pub fn remove_sink(sink: Sink) {
    for slot in &SINKS {
        let _ = slot.compare_exchange(sink as usize, 0, Ordering::AcqRel, Ordering::Relaxed);
    }
}

fn emit(line: &str) {
    // A zero-length debug string is a host I/O request for debuggers.
    svc::output_debug_string(if line.is_empty() { b" " } else { line.as_bytes() });
    for slot in &SINKS {
        let f = slot.load(Ordering::Acquire);
        if f != 0 {
            let sink: Sink = unsafe { core::mem::transmute::<usize, Sink>(f) };
            sink(line);
        }
    }
}

/// Buffers output and emits it one line at a time.
pub struct LineWriter {
    buf: [u8; 256],
    len: usize,
}

impl LineWriter {
    pub const fn new() -> Self {
        LineWriter { buf: [0; 256], len: 0 }
    }

    pub fn flush(&mut self) {
        if self.len > 0 {
            // Cut at a character boundary so sinks always get valid UTF-8.
            let mut end = self.len;
            while end > 0 && core::str::from_utf8(&self.buf[..end]).is_err() {
                end -= 1;
            }
            let s = unsafe { core::str::from_utf8_unchecked(&self.buf[..end]) };
            emit(s);
            self.buf.copy_within(end..self.len, 0);
            self.len -= end;
            if end == 0 {
                self.len = 0;
            }
        }
    }
}

impl Default for LineWriter {
    fn default() -> Self {
        Self::new()
    }
}

impl Write for LineWriter {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for &b in s.as_bytes() {
            if b == b'\n' {
                self.flush();
                continue;
            }
            if self.len == self.buf.len() {
                self.flush();
            }
            self.buf[self.len] = b;
            self.len += 1;
        }
        Ok(())
    }
}

#[doc(hidden)]
pub fn _log(args: fmt::Arguments<'_>) {
    let mut w = LineWriter::new();
    let _ = w.write_fmt(args);
    w.flush();
}

/// Logs a line to the debugger and any registered sinks.
///
/// ```ignore
/// log!("loaded {} sprites in {} ms", count, ms);
/// ```
#[macro_export]
macro_rules! log {
    () => { $crate::log::_log(format_args!("")) };
    ($($arg:tt)*) => { $crate::log::_log(format_args!($($arg)*)) };
}
