//! The procedural texture unit computes texels from texture coordinates:
//! clamping and noise shape (u, v), a mapping function turns them into one
//! value, a table reshapes it, and a colour gradient colours it. No texture
//! memory is used. Six panels: wood rings, marble, stripes, bricks, a glow
//! with its own alpha, and drifting clouds. START exits.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec::Vec;
use citronnade::gpu::{
    self, shader, AttribType, Blend, Cull, Gpu, LinearBuffer, Primitive, ProcClamp, ProcLut, ProcMap, ProcNoise, ProcShift, ProcTex,
    RenderTarget, Shader, Source, TexEnv, TestFunc, VertexLayout, WriteMask,
};
use citronnade::math::{self, Mat4};
use citronnade::prelude::*;

const SHADER: &[u8] = shader!(
    r#"
    .fvec projection[4]
    .constf consts(0.0, 1.0, 0.0, 0.0)
    .alias inpos v0
    .alias intc v1
    .out outpos position
    .out outtc texcoord0
    .proc main
        mov r0.xy, inpos
        mov r0.zw, consts.xxxy
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
    pos: [f32; 2],
    uv: [f32; 2],
}

const W: f32 = 120.0;
const H: f32 = 96.0;

fn origin(i: usize) -> (f32, f32) {
    (10.0 + (i % 3) as f32 * 130.0, 12.0 + (i / 3) as f32 * 116.0)
}

/// A gradient of `n` colours between stops (0xAABBGGRR).
fn gradient(stops: &[(f32, [u8; 4])], n: usize) -> Vec<u32> {
    (0..n)
        .map(|i| {
            let t = i as f32 / (n - 1) as f32;
            let k = stops.iter().rposition(|s| s.0 <= t).unwrap_or(0).min(stops.len() - 2);
            let (t0, a) = stops[k];
            let (t1, b) = stops[k + 1];
            let f = ((t - t0) / (t1 - t0)).clamp(0.0, 1.0);
            let c: [u8; 4] = core::array::from_fn(|j| math::lerp(a[j] as f32, b[j] as f32, f) as u8);
            gpu::rgba(c[0], c[1], c[2], c[3])
        })
        .collect()
}

