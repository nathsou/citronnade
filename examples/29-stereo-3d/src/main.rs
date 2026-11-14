//! Stereoscopic 3D on the top screen. The scene (a row of cubes receding
//! into the distance over a floor) is drawn twice, once per eye, with
//! projections shifted by the interocular distance, which follows the 3D
//! slider. Objects at the convergence plane appear at the screen's depth;
//! nearer ones pop out, farther ones sink in. Up and Down move the plane.
//! With the slider down both eyes see the same image. START exits.
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
    .fvec light_dir
    .fvec tint
    ; x = 0, y = 1, z = ambient light
    .constf consts(0.0, 1.0, 0.3, 0.0)
    .alias inpos v0
    .alias innrm v1
    .alias inclr v2
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
        dp3 r3.x, light_dir, r2
        max r3.x, consts.x, r3.x
        add r4, consts.zzzz, r3.xxxx
        mul r5, tint, inclr
        mul outclr.xyz, r5, r4
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

/// A unit cube (white, tinted per draw) and a
/// checkered floor.
fn geometry() -> (Vec<Vertex>, Vec<u16>, u32) {
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
            v.push(Vertex { pos: p.map(|x| x * 0.5), normal: n, color: [1.0, 1.0, 1.0] });
        }
        i.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    // Floor: 16x16 tiles from -8 to 8 in x and from 2 to -30 in z.
    for tz in 0..16 {
        for tx in 0..16 {
            let base = v.len() as u16;
            let shade = if (tx + tz) % 2 == 0 { 0.55 } else { 0.3 };
            let (x0, z0) = (-8.0 + tx as f32, 2.0 - tz as f32 * 2.0);
            for (dx, dz) in [(0.0, 0.0), (1.0, 0.0), (1.0, -2.0), (0.0, -2.0)] {
                v.push(Vertex { pos: [x0 + dx, -1.0, z0 + dz], normal: [0.0, 1.0, 0.0], color: [shade, shade, shade * 1.2] });
            }
            i.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        }
    }
    let floor_count = (i.len() - 36) as u32;
    (v, i, floor_count)
}

