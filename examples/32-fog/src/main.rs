//! Fog: each fragment fades towards a fog colour by a factor the GPU looks
//! up from its depth. Pillars line an avenue that the camera travels along;
//! the far ones vanish into the haze. A switches between exponential fog in
//! the sky's colour and linear depth cueing towards black; Up and Down
//! change the density. START exits.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use alloc::vec::Vec;
use citronnade::gpu::{
    self, shader, AttribType, Cull, FogLut, Gpu, LinearBuffer, Primitive, RenderTarget, Shader, Source, TexEnv, TestFunc,
    VertexLayout, WriteMask,
};
use citronnade::math::{self, Mat4, Vec3};
use citronnade::prelude::*;

const SHADER: &[u8] = shader!(
    r#"
    .fvec projection[4], modelview[4]
    .fvec light_dir
    .fvec tint
    .constf consts(0.0, 1.0, 0.35, 0.0)
    .alias inpos v0
    .alias innrm v1
    .out outpos position
    .out outclr color

    .proc main
        mov r0.xyz, inpos
        mov r0.w, consts.y
        dp4 r1.x, modelview[0], r0
        dp4 r1.y, modelview[1], r0
        dp4 r1.z, modelview[2], r0
        dp4 r1.w, modelview[3], r0
        dp4 outpos.x, projection[0], r1
        dp4 outpos.y, projection[1], r1
        dp4 outpos.z, projection[2], r1
        dp4 outpos.w, projection[3], r1
        dp3 r2.x, modelview[0], innrm
        dp3 r2.y, modelview[1], innrm
        dp3 r2.z, modelview[2], innrm
        ; normalise (the model matrices scale the boxes)
        dp3 r2.w, r2, r2
        rsq r2.w, r2.w
        mul r2.xyz, r2.xyz, r2.w
        dp3 r3.x, light_dir, r2
        max r3.x, consts.x, r3.x
        add r4, consts.zzzz, r3.xxxx
        mul outclr.xyz, tint, r4
        mov outclr.w, consts.y
        end
    .end
    "#
);

#[derive(Clone, Copy)]
#[repr(C)]
struct Vertex {
    pos: [f32; 3],
    normal: [f32; 3],
}

