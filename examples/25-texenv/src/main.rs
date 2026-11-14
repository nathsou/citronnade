//! The PICA200 has no fragment shaders: each pixel's colour comes from up to
//! six texture combiner stages, each computing a function of three inputs
//! (vertex colour, textures, a constant, the previous stage). Six panels on
//! the top screen:
//!
//! 1. texture x vertex colour (modulate);
//! 2. two textures (a base and a detail texture tiled four times);
//! 3. a cross-fade between two textures by a constant's alpha;
//! 4. two chained stages: modulate, then add a pulsing constant;
//! 5. dot3 bump mapping: a normal map dotted with a moving light direction;
//! 6. the alpha test dissolving a texture by a rising threshold.
//!
//! START exits.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec::Vec;
use citronnade::gpu::{
    self, shader, AlphaOperand, AttribType, Blend, Combine, Cull, Gpu, LinearBuffer, Primitive, RenderTarget, RgbOperand, Scale,
    Shader, Source, TexEnv, TexFormat, TestFunc, Texture, VertexLayout, Wrap, WriteMask,
};
use citronnade::math::{self, Mat4};
use citronnade::prelude::*;

const SHADER: &[u8] = shader!(
    r#"
    .fvec projection[4]
    .fvec detail            ; texcoord1 = texcoord0 * detail
    .constf ones(1.0, 1.0, 1.0, 1.0)
    .alias inpos v0
    .alias inclr v1
    .alias intc v2
    .out outpos position
    .out outclr color
    .out outtc0 texcoord0
    .out outtc1 texcoord1

    .proc main
        mov r0.xyz, inpos
        mov r0.w, ones
        dp4 outpos.x, projection[0], r0
        dp4 outpos.y, projection[1], r0
        dp4 outpos.z, projection[2], r0
        dp4 outpos.w, projection[3], r0
        mov outclr, inclr
        mov outtc0, intc
        mul outtc1, detail, intc
        end
    .end
    "#
);

#[derive(Clone, Copy)]
#[repr(C)]
struct Vertex {
    pos: [f32; 3],
    color: [f32; 4],
    uv: [f32; 2],
}

const PANEL_W: f32 = 120.0;
const PANEL_H: f32 = 92.0;

fn panel_origin(i: usize) -> (f32, f32) {
    (10.0 + (i % 3) as f32 * 130.0, 12.0 + (i / 3) as f32 * 116.0)
}

/// Deterministic value noise in 0..1 with `cell`-pixel features.
fn noise(x: u32, y: u32, cell: u32, seed: u32) -> f32 {
    let hash = |x: u32, y: u32| {
        let mut h = x.wrapping_mul(0x9E37_79B1) ^ y.wrapping_mul(0x85EB_CA77) ^ seed.wrapping_mul(0xC2B2_AE3D);
        h ^= h >> 15;
        h = h.wrapping_mul(0x2C1B_3C6D);
        h ^= h >> 12;
        (h & 0xFFFF) as f32 / 65535.0
    };
    let (cx, cy) = (x / cell, y / cell);
    let (fx, fy) = ((x % cell) as f32 / cell as f32, (y % cell) as f32 / cell as f32);
    let s = |t: f32| t * t * (3.0 - 2.0 * t);
    let (sx, sy) = (s(fx), s(fy));
    let top = math::lerp(hash(cx, cy), hash(cx + 1, cy), sx);
    let bottom = math::lerp(hash(cx, cy + 1), hash(cx + 1, cy + 1), sx);
    math::lerp(top, bottom, sy)
}

fn image(w: u32, h: u32, f: impl Fn(u32, u32) -> [u8; 4]) -> Vec<u8> {
    let mut v = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            v.extend_from_slice(&f(x, y));
        }
    }
    v
}

