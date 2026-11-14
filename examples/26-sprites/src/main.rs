//! Thousands of sprites in one draw call: every frame the CPU writes four
//! vertices per sprite into a linear-memory vertex buffer, and one indexed
//! draw renders them all from a generated sprite atlas, tinted by vertex
//! colour (given as bytes, scaled to 0..1 in the vertex shader).
//!
//! A toggles alpha/additive blending, L and R change the number of sprites,
//! touching spawns sprites at the stylus. START exits.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use alloc::vec::Vec;
use citronnade::gpu::{
    self, shader, AttribType, Blend, Cull, Gpu, LinearBuffer, Primitive, RenderTarget, Shader, Source, TexEnv, TexFormat, TestFunc,
    Texture, VertexLayout, WriteMask,
};
use citronnade::math::{self, Mat4};
use citronnade::prelude::*;
use citronnade::time;

const SHADER: &[u8] = shader!(
    r#"
    .fvec projection[4]
    ; x = 0, y = 1, z = 1/255
    .constf consts(0.0, 1.0, 0.0039215686, 0.0)
    .alias inpos v0     ; x, y in pixels
    .alias intc v1      ; u, v
    .alias inclr v2     ; r, g, b, a as bytes
    .out outpos position
    .out outtc texcoord0
    .out outclr color

    .proc main
        mov r0.xy, inpos
        mov r0.zw, consts.xxxy      ; z = 0, w = 1
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

#[derive(Clone, Copy)]
struct Sprite {
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
    angle: f32,
    spin: f32,
    size: f32,
    kind: u8,
    color: [u8; 4],
}

const MAX_SPRITES: usize = 8000;

/// A 64x64 atlas of four white 32x32 sprites: an orb, a star, a ring and a gem.
fn atlas() -> Vec<u8> {
    let mut img = alloc::vec![0u8; 64 * 64 * 4];
    for kind in 0..4 {
        let (ox, oy) = ((kind % 2) * 32, (kind / 2) * 32);
        for y in 0..32 {
            for x in 0..32 {
                let (dx, dy) = (x as f32 - 15.5, y as f32 - 15.5);
                let r = math::sqrt(dx * dx + dy * dy) / 15.5;
                let a = match kind {
                    0 => (1.0 - r).max(0.0) * (1.0 - r).max(0.0) * 1.6,
                    1 => {
                        let ang = math::atan2(dy, dx);
                        let star = 0.55 + 0.4 * math::cos(ang * 5.0);
                        if r < star { 1.0 } else { ((star + 0.08 - r) / 0.08).max(0.0) }
                    }
                    2 => (1.0 - (r - 0.7).abs() / 0.2).max(0.0),
                    _ => {
                        let d = dx.abs() + dy.abs() * 0.8;
                        if d < 13.0 { if dx > 0.0 { 1.0 } else { 0.75 } } else { 0.0 }
                    }
                };
                let a = (a.min(1.0) * 255.0) as u8;
                let i = ((oy + y) * 64 + ox + x) * 4;
                img[i..i + 4].copy_from_slice(&[255, 255, 255, a]);
            }
        }
    }
    img
}

struct Rng(u32);

impl Rng {
    fn next(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        self.0
    }

    fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (self.next() % 10_000) as f32 / 10_000.0 * (hi - lo)
    }
}

