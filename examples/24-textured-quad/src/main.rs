//! Textures on quads. The top screen shows, left to right: an orientation
//! test (red top left, green top right, blue bottom left, yellow bottom
//! right), the same 8x8 checkerboard magnified with nearest and with linear
//! filtering, and texture coordinates from -1 to 2 with repeat, mirrored
//! repeat and clamp. The bottom screen spins an RGB565 texture generated on
//! the console and, behind it, a tile converted from a PNG at build time
//! (`build.rs`, with `citronnade-assets`). START exits.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec::Vec;
use citronnade::gpu::{
    self, shader, AttribType, Cull, Filter, Gpu, LinearBuffer, Primitive, RenderTarget, Shader, Source, TexEnv, TexFormat,
    TestFunc, Texture, VertexLayout, Wrap, WriteMask,
};
use citronnade::math::{self, Mat4};
use citronnade::prelude::*;

/// `assets/tile.png`, tiled into RGBA5551 by `build.rs`.
const TILE: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/tile.ctex"));

const SHADER: &[u8] = shader!(
    r#"
    .fvec projection[4]
    .constf ones(1.0, 1.0, 1.0, 1.0)
    .alias inpos v0
    .alias intc v1
    .out outpos position
    .out outtc texcoord0

    .proc main
        mov r0.xyz, inpos
        mov r0.w, ones
        dp4 outpos.x, projection[0], r0
        dp4 outpos.y, projection[1], r0
        dp4 outpos.z, projection[2], r0
        dp4 outpos.w, projection[3], r0
        mov outtc, intc
        end
    .end
    "#
);

#[derive(Clone, Copy)]
#[repr(C)]
struct Vertex {
    pos: [f32; 3],
    uv: [f32; 2],
}