#[citronnade::main]
fn main(sys: System) -> Result<()> {
    let System { apt, gsp, hid, .. } = sys;
    let mut screens = gsp.screens()?;
    let mut input = hid.input()?;
    let mut gpu = Gpu::new()?;
    let top = RenderTarget::top()?;
    let shader = Shader::new(SHADER).expect("shader");
    let projection = shader.uniform("projection").unwrap();
    let detail = shader.uniform("detail").unwrap();
    let layout = VertexLayout::new().attrib(0, AttribType::Float, 3).attrib(1, AttribType::Float, 4).attrib(2, AttribType::Float, 2);

    // Textures (all generated here).
    let stone = image(64, 64, |x, y| {
        let n = noise(x, y, 8, 1) * 0.7 + noise(x, y, 3, 2) * 0.3;
        [(90.0 + 120.0 * n) as u8, (70.0 + 90.0 * n) as u8, (50.0 + 60.0 * n) as u8, 255]
    });
    let stone = Texture::from_rgba(64, 64, &stone, TexFormat::Rgb565)?;
    let grid = image(16, 16, |x, y| if x == 0 || y == 0 { [60, 60, 60, 255] } else { [255, 255, 255, 255] });
    let mut grid = Texture::from_rgba(16, 16, &grid, TexFormat::L8)?;
    grid.set_wrap(Wrap::Repeat, Wrap::Repeat);
    let sky = image(64, 64, |x, y| {
        let n = noise(x, y, 16, 3);
        [(40.0 + 60.0 * n) as u8, (90.0 + 80.0 * n) as u8, (200.0 + 55.0 * n) as u8 / 1, 255]
    });
    let sky = Texture::from_rgba(64, 64, &sky, TexFormat::Rgba8)?;
    // Hemispherical bumps encoded as normals: rgb = n * 0.5 + 0.5.
    let bumps = image(64, 64, |x, y| {
        let (dx, dy) = ((x % 16) as f32 - 7.5, (y % 16) as f32 - 7.5);
        let r2 = (dx * dx + dy * dy) / (7.0 * 7.0);
        let (nx, ny, nz) = if r2 < 1.0 {
            let nz = math::sqrt(1.0 - r2);
            (dx / 7.0, -dy / 7.0, nz)
        } else {
            (0.0, 0.0, 1.0)
        };
        [((nx * 0.5 + 0.5) * 255.0) as u8, ((ny * 0.5 + 0.5) * 255.0) as u8, ((nz * 0.5 + 0.5) * 255.0) as u8, 255]
    });
    let bumps = Texture::from_rgba(64, 64, &bumps, TexFormat::Rgb8)?;
    let dissolve = image(64, 64, |x, y| {
        let n = noise(x, y, 6, 4);
        [255, (150.0 + 100.0 * n) as u8, 40, (n * 255.0) as u8]
    });
    let dissolve = Texture::from_rgba(64, 64, &dissolve, TexFormat::Rgba8)?;

    // Six quads with a colour gradient across each.
    let mut v = Vec::new();
    for i in 0..6 {
        let (x, y) = panel_origin(i);
        let c = |r: f32, g: f32, b: f32| [r, g, b, 1.0];
        let (tl, tr, bl, br) = (c(1.0, 0.3, 0.3), c(0.3, 1.0, 0.3), c(0.3, 0.3, 1.0), c(1.0, 1.0, 1.0));
        v.push(Vertex { pos: [x, y + PANEL_H, 0.0], color: bl, uv: [0.0, 0.0] });
        v.push(Vertex { pos: [x + PANEL_W, y + PANEL_H, 0.0], color: br, uv: [1.0, 0.0] });
        v.push(Vertex { pos: [x, y, 0.0], color: tl, uv: [0.0, 1.0] });
        v.push(Vertex { pos: [x + PANEL_W, y, 0.0], color: tr, uv: [1.0, 1.0] });
    }
    let vertices = LinearBuffer::from_slice(&v)?;
    log!("texenv: 6 panels, {} vertices", v.len());

    let names = ["tex x colour", "base x detail", "cross-fade", "two stages", "dot3 bumps", "alpha test"];
    let mut frame = 0u32;
    while apt.main_loop() {
        if input.scan().down().contains(Buttons::START) {
            break;
        }
        let t = frame as f32 / 60.0;
        gpu.clear(&top, gpu::rgba(24, 24, 32, 255), 0)?;
        gpu.bind_target(&top);
        gpu.bind_shader(&shader);
        gpu.set_layout(&layout);
        gpu.bind_vertices(&vertices);
        gpu.set_depth_test(false, TestFunc::Always, WriteMask::COLOR);
        gpu.set_cull(Cull::None);
        gpu.set_mat4(projection, &Mat4::screen_2d(400.0, 240.0));
        gpu.set_vec4(detail, [4.0, 4.0, 1.0, 1.0]);

        for panel in 0..6 {
            gpu.reset_texenv(1);
            gpu.bind_texture(1, None);
            gpu.set_alpha_test(false, TestFunc::Always, 0);
            gpu.set_blend(Some(Blend::ALPHA));
            match panel {
                0 => {
                    gpu.bind_texture(0, Some(&stone));
                    gpu.set_texenv(0, TexEnv::modulate(Source::Texture0, Source::PrimaryColor));
                }
                1 => {
                    gpu.bind_texture(0, Some(&stone));
                    gpu.bind_texture(1, Some(&grid));
                    gpu.set_texenv(0, TexEnv::modulate(Source::Texture0, Source::Texture1).scale(Scale::X2, Scale::X1));
                }
                2 => {
                    // A * C + B * (1 - C), C = the constant's alpha.
                    let fade = (math::sin(t * 2.0) * 0.5 + 0.5) * 255.0;
                    gpu.bind_texture(0, Some(&stone));
                    gpu.bind_texture(1, Some(&sky));
                    gpu.set_vec4(detail, [1.0, 1.0, 1.0, 1.0]);
                    gpu.set_texenv(
                        0,
                        TexEnv::PASS
                            .sources(Source::Texture0, Source::Texture1, Source::Constant)
                            .rgb_operands(RgbOperand::Color, RgbOperand::Color, RgbOperand::Alpha)
                            .alpha_operands(AlphaOperand::Alpha, AlphaOperand::Alpha, AlphaOperand::Alpha)
                            .func(Combine::Interpolate)
                            .constant(gpu::rgba(0, 0, 0, fade as u8)),
                    );
                }
                3 => {
                    let pulse = (math::sin(t * 4.0) * 0.5 + 0.5) * 120.0;
                    gpu.bind_texture(0, Some(&sky));
                    gpu.set_texenv(0, TexEnv::modulate(Source::Texture0, Source::PrimaryColor));
                    gpu.set_texenv(
                        1,
                        TexEnv::PASS
                            .sources(Source::Previous, Source::Constant, Source::PrimaryColor)
                            .rgb_func(Combine::Add)
                            .constant(gpu::rgba(pulse as u8, (pulse * 0.6) as u8, 0, 0)),
                    );
                }
                4 => {
                    // 4 * ((N - 0.5) . (L - 0.5)) with L the light direction.
                    let (lx, ly) = (math::cos(t * 1.5) * 0.7, math::sin(t * 1.5) * 0.7);
                    let lz = math::sqrt(1.0 - lx * lx - ly * ly);
                    let enc = |c: f32| ((c * 0.5 + 0.5) * 255.0) as u8;
                    gpu.bind_texture(0, Some(&bumps));
                    gpu.set_texenv(
                        0,
                        TexEnv::PASS
                            .rgb_sources(Source::Texture0, Source::Constant, Source::PrimaryColor)
                            .rgb_func(Combine::Dot3Rgb)
                            .alpha_sources(Source::Constant, Source::PrimaryColor, Source::PrimaryColor)
                            .constant(gpu::rgba(enc(lx), enc(ly), enc(lz), 255)),
                    );
                    gpu.set_texenv(1, TexEnv::modulate(Source::Previous, Source::PrimaryColor));
                }
                _ => {
                    let threshold = ((frame * 3) % 300).min(255) as u8;
                    gpu.bind_texture(0, Some(&dissolve));
                    gpu.set_texenv(0, TexEnv::replace(Source::Texture0));
                    gpu.set_alpha_test(true, TestFunc::Greater, threshold);
                    gpu.set_blend(None);
                }
            }
            gpu.draw_arrays(Primitive::TriangleStrip, panel as u32 * 4, 4);
        }
        screens.show_target(&mut gpu, &top, Screen::Top)?;

        // Labels drawn by the CPU on top of the GPU's image.
        let mut c = screens.top();
        for (i, name) in names.iter().enumerate() {
            let (x, y) = panel_origin(i);
            c.text(x as i32, (y + PANEL_H) as i32 + 4, name, Color::WHITE, 1);
        }
        let mut b = screens.bottom();
        b.clear(Color::rgb(16, 16, 24));
        b.text(8, 8, "Texture combiners", Color::WHITE, 2);
        for (i, line) in [
            "1  Modulate(Texture0, PrimaryColor)",
            "2  Modulate(Texture0, Texture1) x2",
            "3  Interpolate(T0, T1, Constant.a)",
            "4  Modulate, then Add(Previous, Constant)",
            "5  Dot3Rgb(Texture0, Constant light)",
            "6  alpha test: alpha > threshold",
        ]
        .iter()
        .enumerate()
        {
            b.text(8, 40 + i as i32 * 16, line, Color::rgb(200, 210, 230), 1);
        }
        b.text(8, 220, "START: exit", Color::WHITE, 1);

        if frame == 30 {
            let c = screens.top();
            let bg = |p: Color| p.r.abs_diff(24) < 4 && p.g.abs_diff(24) < 4 && p.b.abs_diff(32) < 4;
            let mut empty = 0;
            for i in 0..5 {
                let (x, y) = panel_origin(i);
                if bg(c.get((x + PANEL_W / 2.0) as i32, (y + PANEL_H / 2.0) as i32)) {
                    empty += 1;
                }
            }
            log!("{}", if empty == 0 { "panels drawn" } else { "empty panel" });
            // Threshold 90: some texels pass and some are discarded.
            let (x0, y0) = panel_origin(5);
            let (mut kept, mut dropped) = (0, 0);
            for y in (y0 as i32 + 4..(y0 + PANEL_H) as i32 - 4).step_by(3) {
                for x in (x0 as i32 + 4..(x0 + PANEL_W) as i32 - 4).step_by(3) {
                    if bg(c.get(x, y)) { dropped += 1 } else { kept += 1 }
                }
            }
            log!("{} ({} kept, {} dropped)", if dropped > 0 && kept > 0 { "alpha test discards" } else { "nothing discarded" }, kept, dropped);
        }
        screens.present();
        frame += 1;
    }
    log!("bye: {} frames", frame);
    Ok(())
}
