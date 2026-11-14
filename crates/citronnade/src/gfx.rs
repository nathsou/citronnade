//! The screens: framebuffers in linear memory, presented by GSP at VBlank.
//!
//! The top screen is 400x240 (800x240 in wide mode, or a pair of 400x240
//! images in stereo mode); the bottom screen is 320x240. Framebuffers are
//! stored rotated (each column of the screen is a 240-pixel row in memory,
//! starting from the bottom); [`Canvas`] hides that.

use alloc::vec::Vec;
use core::alloc::Layout;
use core::marker::PhantomData;
use core::ptr::NonNull;
use core::sync::atomic::{AtomicBool, Ordering};

use citronnade_rt::heap;
use citronnade_services::gsp;

use crate::font;
use crate::Result;

/// A colour with alpha (0 transparent, 255 opaque).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Color {
        Color { r, g, b, a: 255 }
    }
    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Color {
        Color { r, g, b, a }
    }
    /// From `0xRRGGBB`.
    pub const fn hex(v: u32) -> Color {
        Color::rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
    }
    pub const fn with_alpha(self, a: u8) -> Color {
        Color { a, ..self }
    }
    /// Linear interpolation, `t` from 0 (self) to 255 (other).
    pub fn lerp(self, other: Color, t: u8) -> Color {
        let mix = |a: u8, b: u8| ((a as u32 * (255 - t as u32) + b as u32 * t as u32 + 127) / 255) as u8;
        Color { r: mix(self.r, other.r), g: mix(self.g, other.g), b: mix(self.b, other.b), a: mix(self.a, other.a) }
    }
    /// From hue (0-359), saturation and value (0-255).
    pub fn hsv(h: u16, s: u8, v: u8) -> Color {
        let h = (h % 360) as u32;
        let (s, v) = (s as u32, v as u32);
        let c = v * s / 255;
        let x = c * (60 - (h % 120).abs_diff(60)) / 60;
        let m = v - c;
        let (r, g, b) = match h / 60 {
            0 => (c, x, 0),
            1 => (x, c, 0),
            2 => (0, c, x),
            3 => (0, x, c),
            4 => (x, 0, c),
            _ => (c, 0, x),
        };
        Color::rgb((r + m) as u8, (g + m) as u8, (b + m) as u8)
    }

    pub const BLACK: Color = Color::rgb(0, 0, 0);
    pub const WHITE: Color = Color::rgb(255, 255, 255);
    pub const RED: Color = Color::rgb(230, 40, 40);
    pub const GREEN: Color = Color::rgb(40, 200, 70);
    pub const BLUE: Color = Color::rgb(40, 90, 230);
    pub const YELLOW: Color = Color::rgb(250, 215, 40);
    pub const CYAN: Color = Color::rgb(40, 210, 220);
    pub const MAGENTA: Color = Color::rgb(210, 60, 200);
    pub const GRAY: Color = Color::rgb(128, 128, 128);
    pub const DARK_GRAY: Color = Color::rgb(48, 48, 52);
    pub const TRANSPARENT: Color = Color::rgba(0, 0, 0, 0);
}

/// Framebuffer pixel formats.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum Format {
    Rgba8 = 0,
    Bgr8 = 1,
    Rgb565 = 2,
    Rgb5A1 = 3,
    Rgba4 = 4,
}

impl Format {
    pub const ALL: [Format; 5] = [Format::Rgba8, Format::Bgr8, Format::Rgb565, Format::Rgb5A1, Format::Rgba4];

    pub const fn bytes_per_pixel(self) -> usize {
        match self {
            Format::Rgba8 => 4,
            Format::Bgr8 => 3,
            _ => 2,
        }
    }

    /// The bytes of a colour in this format.
    pub fn encode(self, c: Color) -> [u8; 4] {
        let (r, g, b, a) = (c.r as u16, c.g as u16, c.b as u16, c.a as u16);
        match self {
            Format::Rgba8 => [c.a, c.b, c.g, c.r],
            Format::Bgr8 => [c.b, c.g, c.r, 0],
            Format::Rgb565 => ((r >> 3) << 11 | (g >> 2) << 5 | (b >> 3)).to_le_bytes().pad(),
            Format::Rgb5A1 => ((r >> 3) << 11 | (g >> 3) << 6 | (b >> 3) << 1 | (a >> 7)).to_le_bytes().pad(),
            Format::Rgba4 => ((r >> 4) << 12 | (g >> 4) << 8 | (b >> 4) << 4 | (a >> 4)).to_le_bytes().pad(),
        }
    }

