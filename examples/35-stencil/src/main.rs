//! The stencil buffer, two classic uses:
//!
//! - a mirror: the floor is drawn into the stencil buffer only, the cube's
//!   mirror image is drawn where the stencil marks the floor (with culling
//!   flipped, since mirroring reverses the winding), then the floor is
//!   blended over it;
//! - an outline: the cube marks its pixels in the stencil buffer, then a
//!   slightly larger flat copy is drawn only where the cube is not.
//!
//! A toggles the outline. START exits.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec::Vec;
use citronnade::gpu::{
    self, shader, AttribType, Blend, Cull, Gpu, LinearBuffer, Primitive, RenderTarget, Shader, Source, StencilOp, TexEnv, TestFunc,
    VertexLayout, WriteMask,
};
use citronnade::math::{self, Mat4, Vec3};
use citronnade::prelude::*;

const SHADER: &[u8] = shader!(
    r#"
    .fvec projection[4], modelview[4]
    .fvec light_dir
    .fvec tint
    .fvec ambient           ; x: ambient light
    .constf consts(0.0, 1.0, 0.0, 0.0)
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
        dp3 r2.w, r2, r2
        rsq r2.w, r2.w
        mul r2.xyz, r2.xyz, r2.w
        dp3 r3.x, light_dir, r2
        max r3.x, consts.x, r3.x
        add r4, ambient.xxxx, r3.xxxx
        mul r5, tint, inclr
        mul outclr.xyz, r5, r4
        mov outclr.w, tint.w
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

fn geometry() -> (Vec<Vertex>, Vec<u16>) {
    let faces: [([f32; 3], [f32; 3], [f32; 3], [f32; 3]); 6] = [
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
    // The mirror: a 3x3 square at y = 0, facing up.
    let base = v.len() as u16;
    for (x, z) in [(-1.5, 1.5), (1.5, 1.5), (1.5, -1.5), (-1.5, -1.5)] {
        v.push(Vertex { pos: [x, 0.0, z], normal: [0.0, 1.0, 0.0], color: [1.0, 1.0, 1.0] });
    }
    i.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    // A wider ground around it.
    let base = v.len() as u16;
    for (x, z) in [(-6.0, 6.0), (6.0, 6.0), (6.0, -6.0), (-6.0, -6.0)] {
        v.push(Vertex { pos: [x, -0.01, z], normal: [0.0, 1.0, 0.0], color: [1.0, 1.0, 1.0] });
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
    let u = |n: &str| shader.uniform(n).unwrap();
    let (u_proj, u_mv, u_light, u_tint, u_ambient) = (u("projection"), u("modelview"), u("light_dir"), u("tint"), u("ambient"));
    let layout = VertexLayout::new().attrib(0, AttribType::Float, 3).attrib(1, AttribType::Float, 3).attrib(2, AttribType::Float, 3);
    let (v, i) = geometry();
    let vertices = LinearBuffer::from_slice(&v)?;
    let cube = LinearBuffer::from_slice(&i[..36])?;
    let mirror = LinearBuffer::from_slice(&i[36..42])?;
    let ground = LinearBuffer::from_slice(&i[42..])?;
    log!("stencil: mirror floor and outline; A toggles the outline");

    let projection = Mat4::perspective_tilt(math::to_radians(55.0), 400.0 / 240.0, 0.1, 50.0);
    let view = Mat4::look_at(Vec3::new(0.0, 2.2, 4.2), Vec3::new(0.0, 0.3, 0.0), Vec3::new(0.0, 1.0, 0.0));
    let light = (view * Vec3::new(-0.4, 0.8, 0.5).normalize().extend(0.0)).xyz();
    let mut outline = true;
    let mut frame = 0u32;
    while apt.main_loop() {
        let down = input.scan().down();
        if down.contains(Buttons::START) {
            break;
        }
        if down.contains(Buttons::A) {
            outline = !outline;
            log!("outline {}", if outline { "on" } else { "off" });
        }
        let t = frame as f32 / 60.0;
        let model = Mat4::translation(0.0, 0.8 + 0.15 * math::sin(t * 2.0), 0.0) * Mat4::rotation_y(t) * Mat4::rotation_x(0.5);
        let mirrored = Mat4::scaling(1.0, -1.0, 1.0) * model;

        // Depth 0, stencil 0.
        gpu.clear(&top, gpu::rgba(20, 22, 34, 255), 0)?;
        gpu.bind_target(&top);
        gpu.bind_shader(&shader);
        gpu.set_layout(&layout);
        gpu.bind_vertices(&vertices);
        gpu.set_texenv(0, TexEnv::replace(Source::PrimaryColor));
        gpu.set_mat4(u_proj, &projection);
        gpu.set_vec4(u_light, [light.x, light.y, light.z, 0.0]);
        gpu.set_vec4(u_ambient, [0.35, 0.0, 0.0, 0.0]);
        gpu.set_blend(None);

        // The ground (not a mirror).
        gpu.set_stencil_test(false, TestFunc::Always, 0, 0xFF, 0);
        gpu.set_depth_test(true, TestFunc::Greater, WriteMask::ALL);
        gpu.set_cull(Cull::BackCcw);
        gpu.set_vec4(u_tint, [0.25, 0.27, 0.32, 1.0]);
        gpu.set_mat4(u_mv, &view);
        gpu.draw_elements(Primitive::Triangles, &ground, 6);

        // 1. The mirror into the stencil buffer only (value 1).
        gpu.set_stencil_test(true, TestFunc::Always, 1, 0xFF, 0xFF);
        gpu.set_stencil_op(StencilOp::Keep, StencilOp::Keep, StencilOp::Replace);
        gpu.set_depth_test(true, TestFunc::GreaterEqual, WriteMask::NONE);
        gpu.draw_elements(Primitive::Triangles, &mirror, 6);

        // 2. The reflection where the stencil is 1; mirroring reverses the
        // winding, so the other faces are culled.
        gpu.set_stencil_test(true, TestFunc::Equal, 1, 0xFF, 0);
        gpu.set_stencil_op(StencilOp::Keep, StencilOp::Keep, StencilOp::Keep);
        gpu.set_depth_test(true, TestFunc::Always, WriteMask::COLOR);
        gpu.set_cull(Cull::FrontCcw);
        gpu.set_vec4(u_tint, [1.0, 1.0, 1.0, 1.0]);
        gpu.set_mat4(u_mv, &(view * mirrored));
        gpu.draw_elements(Primitive::Triangles, &cube, 36);

        // 3. The mirror's surface, blended over the reflection.
        gpu.set_stencil_test(false, TestFunc::Always, 0, 0xFF, 0);
        gpu.set_cull(Cull::BackCcw);
        gpu.set_blend(Some(Blend::ALPHA));
        gpu.set_depth_test(true, TestFunc::GreaterEqual, WriteMask::ALL);
        gpu.set_vec4(u_tint, [0.5, 0.6, 0.8, 0.45]);
        gpu.set_mat4(u_mv, &view);
        gpu.draw_elements(Primitive::Triangles, &mirror, 6);
        gpu.set_blend(None);

        // 4. The cube, marking its pixels with 2.
        gpu.set_stencil_test(true, TestFunc::Always, 2, 0xFF, 0xFF);
        gpu.set_stencil_op(StencilOp::Keep, StencilOp::Keep, StencilOp::Replace);
        gpu.set_depth_test(true, TestFunc::Greater, WriteMask::ALL);
        gpu.set_vec4(u_tint, [1.0, 1.0, 1.0, 1.0]);
        gpu.set_mat4(u_mv, &(view * model));
        gpu.draw_elements(Primitive::Triangles, &cube, 36);

        // 5. The outline: a larger flat cube where the stencil is not 2.
        if outline {
            gpu.set_stencil_test(true, TestFunc::NotEqual, 2, 0xFF, 0);
            gpu.set_stencil_op(StencilOp::Keep, StencilOp::Keep, StencilOp::Keep);
            gpu.set_depth_test(false, TestFunc::Always, WriteMask::COLOR);
            gpu.set_vec4(u_tint, [1.0, 1.0, 1.0, 1.0]);
            gpu.set_vec4(u_ambient, [10.0, 0.0, 0.0, 0.0]);
            gpu.set_mat4(u_mv, &(view * model * Mat4::scaling(1.12, 1.12, 1.12)));
            gpu.draw_elements(Primitive::Triangles, &cube, 36);
        }
        gpu.set_stencil_test(false, TestFunc::Always, 0, 0xFF, 0);
        screens.show_target(&mut gpu, &top, Screen::Top)?;

        if frame == 30 || frame == 80 {
            let c = screens.top();
            // Below the cube, inside the mirror: the reflection's colours
            // (not the mirror's plain blue-grey).
            let mut coloured = 0;
            for y in (175..215).step_by(3) {
                for x in (170..230).step_by(3) {
                    let p = c.get(x, y);
                    let spread = p.r.max(p.g).max(p.b) - p.r.min(p.g).min(p.b);
                    if spread > 60 {
                        coloured += 1;
                    }
                }
            }
            if frame == 30 {
                log!("reflection {} ({} saturated samples)", if coloured > 10 { "ok" } else { "wrong" }, coloured);
            }
            // The outline is pure white, which no face of the cube is.
            let mut white = 0;
            for y in (30..140).step_by(2) {
                for x in (130..270).step_by(2) {
                    let p = c.get(x, y);
                    if p.r > 245 && p.g > 245 && p.b > 245 {
                        white += 1;
                    }
                }
            }
            if frame == 30 {
                log!("outline {} ({} white samples)", if white > 20 { "ok" } else { "wrong" }, white);
            } else {
                log!("outline gone {} ({} white samples)", if white == 0 { "ok" } else { "wrong" }, white);
            }
        }
        let mut b = screens.bottom();
        b.clear(Color::rgb(16, 16, 24));
        b.text(8, 8, "Stencil", Color::WHITE, 2);
        b.text(8, 36, "mirror: floor -> stencil 1, reflection where 1", Color::rgb(200, 210, 230), 1);
        b.text(8, 50, "outline: cube -> stencil 2, larger copy where not 2", Color::rgb(200, 210, 230), 1);
        b.text(8, 220, "A: outline  START: exit", Color::WHITE, 1);
        screens.present();
        frame += 1;
    }
    log!("bye: {} frames", frame);
    Ok(())
}
