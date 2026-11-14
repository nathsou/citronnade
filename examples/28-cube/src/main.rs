//! A cube in perspective: a vertex shader transforms positions by the
//! model-view and projection matrices and lights each vertex with one
//! directional light; the depth buffer keeps the nearest surface (a wall
//! drawn after the cube stays behind it) and back faces are culled. The
//! Circle Pad orbits the camera, L and R zoom. START exits.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use alloc::vec::Vec;
use citronnade::gpu::{
    self, shader, AttribType, Cull, Gpu, LinearBuffer, Primitive, RenderTarget, Shader, Source, TexEnv, TestFunc, VertexLayout,
    WriteMask,
};
use citronnade::math::{self, Mat4, Vec3};
use citronnade::prelude::*;

const SHADER: &[u8] = shader!(
    r#"
    .fvec projection[4], modelview[4]
    .fvec light_dir         ; towards the light, view space, normalised
    .fvec ambient
    .constf consts(0.0, 1.0, 0.5, 0.0)
    .alias inpos v0
    .alias innrm v1
    .alias inclr v2
    .out outpos position
    .out outclr color

    .proc main
        mov r0.xyz, inpos
        mov r0.w, consts.y
        ; view-space position
        dp4 r1.x, modelview[0], r0
        dp4 r1.y, modelview[1], r0
        dp4 r1.z, modelview[2], r0
        dp4 r1.w, modelview[3], r0
        dp4 outpos.x, projection[0], r1
        dp4 outpos.y, projection[1], r1
        dp4 outpos.z, projection[2], r1
        dp4 outpos.w, projection[3], r1
        ; view-space normal (the model-view has no scaling)
        dp3 r2.x, modelview[0], innrm
        dp3 r2.y, modelview[1], innrm
        dp3 r2.z, modelview[2], innrm
        ; colour * (ambient + max(0, n . l))
        dp3 r3.x, light_dir, r2
        max r3.x, consts.x, r3.x
        add r4, ambient, r3.xxxx
        mul outclr.xyz, inclr, r4
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
    color: [f32; 3],
}