fn cube() -> (Vec<Vertex>, Vec<u16>) {
    let faces: [([f32; 3], [f32; 3], [f32; 3]); 6] = [
        ([0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
        ([0.0, 0.0, -1.0], [-1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
        ([1.0, 0.0, 0.0], [0.0, 0.0, -1.0], [0.0, 1.0, 0.0]),
        ([-1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]),
        ([0.0, 1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, -1.0]),
        ([0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
    ];
    let mut v = Vec::new();
    let mut i = Vec::new();
    for (n, u, w) in faces {
        let base = v.len() as u16;
        for (su, sv) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
            let p = [n[0] + u[0] * su + w[0] * sv, n[1] + u[1] * su + w[1] * sv, n[2] + u[2] * su + w[2] * sv];
            v.push(Vertex { pos: p.map(|x| x * 0.5), normal: n });
        }
        i.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    (v, i)
}

const NEAR: f32 = 0.2;
const FAR: f32 = 80.0;
const SKY: (u8, u8, u8) = (150, 170, 200);

#[citronnade::main]
fn main(sys: System) -> Result<()> {
    let System { apt, gsp, hid, .. } = sys;
    let mut screens = gsp.screens()?;
    let mut input = hid.input()?;
    let mut gpu = Gpu::new()?;
    let top = RenderTarget::top()?;
    let shader = Shader::new(SHADER).expect("shader");
    let u_projection = shader.uniform("projection").unwrap();
    let u_modelview = shader.uniform("modelview").unwrap();
    let u_light = shader.uniform("light_dir").unwrap();
    let u_tint = shader.uniform("tint").unwrap();
    let layout = VertexLayout::new().attrib(0, AttribType::Float, 3).attrib(1, AttribType::Float, 3);
    let (v, i) = cube();
    let vertices = LinearBuffer::from_slice(&v)?;
    let indices = LinearBuffer::from_slice(&i)?;
    log!("fog: pillars every 4 units to {} units away", FAR as u32);

    let projection = Mat4::perspective_tilt(math::to_radians(60.0), 400.0 / 240.0, NEAR, FAR);
    let mut density = 0.06f32;
    let mut cueing = false;
    let mut lut = FogLut::exp(density, 1.5, NEAR, FAR);
    let mut frame = 0u32;
    while apt.main_loop() {
        let down = input.scan().down();
        if down.contains(Buttons::START) {
            break;
        }
        let mut changed = false;
        if down.contains(Buttons::A) {
            cueing = !cueing;
            changed = true;
            log!("mode: {}", if cueing { "depth cueing" } else { "exponential fog" });
        }
        if down.contains(Buttons::DPAD_UP) {
            density = (density * 1.3).min(0.5);
            changed = true;
        }
        if down.contains(Buttons::DPAD_DOWN) {
            density = (density / 1.3).max(0.005);
            changed = true;
        }
        if changed {
            lut = if cueing { FogLut::linear(2.0, 2.0 + 1.5 / density, NEAR, FAR) } else { FogLut::exp(density, 1.5, NEAR, FAR) };
        }
        let (fog_color, clear) = if cueing { (gpu::rgba(0, 0, 0, 255), gpu::rgba(0, 0, 0, 255)) } else {
            (gpu::rgba(SKY.0, SKY.1, SKY.2, 255), gpu::rgba(SKY.0, SKY.1, SKY.2, 255))
        };

        // Travel forward; the avenue repeats every 4 units.
        let travel = if frame < 30 { 0.0 } else { (frame - 30) as f32 * 0.05 } % 4.0;
        let view = Mat4::look_at(Vec3::new(0.0, 1.0, -travel), Vec3::new(0.0, 0.7, -travel - 10.0), Vec3::new(0.0, 1.0, 0.0));
        let l = (view * Vec3::new(0.4, 0.8, 0.3).normalize().extend(0.0)).xyz();

        gpu.clear(&top, clear, 0)?;
        gpu.bind_target(&top);
        gpu.bind_shader(&shader);
        gpu.set_layout(&layout);
        gpu.bind_vertices(&vertices);
        gpu.set_texenv(0, TexEnv::replace(Source::PrimaryColor));
        gpu.set_depth_test(true, TestFunc::Greater, WriteMask::ALL);
        gpu.set_cull(Cull::BackCcw);
        gpu.set_blend(None);
        gpu.set_fog(Some((&lut, fog_color)));
        gpu.set_mat4(u_projection, &projection);
        gpu.set_vec4(u_light, [l.x, l.y, l.z, 0.0]);
        // The ground.
        gpu.set_vec4(u_tint, [0.35, 0.45, 0.3, 1.0]);
        gpu.set_mat4(u_modelview, &(view * Mat4::translation(0.0, -0.5, -40.0) * Mat4::scaling(60.0, 1.0, 90.0)));
        gpu.draw_elements(Primitive::Triangles, &indices, 36);
        // Two rows of pillars.
        for k in 0..20 {
            for side in [-1.0f32, 1.0] {
                let z = -(k as f32) * 4.0;
                let hue = Color::hsv(((k * 37) % 360) as u16, 160, 230);
                gpu.set_vec4(u_tint, [hue.r as f32 / 255.0, hue.g as f32 / 255.0, hue.b as f32 / 255.0, 1.0]);
                let m = Mat4::translation(side * 2.2, 1.5, z) * Mat4::scaling(0.6, 3.0, 0.6);
                gpu.set_mat4(u_modelview, &(view * m));
                gpu.draw_elements(Primitive::Triangles, &indices, 36);
            }
        }
        screens.show_target(&mut gpu, &top, Screen::Top)?;

        if frame == 20 {
            // Walk the right row of pillars from the nearest (screen edge)
            // to the farthest (centre): each pillar's colour must be closer
            // to the sky's than the one before.
            let c = screens.top();
            let dist = |p: Color| p.r.abs_diff(SKY.0) as u32 + p.g.abs_diff(SKY.1) as u32 + p.b.abs_diff(SKY.2) as u32;
            let mut runs: Vec<(u32, u32)> = Vec::new(); // (sum of distances, pixels)
            let mut inside = false;
            for x in (200..400).rev() {
                let d = dist(c.get(x, 100));
                if d > 6 {
                    if !inside {
                        runs.push((0, 0));
                        inside = true;
                    }
                    let r = runs.last_mut().unwrap();
                    r.0 += d;
                    r.1 += 1;
                } else {
                    inside = false;
                }
            }
            let avg: Vec<u32> = runs.iter().filter(|r| r.1 >= 2).map(|r| r.0 / r.1).collect();
            let fading = avg.len() >= 3 && avg.windows(2).all(|w| w[1] < w[0]);
            log!("far pillars {} (distance to the sky, near to far: {:?})", if fading { "fogged" } else { "not fogged" }, avg);
        }

        let mut b = screens.bottom();
        b.clear(Color::rgb(16, 16, 24));
        b.text(8, 8, "Fog", Color::WHITE, 2);
        b.text(8, 36, if cueing { "depth cueing (linear, to black)" } else { "exponential fog (sky colour)" }, Color::rgb(200, 210, 230), 1);
        b.text(8, 50, &format!("density {:.3}", density), Color::rgb(200, 210, 230), 1);
        b.text(32, 66, "fog factor against distance (1 = clear)", Color::rgb(170, 180, 200), 1);
        // The fog factor against distance.
        for x in 0..256 {
            let d = x as f32 / 256.0 * FAR;
            let depth = (FAR * NEAR / d.max(NEAR) - NEAR) / (FAR - NEAR);
            let idx = ((depth * 128.0) as usize).min(127);
            let f = (lut.factor(idx) * 100.0) as i32;
            b.fill_rect(32 + x, 180 - f, 1, f as u32, Color::rgb(90, 120, 170));
        }
        b.text(32, 184, "0", Color::WHITE, 1);
        b.text(260, 184, &format!("{}", FAR as u32), Color::WHITE, 1);
        b.text(8, 206, "A: fog / depth cueing  Up/Down: density", Color::WHITE, 1);
        b.text(8, 220, "START: exit", Color::WHITE, 1);
        screens.present();
        frame += 1;
    }
    log!("bye: {} frames", frame);
    Ok(())
}
