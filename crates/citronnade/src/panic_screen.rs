//! Shows panics on the bottom screen and waits for START before exiting.

use core::fmt::Write;
use core::panic::PanicInfo;

use citronnade_rt::sync::Mutex as SpinLock;
use citronnade_rt::svc;
use citronnade_services::{gsp, hid};

use crate::gfx::{Canvas, Color, Format, Screens};
use crate::input::Buttons;

#[derive(Clone, Copy)]
struct Target {
    buffers: [usize; 2],
    format: Format,
}

static TARGET: SpinLock<Option<Target>> = SpinLock::new(None);

pub(crate) fn register(screens: &Screens) {
    let (buffers, format, _) = screens.bottom_buffers();
    *TARGET.lock() = Some(Target { buffers, format });
    citronnade_rt::set_panic_hook(hook);
}

pub(crate) fn update(screens: &Screens) {
    let (buffers, format, _) = screens.bottom_buffers();
    if let Some(mut t) = TARGET.try_lock() {
        *t = Some(Target { buffers, format });
    }
}

pub(crate) fn unregister() {
    *TARGET.lock() = None;
}

/// A fixed buffer for formatting without allocating (the panic may come
/// from the allocator).
struct Buf {
    bytes: [u8; 768],
    len: usize,
}

impl Write for Buf {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        for &b in s.as_bytes() {
            if self.len < self.bytes.len() {
                self.bytes[self.len] = b;
                self.len += 1;
            }
        }
        Ok(())
    }
}

fn draw(canvas: &mut Canvas<'_>, message: &str, location: &str) {
    canvas.clear(Color::rgb(120, 16, 24));
    canvas.fill_rect(0, 0, 320, 22, Color::rgb(200, 30, 40));
    canvas.text(8, 4, "The program panicked", Color::WHITE, 2);
    let mut y = 30;
    canvas.text(8, y, location, Color::rgb(255, 210, 210), 1);
    y += 14;
    // Wrap the message at 51 columns.
    for line in message.split('\n') {
        let mut rest = line;
        while !rest.is_empty() && y < 210 {
            let take = rest.char_indices().nth(51).map_or(rest.len(), |(i, _)| i);
            canvas.text(8, y, &rest[..take], Color::WHITE, 1);
            rest = &rest[take..];
            y += 9;
        }
    }
    canvas.text(8, 224, "Press START to exit.", Color::rgb(255, 230, 120), 1);
}

fn hook(info: &PanicInfo<'_>) {
    let Some(target) = TARGET.try_lock().and_then(|t| *t) else { return };
    let mut message = Buf { bytes: [0; 768], len: 0 };
    let _ = write!(message, "{}", info.message());
    let mut location = Buf { bytes: [0; 768], len: 0 };
    if let Some(l) = info.location() {
        let _ = write!(location, "at {}:{}", l.file(), l.line());
    }
    let msg = core::str::from_utf8(&message.bytes[..message.len]).unwrap_or("(invalid UTF-8)");
    let loc = core::str::from_utf8(&location.bytes[..location.len]).unwrap_or("");
    let size = 240 * 320 * target.format.bytes_per_pixel();
    for &b in &target.buffers {
        let mut canvas = unsafe { Canvas::from_raw(b as *mut u8, 320, target.format) };
        draw(&mut canvas, msg, loc);
        let _ = gsp::flush_data_cache(b as *const u8, size);
    }
    // Wait for START (up to a minute), so the message can be read.
    let start = svc::get_system_tick();
    let limit = 60 * svc::TICKS_PER_SECOND;
    let mut was_held = Buttons(hid::read().buttons).contains(Buttons::START);
    while svc::get_system_tick() - start < limit {
        let held = Buttons(hid::read().buttons).contains(Buttons::START);
        if held && !was_held {
            break;
        }
        was_held = held;
        svc::sleep_thread(16_000_000);
    }
}