/// A unit cube's six faces (four vertices each, counter-clockwise from
/// outside) and a large wall.
fn geometry() -> (Vec<Vertex>, Vec<u16>) {
    let faces: [([f32; 3], [f32; 3], [f32; 3], [f32; 3]); 6] = [
        // normal, u axis, v axis, colour
        ([0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.95, 0.35, 0.3]),
        ([0.0, 0.0, -1.0], [-1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.3, 0.8, 0.4]),
        ([1.0, 0.0, 0.0], [0.0, 0.0, -1.0], [0.0, 1.0, 0.0], [0.3, 0.5, 0.95]),
        ([-1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0], [0.95, 0.85, 0.3]),
        ([0.0, 1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, -1.0], [0.85, 0.4, 0.9]),
        ([0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.4, 0.9, 0.9]),
    ];
    let mut v = Vec::new();
    let mut i = Vec::new();
    for (n, u, w, c) in faces {
        let base = v.len() as u16;
        for (su, sv) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
            let p = [n[0] + u[0] * su + w[0] * sv, n[1] + u[1] * su + w[1] * sv, n[2] + u[2] * su + w[2] * sv];
            v.push(Vertex { pos: p.map(|x| x * 0.5), normal: n, color: c });
        }
        i.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    // The wall: a big quad facing +z at z = -3 (in model space; drawn with
    // its own matrix that keeps it behind the cube).
    let base = v.len() as u16;
    for (x, y) in [(-9.0, -6.0), (9.0, -6.0), (9.0, 6.0), (-9.0, 6.0)] {
        v.push(Vertex { pos: [x, y, 0.0], normal: [0.0, 0.0, 1.0], color: [0.35, 0.35, 0.4] });
    }
    i.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    (v, i)
}

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
    let u_ambient = shader.uniform("ambient").unwrap();
    let layout = VertexLayout::new().attrib(0, AttribType::Float, 3).attrib(1, AttribType::Float, 3).attrib(2, AttribType::Float, 3);

    let (verts, idx) = geometry();
    log!("cube: {} vertices, {} indices", verts.len() - 4, idx.len() - 6);
    let vertices = LinearBuffer::from_slice(&verts)?;
    let cube_indices = LinearBuffer::from_slice(&idx[..36])?;
    let wall_indices = LinearBuffer::from_slice(&idx[36..])?;

    let projection = Mat4::perspective_tilt(math::to_radians(60.0), 400.0 / 240.0, 0.1, 50.0);
    let (mut yaw, mut pitch, mut distance) = (0.6f32, 0.4f32, 3.5f32);
    let mut spin = 0.0f32;
    let mut frame = 0u32;
    while apt.main_loop() {
        let state = input.scan();
        if state.down().contains(Buttons::START) {
            break;
        }
        let (sx, sy) = state.circle_pad();
        yaw += sx as f32 / 156.0 * 0.05;
        pitch = (pitch + sy as f32 / 156.0 * 0.05).clamp(-1.4, 1.4);
        if state.held().contains(Buttons::L) {
            distance = (distance + 0.05).min(10.0);
        }
        if state.held().contains(Buttons::R) {
            distance = (distance - 0.05).max(1.5);
        }
        if frame != 30 {
            spin += 0.01;
        }

        let eye = Vec3::new(distance * math::cos(pitch) * math::sin(yaw), distance * math::sin(pitch), distance * math::cos(pitch) * math::cos(yaw));
        let view = Mat4::look_at(eye, Vec3::ZERO, Vec3::new(0.0, 1.0, 0.0));
        let model = Mat4::rotation_y(spin) * Mat4::rotation_x(spin * 0.7);
        // The light comes from the upper left of the camera.
        let l = Vec3::new(-0.4, 0.7, 0.6).normalize();

        gpu.clear(&top, gpu::rgba(18, 22, 34, 255), 0)?;
        gpu.bind_target(&top);
        gpu.bind_shader(&shader);
        gpu.set_layout(&layout);
        gpu.bind_vertices(&vertices);
        gpu.set_texenv(0, TexEnv::replace(Source::PrimaryColor));
        gpu.set_depth_test(true, TestFunc::Greater, WriteMask::ALL);
        gpu.set_cull(Cull::BackCcw);
        gpu.set_blend(None);
        gpu.set_mat4(u_projection, &projection);
        gpu.set_vec4(u_light, [l.x, l.y, l.z, 0.0]);
        gpu.set_vec4(u_ambient, [0.25, 0.25, 0.3, 1.0]);
        gpu.set_mat4(u_modelview, &(view * model));
        gpu.draw_elements(Primitive::Triangles, &cube_indices, 36);
        // The wall, drawn second, 3 units behind the origin as seen by the
        // camera: the depth test must keep it behind the cube.
        let wall = Mat4::look_at(eye, Vec3::ZERO, Vec3::new(0.0, 1.0, 0.0)).inverse().unwrap()
            * Mat4::translation(0.0, 0.0, -distance - 3.0);
        gpu.set_mat4(u_modelview, &(view * wall));
        gpu.draw_elements(Primitive::Triangles, &wall_indices, 6);
        screens.show_target(&mut gpu, &top, Screen::Top)?;

        if frame == 30 {
            let c = screens.top();
            let centre = c.get(200, 120);
            let wall_px = c.get(30, 120);
            let is_wall = |p: Color| p.r.abs_diff(p.b) < 30 && p.r.abs_diff(p.g) < 10 && p.r > 40;
            log!("depth test {} centre {:?} wall {:?}", if !is_wall(centre) && is_wall(wall_px) { "ok" } else { "wrong" }, (centre.r, centre.g, centre.b), (wall_px.r, wall_px.g, wall_px.b));
        }
        if frame == 31 {
            // Drawing with front faces culled instead shows the inside (the
            // far faces): the centre colour must change.
            let before = screens.top().get(200, 120);
            gpu.clear(&top, gpu::rgba(18, 22, 34, 255), 0)?;
            gpu.bind_target(&top);
            gpu.set_cull(Cull::FrontCcw);
            gpu.set_mat4(u_modelview, &(view * model));
            gpu.draw_elements(Primitive::Triangles, &cube_indices, 36);
            screens.show_target(&mut gpu, &top, Screen::Top)?;
            let after = screens.top().get(200, 120);
            let differs = before.r.abs_diff(after.r) + before.g.abs_diff(after.g) + before.b.abs_diff(after.b) > 20;
            log!("culling {} ({:?} -> {:?})", if differs { "ok" } else { "wrong" }, (before.r, before.g, before.b), (after.r, after.g, after.b));
        }

        let mut b = screens.bottom();
        b.clear(Color::rgb(16, 16, 24));
        b.text(8, 8, "Cube", Color::WHITE, 2);
        b.text(8, 36, &format!("yaw {:5.2}  pitch {:5.2}", yaw, pitch), Color::rgb(200, 210, 230), 1);
        b.text(8, 50, &format!("distance {:4.1}", distance), Color::rgb(200, 210, 230), 1);
        b.text(8, 206, "Circle Pad: orbit  L/R: zoom", Color::WHITE, 1);
        b.text(8, 220, "START: exit", Color::WHITE, 1);
        screens.present();
        frame += 1;
    }
    log!("bye: {} frames", frame);
    Ok(())
}