fn spawn(rng: &mut Rng, x: f32, y: f32) -> Sprite {
    let hue = (rng.next() % 360) as u16;
    let c = Color::hsv(hue, 180, 255);
    Sprite {
        x,
        y,
        vx: rng.range(-90.0, 90.0),
        vy: rng.range(-90.0, 90.0),
        angle: rng.range(0.0, 6.28),
        spin: rng.range(-3.0, 3.0),
        size: rng.range(5.0, 12.0),
        kind: (rng.next() % 4) as u8,
        color: [c.r, c.g, c.b, 220],
    }
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
    let layout = VertexLayout::new().attrib(0, AttribType::Float, 2).attrib(1, AttribType::Float, 2).attrib(2, AttribType::UByte, 4);
    assert_eq!(layout.stride(), core::mem::size_of::<Vertex>());

    let atlas = Texture::from_rgba(64, 64, &atlas(), TexFormat::Rgba8)?;
    log!("sprites: atlas of 4 sprites, up to {} sprites", MAX_SPRITES);

    let mut vertices: LinearBuffer<Vertex> = LinearBuffer::zeroed(MAX_SPRITES * 4)?;
    let mut idx = Vec::with_capacity(MAX_SPRITES * 6);
    for i in 0..MAX_SPRITES as u16 {
        let b = i.wrapping_mul(4);
        idx.extend_from_slice(&[b, b + 1, b + 2, b + 2, b + 1, b + 3]);
    }
    // 16-bit indices reach 65536 vertices: 16384 sprites.
    let indices = LinearBuffer::from_slice(&idx)?;

    let mut rng = Rng(0x1234_5678);
    let mut sprites: Vec<Sprite> = (0..1000).map(|_| spawn(&mut rng, 200.0, 120.0)).collect();
    let mut additive = false;
    let (mut total_us, mut frames) = (0u64, 0u64);
    let mut last = time::Instant::now();

    while apt.main_loop() {
        let state = input.scan();
        let down = state.down();
        if down.contains(Buttons::START) {
            break;
        }
        if down.contains(Buttons::A) {
            additive = !additive;
            log!("blending: {}", if additive { "additive" } else { "alpha" });
        }
        if down.contains(Buttons::R) {
            let n = (sprites.len() + 1000).min(MAX_SPRITES);
            while sprites.len() < n {
                sprites.push(spawn(&mut rng, 200.0, 120.0));
            }
            log!("sprites: {}", sprites.len());
        }
        if down.contains(Buttons::L) {
            let n = sprites.len().saturating_sub(1000).max(100);
            sprites.truncate(n);
            log!("sprites: {}", sprites.len());
        }
        if let Some(t) = state.touch() {
            for _ in 0..10 {
                if sprites.len() < MAX_SPRITES {
                    sprites.push(spawn(&mut rng, t.0 as f32 * 400.0 / 320.0, t.1 as f32));
                }
            }
        }

        let start = time::Instant::now();
        let dt = last.elapsed().as_micros().min(50_000) as f32 / 1e6;
        last = start;

        // Move and write the vertices.
        for (s, quad) in sprites.iter_mut().zip(vertices.chunks_exact_mut(4)) {
            s.x += s.vx * dt;
            s.y += s.vy * dt;
            if s.x < 0.0 || s.x > 400.0 {
                s.vx = -s.vx;
                s.x = s.x.clamp(0.0, 400.0);
            }
            if s.y < 0.0 || s.y > 240.0 {
                s.vy = -s.vy;
                s.y = s.y.clamp(0.0, 240.0);
            }
            s.angle += s.spin * dt;
            let (sn, cs) = (math::sin(s.angle) * s.size, math::cos(s.angle) * s.size);
            let (u0, v1) = ((s.kind % 2) as f32 * 0.5, 1.0 - (s.kind / 2) as f32 * 0.5);
            let corners = [(-1.0, 1.0, u0, v1 - 0.5), (1.0, 1.0, u0 + 0.5, v1 - 0.5), (-1.0, -1.0, u0, v1), (1.0, -1.0, u0 + 0.5, v1)];
            for (v, (cx, cy, u, vv)) in quad.iter_mut().zip(corners) {
                *v = Vertex { pos: [s.x + cx * cs - cy * sn, s.y + cx * sn + cy * cs], uv: [u, vv], color: s.color };
            }
        }

        gpu.clear(&top, gpu::rgba(8, 10, 20, 255), 0)?;
        gpu.bind_target(&top);
        gpu.bind_shader(&shader);
        gpu.set_layout(&layout);
        gpu.bind_vertices(&vertices);
        gpu.bind_texture(0, Some(&atlas));
        gpu.set_texenv(0, TexEnv::modulate(Source::Texture0, Source::PrimaryColor));
        gpu.set_depth_test(false, TestFunc::Always, WriteMask::COLOR);
        gpu.set_cull(Cull::None);
        gpu.set_blend(Some(if additive { Blend::ADD } else { Blend::ALPHA }));
        gpu.set_mat4(projection, &Mat4::screen_2d(400.0, 240.0));
        gpu.draw_elements(Primitive::Triangles, &indices, sprites.len() as u32 * 6);
        screens.show_target(&mut gpu, &top, Screen::Top)?;
        let us = start.elapsed().as_micros() as u64;
        total_us += us;
        frames += 1;

        let mut b = screens.bottom();
        b.clear(Color::rgb(16, 16, 24));
        b.text(8, 8, &format!("{} sprites", sprites.len()), Color::WHITE, 2);
        b.text(8, 34, &format!("frame {:.1} ms", us as f32 / 1000.0), Color::rgb(200, 210, 230), 1);
        b.text(8, 48, if additive { "blending: additive" } else { "blending: alpha" }, Color::rgb(200, 210, 230), 1);
        b.text(8, 190, "A: blending   L/R: fewer/more", Color::WHITE, 1);
        b.text(8, 206, "touch: spawn   START: exit", Color::WHITE, 1);
        screens.present();
    }
    log!("bye: average frame {} us (CPU update + GPU)", total_us / frames.max(1));
    Ok(())
}
