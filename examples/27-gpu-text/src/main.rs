//! Text drawn by the GPU: the built-in 5x7 font is packed into a 128x64
//! alpha-only (A8) texture, and each character becomes a textured quad whose
//! colour comes from the vertex and whose alpha comes from the atlas. Both
//! screens use their own render target. START exits.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use alloc::vec::Vec;
use citronnade::font;
use citronnade::gpu::{
    self, shader, AlphaOperand, AttribType, Blend, Combine, Cull, Filter, Gpu, LinearBuffer, Primitive, RenderTarget, Shader, Source,
    TexEnv, TexFormat, TestFunc, Texture, VertexLayout, WriteMask,
};
use citronnade::math::{self, Mat4};
use citronnade::prelude::*;

const SHADER: &[u8] = shader!(
    r#"
    .fvec projection[4]
    .constf consts(0.0, 1.0, 0.0039215686, 0.0)
    .alias inpos v0
    .alias intc v1
    .alias inclr v2
    .out outpos position
    .out outtc texcoord0
    .out outclr color

    .proc main
        mov r0.xy, inpos
        mov r0.zw, consts.xxxy
        dp4 outpos.x, projection[0], r0
        dp4 outpos.y, projection[1], r0
        dp4 outpos.z, projection[2], r0
        dp4 outpos.w, projection[3], r0
        mov outtc, intc
        mul outclr, consts.zzzz, inclr
        end
    .end
    "#
);

#[derive(Clone, Copy, Default)]
#[repr(C)]
struct Vertex {
    pos: [f32; 2],
    uv: [f32; 2],
    color: [u8; 4],
}

const ATLAS_W: u32 = 128;
const ATLAS_H: u32 = 64;
const COLS: u32 = 16;

/// Characters 0x20-0x7F in 8x8 cells, 16 per row; 0x7F is the box glyph.
fn atlas() -> Vec<u8> {
    let mut img = alloc::vec![0u8; (ATLAS_W * ATLAS_H * 4) as usize];
    for code in 0x20u32..0x80 {
        let i = code - 0x20;
        let (cx, cy) = ((i % COLS) * 8, (i / COLS) * 8);
        let rows = font::glyph(char::from_u32(code).unwrap_or('?'));
        for (y, row) in rows.iter().enumerate() {
            for x in 0..5 {
                if row & (0x10 >> x) != 0 {
                    let p = (((cy + y as u32) * ATLAS_W + cx + x) * 4) as usize;
                    img[p..p + 4].copy_from_slice(&[255, 255, 255, 255]);
                }
            }
        }
    }
    img
}

/// Collects character quads for one draw call.
struct TextBatch {
    vertices: Vec<Vertex>,
}

impl TextBatch {
    fn new() -> TextBatch {
        TextBatch { vertices: Vec::new() }
    }

    /// Adds `s` at (x, y) (top left) with `scale`-sized pixels; `wave`
    /// offsets each character vertically. Returns the width.
    fn text(&mut self, x: f32, y: f32, s: &str, scale: f32, color: [u8; 4], wave: impl Fn(usize) -> f32) -> f32 {
        let mut pen = x;
        for (i, ch) in s.chars().enumerate() {
            let code = if (0x20..0x7F).contains(&(ch as u32)) { ch as u32 } else { 0x7F };
            let idx = code - 0x20;
            let (u0, v0) = ((idx % COLS) as f32 * 8.0 / ATLAS_W as f32, 1.0 - (idx / COLS) as f32 * 8.0 / ATLAS_H as f32);
            let (u1, v1) = (u0 + 8.0 / ATLAS_W as f32, v0 - 8.0 / ATLAS_H as f32);
            let (x0, y0) = (pen, y + wave(i));
            let (x1, y1) = (x0 + 8.0 * scale, y0 + 8.0 * scale);
            // Two triangles: (x0,y1) (x1,y1) (x0,y0) / (x0,y0) (x1,y1) (x1,y0)
            let q = [
                Vertex { pos: [x0, y1], uv: [u0, v1], color },
                Vertex { pos: [x1, y1], uv: [u1, v1], color },
                Vertex { pos: [x0, y0], uv: [u0, v0], color },
                Vertex { pos: [x0, y0], uv: [u0, v0], color },
                Vertex { pos: [x1, y1], uv: [u1, v1], color },
                Vertex { pos: [x1, y0], uv: [u1, v0], color },
            ];
            self.vertices.extend_from_slice(&q);
            pen += font::CELL_WIDTH as f32 * scale;
        }
        pen - x
    }

    /// Text with a dark drop shadow.
    fn shadowed(&mut self, x: f32, y: f32, s: &str, scale: f32, color: [u8; 4]) {
        self.text(x + scale, y + scale, s, scale, [0, 0, 0, 160], |_| 0.0);
        self.text(x, y, s, scale, color, |_| 0.0);
    }
}