    /// The colour of pixel bytes in this format.
    pub fn decode(self, p: &[u8]) -> Color {
        let x5 = |v: u16| ((v << 3) | (v >> 2)) as u8;
        let x6 = |v: u16| ((v << 2) | (v >> 4)) as u8;
        let x4 = |v: u16| (v * 17) as u8;
        match self {
            Format::Rgba8 => Color::rgba(p[3], p[2], p[1], p[0]),
            Format::Bgr8 => Color::rgb(p[2], p[1], p[0]),
            Format::Rgb565 => {
                let v = u16::from_le_bytes([p[0], p[1]]);
                Color::rgb(x5(v >> 11), x6((v >> 5) & 63), x5(v & 31))
            }
            Format::Rgb5A1 => {
                let v = u16::from_le_bytes([p[0], p[1]]);
                Color::rgba(x5(v >> 11), x5((v >> 6) & 31), x5((v >> 1) & 31), if v & 1 != 0 { 255 } else { 0 })
            }
            Format::Rgba4 => {
                let v = u16::from_le_bytes([p[0], p[1]]);
                Color::rgba(x4(v >> 12), x4((v >> 8) & 15), x4((v >> 4) & 15), x4(v & 15))
            }
        }
    }
}

/// Blends `c` over `count` consecutive pixels.
///
/// # Safety
/// `dst` must be valid for `count` pixels of `format`.
#[inline]
unsafe fn blend_pixels(dst: *mut u8, count: usize, c: Color, format: Format) {
    let a = c.a as u32 + (c.a >> 7) as u32; // 0..=256
    let inv = 256 - a;
    let (sr, sg, sb) = (c.r as u32 * a, c.g as u32 * a, c.b as u32 * a);
    let mix = |src: u32, dst: u8| ((src + dst as u32 * inv) >> 8) as u8;
    unsafe {
        match format {
            Format::Bgr8 => {
                for i in 0..count {
                    let p = dst.add(3 * i);
                    *p = mix(sb, *p);
                    *p.add(1) = mix(sg, *p.add(1));
                    *p.add(2) = mix(sr, *p.add(2));
                }
            }
            Format::Rgba8 => {
                for i in 0..count {
                    let p = dst.add(4 * i);
                    *p.add(1) = mix(sb, *p.add(1));
                    *p.add(2) = mix(sg, *p.add(2));
                    *p.add(3) = mix(sr, *p.add(3));
                    *p = 255;
                }
            }
            _ => {
                for i in 0..count {
                    let p = dst.add(2 * i);
                    let under = format.decode(core::slice::from_raw_parts(p, 2));
                    let out = Color::rgb(mix(sr, under.r), mix(sg, under.g), mix(sb, under.b));
                    let e = format.encode(out);
                    *p = e[0];
                    *p.add(1) = e[1];
                }
            }
        }
    }
}

trait Pad {
    fn pad(self) -> [u8; 4];
}

impl Pad for [u8; 2] {
    fn pad(self) -> [u8; 4] {
        [self[0], self[1], 0, 0]
    }
}