/// A quad as a triangle strip: (x, y) top left, size, and the texture
/// coordinates of its left/right and top/bottom edges (v = 1 is the top of
/// the image).
fn quad(out: &mut Vec<Vertex>, x: f32, y: f32, w: f32, h: f32, u0: f32, u1: f32, v_top: f32, v_bottom: f32) {
    out.push(Vertex { pos: [x, y + h, 0.0], uv: [u0, v_bottom] });
    out.push(Vertex { pos: [x + w, y + h, 0.0], uv: [u1, v_bottom] });
    out.push(Vertex { pos: [x, y, 0.0], uv: [u0, v_top] });
    out.push(Vertex { pos: [x + w, y, 0.0], uv: [u1, v_top] });
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
    let bottom = RenderTarget::bottom()?;
    let shader = Shader::new(SHADER).expect("shader");
    let projection = shader.uniform("projection").unwrap();
    let layout = VertexLayout::new().attrib(0, AttribType::Float, 3).attrib(1, AttribType::Float, 2);

    // Quadrants: red, green / blue, yellow; a white border marks the edges.
    let quadrants = image(32, 32, |x, y| {
        if x == 0 || y == 0 || x == 31 || y == 31 {
            return [255, 255, 255, 255];
        }
        match (x < 16, y < 16) {
            (true, true) => [230, 40, 40, 255],
            (false, true) => [40, 200, 60, 255],
            (true, false) => [50, 80, 230, 255],
            (false, false) => [240, 220, 40, 255],
        }
    });
    let mut orientation = Texture::from_rgba(32, 32, &quadrants, TexFormat::Rgba8)?;
    orientation.set_filter(Filter::Nearest, Filter::Nearest);

    let checker = image(8, 8, |x, y| if (x + y) % 2 == 0 { [250, 250, 250, 255] } else { [30, 120, 200, 255] });
    let mut nearest = Texture::from_rgba(8, 8, &checker, TexFormat::Rgba8)?;
    nearest.set_filter(Filter::Nearest, Filter::Nearest);
    let mut linear = Texture::from_rgba(8, 8, &checker, TexFormat::Rgba8)?;
    linear.set_filter(Filter::Linear, Filter::Linear);

    // An arrow pointing up-right, to make mirroring visible.
    let arrow = image(16, 16, |x, y| {
        let on = (y < 4 && x >= 8) || (x >= 12 && y < 8) || (x + y == 15 || x + y == 16);
        if on { [255, 140, 0, 255] } else { [40, 40, 60, 255] }
    });
    let mut wraps = Vec::new();
    for w in [Wrap::Repeat, Wrap::MirroredRepeat, Wrap::ClampToEdge] {
        let mut t = Texture::from_rgba(16, 16, &arrow, TexFormat::Rgba8)?;
        t.set_filter(Filter::Nearest, Filter::Nearest);
        t.set_wrap(w, w);
        wraps.push(t);
    }

    // A smooth procedural pattern in RGB565 for the bottom screen.
    let rings = image(64, 64, |x, y| {
        let (dx, dy) = (x as f32 - 31.5, y as f32 - 31.5);
        let d = math::sqrt(dx * dx + dy * dy);
        let t = (math::sin(d * 0.6) * 0.5 + 0.5) * 255.0;
        [t as u8, (255.0 - t) as u8 / 2 + 60, 200, 255]
    });
    let rings = Texture::from_rgba(64, 64, &rings, TexFormat::Rgb565)?;
    let mut tile = Texture::from_ctex(TILE)?;
    tile.set_wrap(Wrap::Repeat, Wrap::Repeat);
    tile.set_filter(Filter::Nearest, Filter::Nearest);

    let mut v = Vec::new();
    quad(&mut v, 10.0, 20.0, 90.0, 90.0, 0.0, 1.0, 1.0, 0.0); // orientation
    quad(&mut v, 110.0, 20.0, 90.0, 90.0, 0.0, 1.0, 1.0, 0.0); // nearest
    quad(&mut v, 210.0, 20.0, 90.0, 90.0, 0.0, 1.0, 1.0, 0.0); // linear
    for i in 0..3 {
        quad(&mut v, 10.0 + i as f32 * 130.0, 130.0, 100.0, 100.0, -1.0, 2.0, 2.0, -1.0); // wraps
    }
    quad(&mut v, -60.0, -60.0, 120.0, 120.0, 0.0, 1.0, 1.0, 0.0); // spinning (centred)
    quad(&mut v, 0.0, 0.0, 320.0, 240.0, 0.0, 5.0, 3.75, 0.0); // tiled background
    let vertices = LinearBuffer::from_slice(&v)?;
    log!("textured-quad: {} vertices, {} KiB of textures", v.len(), (32 * 32 * 4 + 2 * 8 * 8 * 4 + 3 * 16 * 16 * 4 + 64 * 64 * 2) / 1024);

    let mut frame = 0u32;
    while apt.main_loop() {
        if input.scan().down().contains(Buttons::START) {
            break;
        }
        gpu.clear(&top, gpu::rgba(20, 20, 28, 255), 0)?;
        gpu.bind_target(&top);
        gpu.bind_shader(&shader);
        gpu.set_layout(&layout);
        gpu.bind_vertices(&vertices);
        gpu.set_texenv(0, TexEnv::replace(Source::Texture0));
        gpu.set_depth_test(false, TestFunc::Always, WriteMask::COLOR);
        gpu.set_cull(Cull::None);
        gpu.set_mat4(projection, &Mat4::screen_2d(400.0, 240.0));
        let top_textures = [&orientation, &nearest, &linear, &wraps[0], &wraps[1], &wraps[2]];
        for (i, t) in top_textures.iter().enumerate() {
            gpu.bind_texture(0, Some(t));
            gpu.draw_arrays(Primitive::TriangleStrip, i as u32 * 4, 4);
        }
        screens.show_target(&mut gpu, &top, Screen::Top)?;

        gpu.clear(&bottom, gpu::rgba(10, 10, 16, 255), 0)?;
        gpu.bind_target(&bottom);
        gpu.set_mat4(projection, &Mat4::screen_2d(320.0, 240.0));
        gpu.bind_texture(0, Some(&tile));
        gpu.draw_arrays(Primitive::TriangleStrip, 28, 4);
        let m = Mat4::screen_2d(320.0, 240.0) * Mat4::translation(160.0, 120.0, 0.0) * Mat4::rotation_z(frame as f32 * 0.02);
        gpu.set_mat4(projection, &m);
        gpu.bind_texture(0, Some(&rings));
        gpu.draw_arrays(Primitive::TriangleStrip, 24, 4);
        screens.show_target(&mut gpu, &bottom, Screen::Bottom)?;

        if frame == 30 {
            let c = screens.top();
            let near = |p: (i32, i32), want: (u8, u8, u8)| {
                let got = c.get(p.0, p.1);
                got.r.abs_diff(want.0) < 12 && got.g.abs_diff(want.1) < 12 && got.b.abs_diff(want.2) < 12
            };
            let ok = near((30, 40), (230, 40, 40))
                && near((80, 40), (40, 200, 60))
                && near((30, 90), (50, 80, 230))
                && near((80, 90), (240, 220, 40));
            log!("orientation {}", if ok { "ok" } else { "wrong" });
        }
        screens.present();
        frame += 1;
    }
    log!("bye: {} frames", frame);
    Ok(())
}