#[citronnade::main]
fn main(sys: System) -> Result<()> {
    let System { apt, gsp, hid, .. } = sys;
    let mut screens = gsp.screens()?;
    let mut input = hid.input()?;
    let mut gpu = Gpu::new()?;
    let top = RenderTarget::top()?;
    let bottom = RenderTarget::bottom()?;
    let shader = Shader::new(SHADER).expect("shader");
    let projection = shader.uniform("projection").unwrap();
    let layout = VertexLayout::new().attrib(0, AttribType::Float, 2).attrib(1, AttribType::Float, 2).attrib(2, AttribType::UByte, 4);
    let mut atlas = Texture::from_rgba(ATLAS_W, ATLAS_H, &atlas(), TexFormat::A8)?;
    atlas.set_filter(Filter::Nearest, Filter::Nearest);
    log!("gpu-text: atlas {}x{} A8 ({} bytes)", ATLAS_W, ATLAS_H, ATLAS_W * ATLAS_H);

    // Colour from the vertex, alpha = atlas alpha x vertex alpha.
    let text_env = TexEnv::PASS
        .rgb_sources(Source::PrimaryColor, Source::PrimaryColor, Source::PrimaryColor)
        .alpha_sources(Source::Texture0, Source::PrimaryColor, Source::PrimaryColor)
        .alpha_operands(AlphaOperand::Alpha, AlphaOperand::Alpha, AlphaOperand::Alpha)
        .alpha_func(Combine::Modulate);

    let mut top_buf = LinearBuffer::<Vertex>::zeroed(6 * 2048)?;
    let mut bottom_buf = LinearBuffer::<Vertex>::zeroed(6 * 2048)?;
    let mut frame = 0u32;
    while apt.main_loop() {
        if input.scan().down().contains(Buttons::START) {
            break;
        }
        let t = frame as f32 / 60.0;

        // Top screen.
        let mut top_text = TextBatch::new();
        top_text.shadowed(12.0, 10.0, "GPU text", 4.0, [255, 220, 90, 255]);
        top_text.text(12.0, 60.0, "drawn from a font atlas, one quad per character", 1.0, [210, 220, 240, 255], |_| 0.0);
        top_text.text(12.0, 90.0, "wavy text", 3.0, [120, 230, 255, 255], |i| math::sin(t * 4.0 + i as f32 * 0.6) * 6.0);
        let marquee = "  The quick brown fox jumps over the lazy dog.  0123456789  ";
        let offset = (t * 60.0) % (marquee.len() as f32 * 12.0);
        top_text.text(400.0 - offset, 140.0, marquee, 2.0, [255, 255, 255, 255], |_| 0.0);
        for (i, size) in [1.0f32, 2.0, 3.0].iter().enumerate() {
            let hue = Color::hsv(((frame * 2 + i as u32 * 120) % 360) as u16, 160, 255);
            top_text.text(12.0 + i as f32 * 110.0, 180.0, &format!("{}x", *size as u32), *size, [hue.r, hue.g, hue.b, 255], |_| 0.0);
        }
        top_text.text(300.0, 200.0, "|", 4.0, [255, 255, 255, 255], |_| 0.0);

        // Bottom screen.
        let mut bottom_text = TextBatch::new();
        bottom_text.shadowed(10.0, 10.0, "Bottom screen", 2.0, [140, 255, 160, 255]);
        for line in 0..10 {
            let s = format!("line {:2}: frame {:5}  {:.2}s", line, frame, t);
            let a = (255.0 * (1.0 - line as f32 / 12.0)) as u8;
            bottom_text.text(10.0, 40.0 + line as f32 * 14.0, &s, 1.0, [220, 230, 255, a], |_| 0.0);
        }
        bottom_text.text(10.0, 220.0, "START: exit", 1.0, [255, 255, 255, 255], |_| 0.0);

        for (target, screen, w, batch, buf) in [
            (&top, Screen::Top, 400.0, &top_text, &mut top_buf),
            (&bottom, Screen::Bottom, 320.0, &bottom_text, &mut bottom_buf),
        ] {
            let n = batch.vertices.len().min(buf.len());
            buf[..n].copy_from_slice(&batch.vertices[..n]);
            gpu.clear(target, gpu::rgba(30, 34, 52, 255), 0)?;
            gpu.bind_target(target);
            gpu.bind_shader(&shader);
            gpu.set_layout(&layout);
            gpu.bind_vertices(buf);
            gpu.bind_texture(0, Some(&atlas));
            gpu.set_texenv(0, text_env);
            gpu.set_depth_test(false, TestFunc::Always, WriteMask::COLOR);
            gpu.set_cull(Cull::None);
            gpu.set_blend(Some(Blend::ALPHA));
            gpu.set_mat4(projection, &Mat4::screen_2d(w, 240.0));
            gpu.draw_arrays(Primitive::Triangles, 0, n as u32);
            screens.show_target(&mut gpu, target, screen)?;
        }

        if frame == 30 {
            // '|' at (300, 200), scale 4: the glyph's column 2 is lit, column 0 is not.
            let c = screens.top();
            let lit = c.get(300 + 2 * 4 + 1, 200 + 3 * 4);
            let unlit = c.get(300 + 1, 200 + 3 * 4);
            let ok = lit.r > 240 && lit.g > 240 && unlit.r < 60;
            log!("glyph pixels {} (lit {:?}, unlit {:?})", if ok { "ok" } else { "wrong" }, (lit.r, lit.g, lit.b), (unlit.r, unlit.g, unlit.b));
        }
        screens.present();
        frame += 1;
    }
    log!("bye: {} frames", frame);
    Ok(())
}