/// Fills `count` pixels of `bpp` bytes at `dst` with `px`, using word stores.
///
/// # Safety
/// `dst` must be valid for `count * bpp` bytes.
#[inline]
unsafe fn fill_pixels(dst: *mut u8, count: usize, px: [u8; 4], bpp: usize) {
    unsafe {
        match bpp {
            4 => {
                let w = u32::from_le_bytes(px);
                if dst as usize % 4 == 0 {
                    let d = dst as *mut u32;
                    for i in 0..count {
                        d.add(i).write(w);
                    }
                } else {
                    for i in 0..count {
                        (dst.add(4 * i) as *mut u32).write_unaligned(w);
                    }
                }
            }
            2 => {
                let mut i = 0;
                let mut p = dst;
                if p as usize % 4 != 0 && count > 0 {
                    p.copy_from_nonoverlapping(px.as_ptr(), 2);
                    p = p.add(2);
                    i = 1;
                }
                let w = u32::from_le_bytes([px[0], px[1], px[0], px[1]]);
                while i + 2 <= count {
                    (p as *mut u32).write(w);
                    p = p.add(4);
                    i += 2;
                }
                if i < count {
                    p.copy_from_nonoverlapping(px.as_ptr(), 2);
                }
            }
            _ => {
                // 3 bytes: four pixels make three words.
                let mut i = 0;
                let mut p = dst;
                while i < count && p as usize % 4 != 0 {
                    p.copy_from_nonoverlapping(px.as_ptr(), 3);
                    p = p.add(3);
                    i += 1;
                }
                let (b, g, r) = (px[0], px[1], px[2]);
                let w0 = u32::from_le_bytes([b, g, r, b]);
                let w1 = u32::from_le_bytes([g, r, b, g]);
                let w2 = u32::from_le_bytes([r, b, g, r]);
                while i + 4 <= count {
                    let q = p as *mut u32;
                    q.write(w0);
                    q.add(1).write(w1);
                    q.add(2).write(w2);
                    p = p.add(12);
                    i += 4;
                }
                while i < count {
                    p.copy_from_nonoverlapping(px.as_ptr(), 3);
                    p = p.add(3);
                    i += 1;
                }
            }
        }
    }
}

/// A view of a framebuffer, in screen coordinates (x right, y down).
pub struct Canvas<'a> {
    ptr: *mut u8,
    width: u32,
    height: u32,
    format: Format,
    _marker: PhantomData<&'a mut [u8]>,
}