#[citronnade::main]
fn main(sys: System) -> Result<()> {
    let System { apt, gsp, hid, .. } = sys;
    let mut screens = gsp.screens()?;
    screens.set_top_mode(TopMode::Stereo);
    let mut input = hid.input()?;
    let mut gpu = Gpu::new()?;
    // One target serves both eyes: each transfer finishes before the next
    // eye is drawn.
    let target = RenderTarget::top()?;
    let shader = Shader::new(SHADER).expect("shader");
    let u_projection = shader.uniform("projection").unwrap();
    let u_modelview = shader.uniform("modelview").unwrap();
    let u_light = shader.uniform("light_dir").unwrap();
    let u_tint = shader.uniform("tint").unwrap();
    let layout = VertexLayout::new().attrib(0, AttribType::Float, 3).attrib(1, AttribType::Float, 3).attrib(2, AttribType::Float, 3);
    let (verts, idx, floor_count) = geometry();
    let vertices = LinearBuffer::from_slice(&verts)?;
    let cube_indices = LinearBuffer::from_slice(&idx[..36])?;
    let floor_indices = LinearBuffer::from_slice(&idx[36..])?;
    log!("stereo-3d: {} vertices; Up/Down move the screen plane", verts.len());

    let mut screen_plane = 4.0f32;
    let mut frame = 0u32;
    let mut last_slider = -1.0f32;
    while apt.main_loop() {
        let down = input.scan().down();
        if down.contains(Buttons::START) {
            break;
        }
        if down.contains(Buttons::DPAD_UP) {
            screen_plane = (screen_plane + 1.0).min(20.0);
            log!("screen plane {:.1}", screen_plane);
        }
        if down.contains(Buttons::DPAD_DOWN) {
            screen_plane = (screen_plane - 1.0).max(1.0);
            log!("screen plane {:.1}", screen_plane);
        }
        let slider = citronnade::slider_3d();
        if (slider - last_slider).abs() > 0.01 {
            log!("slider {:.2}", slider);
            last_slider = slider;
        }
        // Interocular distance in world units: up to 0.3 at full slider.
        let iod = slider * 0.3;
        let t = frame as f32 / 60.0;
        let view = Mat4::look_at(Vec3::new(0.0, 0.6, 3.0), Vec3::new(0.0, 0.0, -2.0), Vec3::new(0.0, 1.0, 0.0));
        let l = Vec3::new(-0.3, 0.8, 0.5).normalize();

        for (eye, sign) in [(0, -1.0f32), (1, 1.0)] {
            let projection = Mat4::perspective_stereo_tilt(math::to_radians(55.0), 400.0 / 240.0, 0.1, 60.0, sign * iod, screen_plane);
            gpu.clear(&target, gpu::rgba(14, 18, 30, 255), 0)?;
            gpu.bind_target(&target);
            gpu.bind_shader(&shader);
            gpu.set_layout(&layout);
            gpu.bind_vertices(&vertices);
            gpu.set_texenv(0, TexEnv::replace(Source::PrimaryColor));
            gpu.set_depth_test(true, TestFunc::Greater, WriteMask::ALL);
            gpu.set_cull(Cull::BackCcw);
            gpu.set_blend(None);
            gpu.set_mat4(u_projection, &projection);
            gpu.set_vec4(u_light, [l.x, l.y, l.z, 0.0]);
            // Floor.
            gpu.set_vec4(u_tint, [1.0, 1.0, 1.0, 1.0]);
            gpu.set_mat4(u_modelview, &view);
            gpu.draw_elements(Primitive::Triangles, &floor_indices, floor_count);
            // Cubes from 1.5 units in front of the camera to 25 behind it.
            for k in 0..7 {
                let z = 1.5 - k as f32 * 4.0;
                let x = if k % 2 == 0 { -1.2 } else { 1.2 };
                let model = Mat4::translation(x, -0.3, z) * Mat4::rotation_y(t + k as f32) * Mat4::rotation_x(0.4);
                let c = Color::hsv((k * 50) as u16, 170, 255);
                gpu.set_vec4(u_tint, [c.r as f32 / 255.0, c.g as f32 / 255.0, c.b as f32 / 255.0, 1.0]);
                gpu.set_mat4(u_modelview, &(view * model));
                gpu.draw_elements(Primitive::Triangles, &cube_indices, 36);
            }
            if eye == 0 {
                screens.show_target(&mut gpu, &target, Screen::Top)?;
            } else {
                screens.show_target_right(&mut gpu, &target)?;
            }
        }

        if frame == 30 {
            // The nearest cube sits in front of the screen plane, so it is
            // shifted between the eyes.
            let (left, right) = screens.top_stereo();
            let mut diff = 0u32;
            for x in (0..400).step_by(2) {
                let (a, b) = (left.get(x, 150), right.get(x, 150));
                diff += (a.r.abs_diff(b.r) as u32 + a.g.abs_diff(b.g) as u32 + a.b.abs_diff(b.b) as u32 > 30) as u32;
            }
            log!("{} ({} of 200 pixels differ, slider {:.2})", if diff > 0 { "eyes differ" } else { "eyes identical" }, diff, slider);
        }

        let mut b = screens.bottom();
        b.clear(Color::rgb(16, 16, 24));
        b.text(8, 8, "Stereo 3D", Color::WHITE, 2);
        b.text(8, 36, &format!("3D slider      {:.2}", slider), Color::rgb(200, 210, 230), 1);
        b.text(8, 50, &format!("eye separation {:.3}", iod), Color::rgb(200, 210, 230), 1);
        b.text(8, 64, &format!("screen plane   {:.1} units", screen_plane), Color::rgb(200, 210, 230), 1);
        // Top view: the camera, the screen plane and the cubes.
        let (cx, cy, scale) = (160, 200, 5.0);
        b.line(20, cy - (screen_plane * scale) as i32, 300, cy - (screen_plane * scale) as i32, Color::rgb(90, 200, 255));
        for k in 0..7 {
            let z = 3.0 - (1.5 - k as f32 * 4.0);
            let x = if k % 2 == 0 { -1.2 } else { 1.2 };
            let c = Color::hsv((k * 50) as u16, 170, 255);
            b.fill_rect(cx + (x * scale * 4.0) as i32 - 2, cy - (z * scale) as i32 - 2, 5, 5, c);
        }
        b.fill_circle(cx, cy, 3, Color::WHITE);
        b.text(8, 220, "Up/Down: screen plane  START: exit", Color::WHITE, 1);
        screens.present();
        frame += 1;
    }
    log!("bye: {} frames", frame);
    Ok(())
}