#[citronnade::main]
fn main(sys: System) -> Result<()> {
    let System { apt, gsp, hid, .. } = sys;
    let mut screens = gsp.screens()?;
    let mut input = hid.input()?;
    let mut gpu = Gpu::new()?;
    let top = RenderTarget::top()?;
    let shader = Shader::new(SHADER).expect("shader");
    let u_proj = shader.uniform("projection").unwrap();
    let layout = VertexLayout::new().attrib(0, AttribType::Float, 2).attrib(1, AttribType::Float, 2);

    // Panels' texture coordinates: centred -1..1 for the radial ones,
    // 0..n for the repeating ones.
    // (u from, u to, v from, v to)
    let ranges = [
        (-1.0, 1.0, -0.8, 0.8),
        (0.0, 1.0, 0.0, 0.8),
        (0.0, 6.0, 0.0, 4.8),
        (0.0, 3.0, 0.0, 4.8),
        (-1.0, 1.0, -0.8, 0.8),
        (0.0, 1.0, 0.0, 0.8),
    ];
    let mut v = Vec::new();
    for (i, &(u0, u1, v0, v1)) in ranges.iter().enumerate() {
        let (x, y) = origin(i);
        v.push(Vertex { pos: [x, y + H], uv: [u0, v0] });
        v.push(Vertex { pos: [x + W, y + H], uv: [u1, v0] });
        v.push(Vertex { pos: [x, y], uv: [u0, v1] });
        v.push(Vertex { pos: [x + W, y], uv: [u1, v1] });
    }
    let vertices = LinearBuffer::from_slice(&v)?;

    // 1. Wood: distance from the centre through a periodic table.
    let mut wood = ProcTex::new(ProcMap::SqrtAdd2, &gradient(&[(0.0, [110, 60, 25, 255]), (1.0, [215, 160, 95, 255])], 32));
    wood.rgb_lut = ProcLut::from_fn(|x| math::sin(x * 40.0) * 0.5 + 0.5);
    wood.noise = Some((ProcNoise { amplitude: 0.08, frequency: 0.6, phase: 0.0 }, ProcNoise { amplitude: 0.08, frequency: 0.6, phase: 0.3 }));
    // 2. Marble: noisy diagonal veins.
    let mut marble = ProcTex::new(ProcMap::Add, &gradient(&[(0.0, [60, 60, 70, 255]), (0.5, [230, 230, 235, 255]), (1.0, [180, 185, 200, 255])], 32));
    marble.rgb_lut = ProcLut::from_fn(|x| math::sin(x * 18.0) * 0.5 + 0.5);
    marble.noise = Some((ProcNoise { amplitude: 0.35, frequency: 0.3, phase: 0.1 }, ProcNoise { amplitude: 0.35, frequency: 0.3, phase: 0.7 }));
    // 3. Stripes: u alone, repeated, through a step table.
    let mut stripes = ProcTex::new(ProcMap::U, &[gpu::rgba(240, 80, 70, 255), gpu::rgba(250, 230, 210, 255)]);
    stripes.u_clamp = ProcClamp::Repeat;
    stripes.v_clamp = ProcClamp::Repeat;
    stripes.rgb_lut = ProcLut::from_fn(|x| if x < 0.5 { 0.0 } else { 1.0 });
    // 4. Bricks: repeated cells, the smaller coordinate near zero is mortar,
    // and rows are shifted by half a brick in alternate pairs.
    let mut bricks = ProcTex::new(ProcMap::Min, &[gpu::rgba(200, 195, 185, 255), gpu::rgba(170, 70, 50, 255)]);
    bricks.u_clamp = ProcClamp::Repeat;
    bricks.v_clamp = ProcClamp::Repeat;
    bricks.u_shift = ProcShift::Odd;
    bricks.rgb_lut = ProcLut::from_fn(|x| if x < 0.08 { 0.0 } else { 1.0 });
    // 5. A glow: colour and alpha both fall off with the distance.
    let mut glow = ProcTex::new(ProcMap::SqrtAdd2, &gradient(&[(0.0, [255, 250, 200, 255]), (0.4, [255, 150, 40, 200]), (1.0, [120, 20, 60, 0])], 32));
    glow.alpha_map = Some(ProcMap::SqrtAdd2);
    glow.alpha_lut = ProcLut::from_fn(|d| 1.0 - d);
    glow.u_clamp = ProcClamp::ToEdge;
    glow.v_clamp = ProcClamp::ToEdge;
    // 6. Clouds: noise whose phase drifts.
    let mut clouds = ProcTex::new(ProcMap::Add, &gradient(&[(0.0, [70, 120, 200, 255]), (0.6, [200, 220, 245, 255]), (1.0, [255, 255, 255, 255])], 32));
    clouds.noise_lut = ProcLut::from_fn(|x| x * x * (3.0 - 2.0 * x));
    log!("procedural-texture: 6 patterns, no texture memory");

    let mut frame = 0u32;
    while apt.main_loop() {
        if input.scan().down().contains(Buttons::START) {
            break;
        }
        let t = frame as f32 / 60.0;
        clouds.noise = Some((ProcNoise { amplitude: 0.6, frequency: 0.25, phase: t * 0.2 }, ProcNoise { amplitude: 0.6, frequency: 0.25, phase: t * 0.13 }));

        gpu.clear(&top, gpu::rgba(24, 24, 32, 255), 0)?;
        gpu.bind_target(&top);
        gpu.bind_shader(&shader);
        gpu.set_layout(&layout);
        gpu.bind_vertices(&vertices);
        gpu.set_texenv(0, TexEnv::replace(Source::Texture3));
        gpu.set_depth_test(false, TestFunc::Always, WriteMask::COLOR);
        gpu.set_cull(Cull::None);
        gpu.set_blend(Some(Blend::ALPHA));
        gpu.set_mat4(u_proj, &Mat4::screen_2d(400.0, 240.0));
        for (i, p) in [&wood, &marble, &stripes, &bricks, &glow, &clouds].iter().enumerate() {
            gpu.set_proctex(Some(p));
            gpu.draw_arrays(Primitive::TriangleStrip, i as u32 * 4, 4);
        }
        gpu.set_proctex(None);
        screens.show_target(&mut gpu, &top, Screen::Top)?;

        let names = ["wood", "marble", "stripes", "bricks", "glow", "clouds"];
        let mut c = screens.top();
        for (i, n) in names.iter().enumerate() {
            let (x, y) = origin(i);
            c.text(x as i32, (y + H) as i32 + 3, n, Color::WHITE, 1);
        }
        if frame == 30 {
            let c = screens.top();
            let bg = |p: Color| p.r.abs_diff(24) < 4 && p.g.abs_diff(24) < 4 && p.b.abs_diff(32) < 4;
            let mut empty = 0;
            let mut flat = 0;
            for i in [0usize, 1, 2, 3, 5] {
                let (x, y) = origin(i);
                if bg(c.get((x + W / 2.0) as i32, (y + H / 2.0) as i32)) {
                    empty += 1;
                }
                // Count colour changes along three rows; keep the busiest.
                let mut changes = 0;
                for dy in [-17.0, -3.0, 11.0] {
                    let row = (y + H / 2.0 + dy) as i32;
                    let mut n = 0;
                    let mut last = c.get(x as i32 + 2, row);
                    for px in (x as i32 + 2..(x + W) as i32 - 2).step_by(2) {
                        let p = c.get(px, row);
                        if p.r.abs_diff(last.r) as u32 + p.g.abs_diff(last.g) as u32 + p.b.abs_diff(last.b) as u32 > 30 {
                            n += 1;
                        }
                        last = p;
                    }
                    changes = changes.max(n);
                }
                if changes < 2 && i != 5 {
                    flat += 1;
                    log!("panel {} has {} changes", names[i], changes);
                }
            }
            log!("{}", if empty == 0 { "panels drawn" } else { "empty panel" });
            log!("{}", if flat == 0 { "patterns vary" } else { "flat pattern" });
        }
        let mut b = screens.bottom();
        b.clear(Color::rgb(16, 16, 24));
        b.text(8, 8, "Procedural textures", Color::WHITE, 2);
        for (i, line) in [
            "wood     sqrt(u2+v2) -> sin table",
            "marble   (u+v)/2 + noise -> sin table",
            "stripes  u, repeat -> step table",
            "bricks   min(u, v), repeat, row shift",
            "glow     distance for colour and alpha",
            "clouds   noise with a drifting phase",
        ]
        .iter()
        .enumerate()
        {
            b.text(8, 40 + i as i32 * 16, line, Color::rgb(200, 210, 230), 1);
        }
        b.text(8, 220, "START: exit", Color::WHITE, 1);
        screens.present();
        frame += 1;
    }
    log!("bye: {} frames", frame);
    Ok(())
}