impl<'a> Canvas<'a> {
    /// # Safety
    /// `ptr` must point to a `width` x 240 framebuffer in `format`, rotated as
    /// the 3DS stores them, valid and exclusively borrowed for `'a`.
    pub unsafe fn from_raw(ptr: *mut u8, width: u32, format: Format) -> Canvas<'a> {
        Canvas { ptr, width, height: 240, format, _marker: PhantomData }
    }

    pub fn width(&self) -> u32 {
        self.width
    }
    pub fn height(&self) -> u32 {
        self.height
    }
    pub fn format(&self) -> Format {
        self.format
    }

    #[inline(always)]
    fn offset(&self, x: u32, y: u32) -> usize {
        (x as usize * 240 + (239 - y as usize)) * self.format.bytes_per_pixel()
    }

    #[inline(always)]
    fn put(&mut self, x: u32, y: u32, bytes: &[u8; 4]) {
        let bpp = self.format.bytes_per_pixel();
        unsafe { core::ptr::copy_nonoverlapping(bytes.as_ptr(), self.ptr.add(self.offset(x, y)), bpp) };
    }

    /// Sets a pixel (ignored outside the canvas).
    pub fn set(&mut self, x: i32, y: i32, c: Color) {
        if x >= 0 && y >= 0 && (x as u32) < self.width && (y as u32) < self.height {
            if c.a == 255 {
                let bytes = self.format.encode(c);
                self.put(x as u32, y as u32, &bytes);
            } else if c.a > 0 {
                let off = self.offset(x as u32, y as u32);
                unsafe { blend_pixels(self.ptr.add(off), 1, c, self.format) };
            }
        }
    }

    /// Reads a pixel (black outside the canvas).
    pub fn get(&self, x: i32, y: i32) -> Color {
        if x < 0 || y < 0 || x as u32 >= self.width || y as u32 >= self.height {
            return Color::BLACK;
        }
        let bpp = self.format.bytes_per_pixel();
        let p = unsafe { core::slice::from_raw_parts(self.ptr.add(self.offset(x as u32, y as u32)), bpp) };
        self.format.decode(p)
    }

    /// Fills the whole canvas.
    pub fn clear(&mut self, c: Color) {
        let bytes = self.format.encode(c.with_alpha(255));
        let n = self.width as usize * 240;
        unsafe { fill_pixels(self.ptr, n, bytes, self.format.bytes_per_pixel()) };
    }

    /// Fills a rectangle (clipped; blended when `c` is translucent).
    pub fn fill_rect(&mut self, x: i32, y: i32, w: u32, h: u32, c: Color) {
        let x0 = x.max(0) as u32;
        let y0 = y.max(0) as u32;
        let x1 = (x + w as i32).clamp(0, self.width as i32) as u32;
        let y1 = (y + h as i32).clamp(0, self.height as i32) as u32;
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        if c.a == 0 {
            return;
        }
        let bpp = self.format.bytes_per_pixel();
        let len = (y1 - y0) as usize;
        if c.a < 255 {
            for px in x0..x1 {
                let start = self.offset(px, y1 - 1);
                unsafe { blend_pixels(self.ptr.add(start), len, c, self.format) };
            }
            return;
        }
        let bytes = self.format.encode(c);
        for px in x0..x1 {
            // A column of the screen is contiguous in memory.
            let start = self.offset(px, y1 - 1);
            unsafe { fill_pixels(self.ptr.add(start), len, bytes, bpp) };
        }
    }

    /// Fills a rectangle with a vertical gradient from `top` to `bottom`.
    /// Fast: one column is drawn, then copied (columns are contiguous).
    pub fn vertical_gradient(&mut self, x: i32, y: i32, w: u32, h: u32, top: Color, bottom: Color) {
        let x0 = x.max(0) as u32;
        let y0 = y.max(0) as u32;
        let x1 = (x + w as i32).clamp(0, self.width as i32) as u32;
        let y1 = (y + h as i32).clamp(0, self.height as i32) as u32;
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        let bpp = self.format.bytes_per_pixel();
        for py in y0..y1 {
            let t = ((py as i32 - y) * 255 / (h.max(2) as i32 - 1)).clamp(0, 255) as u8;
            let bytes = self.format.encode(top.lerp(bottom, t).with_alpha(255));
            self.put(x0, py, &bytes);
        }
        let start = self.offset(x0, y1 - 1);
        let len = (y1 - y0) as usize * bpp;
        for px in x0 + 1..x1 {
            let dst = self.offset(px, y1 - 1);
            unsafe { core::ptr::copy_nonoverlapping(self.ptr.add(start), self.ptr.add(dst), len) };
        }
    }

    /// Draws a rectangle's outline.
    pub fn rect(&mut self, x: i32, y: i32, w: u32, h: u32, c: Color) {
        if w == 0 || h == 0 {
            return;
        }
        self.fill_rect(x, y, w, 1, c);
        self.fill_rect(x, y + h as i32 - 1, w, 1, c);
        self.fill_rect(x, y, 1, h, c);
        self.fill_rect(x + w as i32 - 1, y, 1, h, c);
    }

    /// Draws a line (Bresenham).
    pub fn line(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, c: Color) {
        let (dx, dy) = ((x1 - x0).abs(), -(y1 - y0).abs());
        let (sx, sy) = (if x0 < x1 { 1 } else { -1 }, if y0 < y1 { 1 } else { -1 });
        let (mut x, mut y, mut err) = (x0, y0, dx + dy);
        loop {
            self.set(x, y, c);
            if x == x1 && y == y1 {
                break;
            }
            let e2 = 2 * err;
            if e2 >= dy {
                err += dy;
                x += sx;
            }
            if e2 <= dx {
                err += dx;
                y += sy;
            }
        }
    }

    /// Fills a circle.
    pub fn fill_circle(&mut self, cx: i32, cy: i32, r: i32, c: Color) {
        for dy in -r..=r {
            let half = isqrt((r * r - dy * dy) as u32) as i32;
            if c.a == 255 {
                self.fill_rect(cx - half, cy + dy, (2 * half + 1) as u32, 1, c);
            } else {
                for dx in -half..=half {
                    self.set(cx + dx, cy + dy, c);
                }
            }
        }
    }

    /// Draws a circle's outline.
    pub fn circle(&mut self, cx: i32, cy: i32, r: i32, c: Color) {
        let (mut x, mut y, mut err) = (r, 0, 1 - r);
        while x >= y {
            for (px, py) in [(x, y), (y, x), (-y, x), (-x, y), (-x, -y), (-y, -x), (y, -x), (x, -y)] {
                self.set(cx + px, cy + py, c);
            }
            y += 1;
            if err < 0 {
                err += 2 * y + 1;
            } else {
                x -= 1;
                err += 2 * (y - x) + 1;
            }
        }
    }

    /// Copies a surface onto the canvas (a memory copy when the width and
    /// format match).
    pub fn draw_surface(&mut self, surface: &Surface) {
        if surface.width == self.width && surface.format == self.format {
            unsafe { core::ptr::copy_nonoverlapping(surface.data.as_ptr(), self.ptr, surface.data.len()) };
            return;
        }
        let bpp = surface.format.bytes_per_pixel();
        for x in 0..surface.width.min(self.width) {
            for y in 0..240 {
                let o = (x as usize * 240 + (239 - y as usize)) * bpp;
                let c = surface.format.decode(&surface.data[o..o + bpp]);
                self.set(x as i32, y as i32, c.with_alpha(255));
            }
        }
    }

    /// Draws a thick line (a disc stamped along it).
    pub fn thick_line(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, radius: i32, c: Color) {
        let (dx, dy) = ((x1 - x0).abs(), -(y1 - y0).abs());
        let (sx, sy) = (if x0 < x1 { 1 } else { -1 }, if y0 < y1 { 1 } else { -1 });
        let (mut x, mut y, mut err) = (x0, y0, dx + dy);
        loop {
            self.fill_circle(x, y, radius, c);
            if x == x1 && y == y1 {
                break;
            }
            let e2 = 2 * err;
            if e2 >= dy {
                err += dy;
                x += sx;
            }
            if e2 <= dx {
                err += dx;
                y += sy;
            }
        }
    }

    /// Draws an RGBA8 image (row-major) with alpha blending.
    pub fn blit(&mut self, x: i32, y: i32, w: u32, h: u32, rgba: &[u8]) {
        for iy in 0..h {
            for ix in 0..w {
                let p = &rgba[4 * (iy * w + ix) as usize..][..4];
                self.set(x + ix as i32, y + iy as i32, Color::rgba(p[0], p[1], p[2], p[3]));
            }
        }
    }

    /// Draws text with the built-in font, `scale` times larger. Returns the
    /// x after the last character. `\n` starts a new line.
    pub fn text(&mut self, x: i32, y: i32, s: &str, c: Color, scale: u32) -> i32 {
        let scale = scale.max(1) as i32;
        let (mut cx, mut cy) = (x, y);
        for ch in s.chars() {
            if ch == '\n' {
                cx = x;
                cy += font::CELL_HEIGHT as i32 * scale;
                continue;
            }
            self.glyph(cx, cy, ch, c, scale);
            cx += font::CELL_WIDTH as i32 * scale;
        }
        cx
    }

    /// Draws one character.
    pub fn glyph(&mut self, x: i32, y: i32, ch: char, c: Color, scale: i32) {
        let rows = font::glyph(ch);
        for (ry, bits) in rows.iter().enumerate() {
            for rx in 0..5 {
                if bits & (0x10 >> rx) != 0 {
                    if scale == 1 {
                        self.set(x + rx, y + ry as i32, c);
                    } else {
                        self.fill_rect(x + rx * scale, y + ry as i32 * scale, scale as u32, scale as u32, c);
                    }
                }
            }
        }
    }

    /// The size of `s` drawn with [`Canvas::text`].
    pub fn text_size(s: &str, scale: u32) -> (u32, u32) {
        let mut lines = 0;
        let mut widest = 0;
        for line in s.split('\n') {
            lines += 1;
            widest = widest.max(line.chars().count() as u32);
        }
        (widest * font::CELL_WIDTH * scale, lines * font::CELL_HEIGHT * scale)
    }
}

fn isqrt(n: u32) -> u32 {
    let mut x = n;
    let mut y = x.div_ceil(2);
    if n < 2 {
        return n;
    }
    while y < x {
        x = y;
        y = (x + n / x) / 2;
    }
    x
}

/// The 3D slider's position, 0.0 (off) to 1.0.
pub fn slider_3d() -> f32 {
    f32::from_bits(citronnade_rt::memory::read_config_u32(citronnade_rt::memory::SHARED_CONFIG + 0x80)).clamp(0.0, 1.0)
}

/// An off-screen image in framebuffer layout, drawn with a [`Canvas`] and
/// copied to a screen quickly with [`Canvas::draw_surface`].
pub struct Surface {
    width: u32,
    format: Format,
    data: Vec<u8>,
}

impl Surface {
    /// A surface as wide as a screen (400 or 320) and 240 tall.
    pub fn new(width: u32, format: Format) -> Surface {
        Surface { width, format, data: alloc::vec![0; width as usize * 240 * format.bytes_per_pixel()] }
    }

    pub fn canvas(&mut self) -> Canvas<'_> {
        unsafe { Canvas::from_raw(self.data.as_mut_ptr(), self.width, self.format) }
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn format(&self) -> Format {
        self.format
    }
}

/// The top screen's layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TopMode {
    /// One 400x240 image.
    Mono,
    /// Two 400x240 images, one per eye (shown in 3D when the slider is up).
    Stereo,
    /// One 800x240 image, with pixels half as wide (not on Old 2DS).
    Wide,
}

/// Which screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Screen {
    Top,
    Bottom,
}

const TOP_BYTES_MAX: usize = 240 * 800 * 4;
const BOTTOM_BYTES_MAX: usize = 240 * 320 * 4;

struct Buffers {
    ptrs: [NonNull<u8>; 2],
    size: usize,
    format: Format,
    shown: usize,
}

impl Buffers {
    fn new(size: usize, format: Format) -> Result<Buffers> {
        let layout = Layout::from_size_align(size, 0x80).unwrap();
        let alloc = || heap::linear_alloc(layout).ok_or(crate::ResultCode(0xD86007F3));
        let a = alloc()?;
        let b = alloc()?;
        for p in [a, b] {
            unsafe { p.as_ptr().write_bytes(0, size) };
        }
        Ok(Buffers { ptrs: [a, b], size, format, shown: 0 })
    }

    fn back(&self) -> NonNull<u8> {
        self.ptrs[self.shown ^ 1]
    }
}

impl Drop for Buffers {
    fn drop(&mut self) {
        let layout = Layout::from_size_align(self.size, 0x80).unwrap();
        for p in self.ptrs {
            unsafe { heap::linear_free(p, layout) };
        }
    }
}

static SCREENS_ALIVE: AtomicBool = AtomicBool::new(false);

/// Both screens, double buffered. Draw into the back buffers through
/// [`Screens::top`] and [`Screens::bottom`], then [`Screens::present`].
pub struct Screens {
    top: Buffers,
    bottom: Buffers,
    top_mode: TopMode,
    frames: u64,
}

impl Screens {
    pub(crate) fn new(top: Format, bottom: Format) -> Result<Screens> {
        gsp::init()?;
        let screens = (|| -> Result<Screens> {
            Ok(Screens {
                top: Buffers::new(TOP_BYTES_MAX, top)?,
                bottom: Buffers::new(BOTTOM_BYTES_MAX, bottom)?,
                top_mode: TopMode::Mono,
                frames: 0,
            })
        })();
        let mut screens = match screens {
            Ok(s) => s,
            Err(e) => {
                gsp::exit();
                return Err(e);
            }
        };
        SCREENS_ALIVE.store(true, Ordering::Release);
        screens.show(0);
        gsp::wait_for_vblank();
        let _ = gsp::set_lcd_force_black(false);
        crate::panic_screen::register(&screens);
        Ok(screens)
    }

    /// Changes the top screen's layout.
    pub fn set_top_mode(&mut self, mode: TopMode) {
        self.top_mode = mode;
    }

    pub fn top_mode(&self) -> TopMode {
        self.top_mode
    }

    /// Changes a screen's pixel format (takes effect for the next frame).
    pub fn set_format(&mut self, screen: Screen, format: Format) {
        match screen {
            Screen::Top => self.top.format = format,
            Screen::Bottom => self.bottom.format = format,
        }
    }

    pub fn format(&self, screen: Screen) -> Format {
        match screen {
            Screen::Top => self.top.format,
            Screen::Bottom => self.bottom.format,
        }
    }

    fn top_width(&self) -> u32 {
        if self.top_mode == TopMode::Wide { 800 } else { 400 }
    }

    /// The top screen's back buffer (the left eye in stereo mode).
    pub fn top(&mut self) -> Canvas<'_> {
        let w = self.top_width();
        unsafe { Canvas::from_raw(self.top.back().as_ptr(), w, self.top.format) }
    }

    /// The right eye's back buffer (stereo mode; the same as `top` otherwise).
    pub fn top_right(&mut self) -> Canvas<'_> {
        let ptr = self.right_eye(self.top.back().as_ptr());
        unsafe { Canvas::from_raw(ptr, 400, self.top.format) }
    }

    /// Both eyes of the top screen.
    pub fn top_stereo(&mut self) -> (Canvas<'_>, Canvas<'_>) {
        let left = self.top.back().as_ptr();
        let right = self.right_eye(left);
        let f = self.top.format;
        unsafe { (Canvas::from_raw(left, 400, f), Canvas::from_raw(right, 400, f)) }
    }

    fn right_eye(&self, left: *mut u8) -> *mut u8 {
        if self.top_mode == TopMode::Stereo {
            unsafe { left.add(240 * 400 * self.top.format.bytes_per_pixel()) }
        } else {
            left
        }
    }

    /// The bottom screen's back buffer.
    pub fn bottom(&mut self) -> Canvas<'_> {
        unsafe { Canvas::from_raw(self.bottom.back().as_ptr(), 320, self.bottom.format) }
    }

    /// A canvas for either screen.
    pub fn canvas(&mut self, screen: Screen) -> Canvas<'_> {
        match screen {
            Screen::Top => self.top(),
            Screen::Bottom => self.bottom(),
        }
    }

    /// Frames presented so far.
    pub fn frame_count(&self) -> u64 {
        self.frames
    }

    fn show(&mut self, index: usize) {
        let mode_top = self.top.format as u32
            | match self.top_mode {
                TopMode::Mono => 1 << 6,
                TopMode::Stereo => 1 << 5,
                TopMode::Wide => 0,
            }
            | 1 << 8;
        let left = self.top.ptrs[index].as_ptr();
        let right = self.right_eye(left);
        gsp::present(0, index as u32, left as u32, right as u32, 240 * self.top.format.bytes_per_pixel() as u32, mode_top);
        let bottom = self.bottom.ptrs[index].as_ptr() as u32;
        let mode_bottom = self.bottom.format as u32 | 1 << 8;
        gsp::present(1, index as u32, bottom, bottom, 240 * self.bottom.format.bytes_per_pixel() as u32, mode_bottom);
    }

    /// Shows what was drawn into the back buffers at the next VBlank, waits
    /// for it, and makes the other buffers the back buffers.
    pub fn present(&mut self) {
        crate::console::render_into(self);
        let back = self.top.shown ^ 1;
        let top_bytes = 240 * self.top_width() as usize * self.top.format.bytes_per_pixel()
            * if self.top_mode == TopMode::Stereo { 2 } else { 1 };
        let _ = gsp::flush_data_cache(self.top.ptrs[back].as_ptr(), top_bytes);
        let _ = gsp::flush_data_cache(self.bottom.ptrs[back].as_ptr(), 240 * 320 * self.bottom.format.bytes_per_pixel());
        self.show(back);
        self.top.shown = back;
        self.bottom.shown = back;
        self.frames += 1;
        gsp::wait_for_vblank();
        crate::panic_screen::update(self);
    }

    /// Waits for the next top-screen VBlank without presenting.
    pub fn wait_vblank(&self) {
        gsp::wait_for_vblank();
    }

    /// The bottom screen's framebuffers, for the panic screen.
    pub(crate) fn bottom_buffers(&self) -> ([usize; 2], Format, usize) {
        ([self.bottom.ptrs[0].as_ptr() as usize, self.bottom.ptrs[1].as_ptr() as usize], self.bottom.format, self.bottom.shown)
    }
}

impl Drop for Screens {
    fn drop(&mut self) {
        crate::panic_screen::unregister();
        if gsp::has_right() {
            gsp::wait_for_vblank();
            let _ = gsp::set_lcd_force_black(true);
        }
        SCREENS_ALIVE.store(false, Ordering::Release);
        gsp::exit();
    }
}

/// Draws into an off-screen RGBA8 image, for building sprites.
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl Image {
    pub fn new(width: u32, height: u32) -> Image {
        Image { width, height, rgba: alloc::vec![0; (width * height * 4) as usize] }
    }

    pub fn set(&mut self, x: u32, y: u32, c: Color) {
        if x < self.width && y < self.height {
            let i = 4 * (y * self.width + x) as usize;
            self.rgba[i..i + 4].copy_from_slice(&[c.r, c.g, c.b, c.a]);
        }
    }
}
