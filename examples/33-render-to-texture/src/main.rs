//! Render to texture: every frame the town is drawn from above into a
//! 256x256 offscreen target, which then serves as a texture: the minimap on
//! the bottom screen and an inset on the top screen, where the town is seen
//! from behind the player. The red tower stands in the north-west corner,
//! the blue one in the south-east. Move with the Circle Pad; START exits.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use alloc::vec::Vec;
use citronnade::gpu::{
    self, shader, AttribType, ColorFormat, Cull, DepthFormat, Filter, Gpu, LinearBuffer, Primitive, RenderTarget, Shader, Source,
    TexEnv, TestFunc, VertexLayout, Wrap, WriteMask,
};
use citronnade::math::{self, Mat4, Vec3};
use citronnade::prelude::*;

const LIT: &[u8] = shader!(
    r#"
    .fvec projection[4], modelview[4]
    .fvec light_dir
    .fvec tint
    .constf consts(0.0, 1.0, 0.4, 0.0)
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

const FLAT: &[u8] = shader!(
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
    pos: [f32; 3],
    normal: [f32; 3],
}

#[derive(Clone, Copy)]
#[repr(C)]
struct FlatVertex {
    pos: [f32; 2],
    uv: [f32; 2],
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

/// Buildings: (x, z, height, colour); the two towers mark the corners.
fn town() -> Vec<(f32, f32, f32, [f32; 3])> {
    let mut b = Vec::new();
    let mut seed = 7u32;
    for gz in -3..=3 {
        for gx in -3..=3 {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
            if (gx + gz) % 2 != 0 || (gx == 0 && gz == 0) || seed % 3 == 0 {
                continue;
            }
            let h = 0.6 + (seed >> 16) as f32 % 100.0 / 40.0;
            let shade = 0.5 + ((seed >> 8) % 40) as f32 / 100.0;
            b.push((gx as f32 * 2.0, gz as f32 * 2.0, h, [shade, shade * 0.95, shade * 0.85]));
        }
    }
    b.push((-7.0, -7.0, 4.0, [0.95, 0.2, 0.2]));
    b.push((7.0, 7.0, 4.0, [0.2, 0.35, 0.95]));
    b
}

#[citronnade::main]
fn main(sys: System) -> Result<()> {
    let System { apt, gsp, hid, .. } = sys;
    let mut screens = gsp.screens()?;
    let mut input = hid.input()?;
    let mut gpu = Gpu::new()?;
    let top = RenderTarget::top()?;
    let bottom = RenderTarget::bottom()?;
    let map = RenderTarget::new(256, 256, ColorFormat::Rgba8, Some(DepthFormat::Depth24Stencil8))?;
    let lit = Shader::new(LIT).expect("lit shader");
    let flat = Shader::new(FLAT).expect("flat shader");
    let (u_proj, u_mv, u_light, u_tint) =
        (lit.uniform("projection").unwrap(), lit.uniform("modelview").unwrap(), lit.uniform("light_dir").unwrap(), lit.uniform("tint").unwrap());
    let u_flat_proj = flat.uniform("projection").unwrap();
    let layout = VertexLayout::new().attrib(0, AttribType::Float, 3).attrib(1, AttribType::Float, 3);
    let flat_layout = VertexLayout::new().attrib(0, AttribType::Float, 2).attrib(1, AttribType::Float, 2);
    let (v, i) = cube();
    let vertices = LinearBuffer::from_slice(&v)?;
    let indices = LinearBuffer::from_slice(&i)?;
    let buildings = town();
    // Quads (triangle strips): the bottom minimap, the top inset.
    let quad = |x: f32, y: f32, s: f32| {
        [
            FlatVertex { pos: [x, y + s], uv: [0.0, 0.0] },
            FlatVertex { pos: [x + s, y + s], uv: [1.0, 0.0] },
            FlatVertex { pos: [x, y], uv: [0.0, 1.0] },
            FlatVertex { pos: [x + s, y], uv: [1.0, 1.0] },
        ]
    };
    let mut quads = Vec::new();
    quads.extend_from_slice(&quad(50.0, 10.0, 220.0));
    quads.extend_from_slice(&quad(300.0, 8.0, 92.0));
    let quads = LinearBuffer::from_slice(&quads)?;
    log!("render-to-texture: {} buildings, minimap target 256x256", buildings.len());

    let (mut px, mut pz, mut heading) = (0.0f32, 2.0f32, 0.0f32);
    let mut frame = 0u32;
    while apt.main_loop() {
        let state = input.scan();
        if state.down().contains(Buttons::START) {
            break;
        }
        let (sx, sy) = state.circle_pad();
        heading -= sx as f32 / 156.0 * 0.04;
        let speed = sy as f32 / 156.0 * 0.08;
        px = (px - math::sin(heading) * speed).clamp(-8.0, 8.0);
        pz = (pz - math::cos(heading) * speed).clamp(-8.0, 8.0);

        // Draws the town (and the player) with a projection and view.
        let draw_town = |gpu: &mut Gpu, projection: &Mat4, view: &Mat4| {
            gpu.bind_shader(&lit);
            gpu.set_layout(&layout);
            gpu.bind_vertices(&vertices);
            gpu.bind_texture(0, None);
            gpu.set_texenv(0, TexEnv::replace(Source::PrimaryColor));
            gpu.set_depth_test(true, TestFunc::Greater, WriteMask::ALL);
            gpu.set_cull(Cull::BackCcw);
            gpu.set_blend(None);
            gpu.set_mat4(u_proj, projection);
            let l = (*view * Vec3::new(0.3, 0.9, 0.4).normalize().extend(0.0)).xyz();
            gpu.set_vec4(u_light, [l.x, l.y, l.z, 0.0]);
            gpu.set_vec4(u_tint, [0.3, 0.5, 0.3, 1.0]);
            gpu.set_mat4(u_mv, &(*view * Mat4::translation(0.0, -0.05, 0.0) * Mat4::scaling(18.0, 0.1, 18.0)));
            gpu.draw_elements(Primitive::Triangles, &indices, 36);
            for &(x, z, h, c) in &buildings {
                gpu.set_vec4(u_tint, [c[0], c[1], c[2], 1.0]);
                gpu.set_mat4(u_mv, &(*view * Mat4::translation(x, h / 2.0, z) * Mat4::scaling(1.4, h, 1.4)));
                gpu.draw_elements(Primitive::Triangles, &indices, 36);
            }
            gpu.set_vec4(u_tint, [1.0, 0.9, 0.2, 1.0]);
            gpu.set_mat4(u_mv, &(*view * Mat4::translation(px, 0.3, pz) * Mat4::rotation_y(heading) * Mat4::scaling(0.5, 0.6, 0.8)));
            gpu.draw_elements(Primitive::Triangles, &indices, 36);
        };

        // 1. The map: from straight above, north (-z) up, untilted.
        gpu.clear(&map, gpu::rgba(20, 30, 20, 255), 0)?;
        gpu.bind_target(&map);
        let map_view = Mat4::look_at(Vec3::new(0.0, 20.0, 0.0), Vec3::ZERO, Vec3::new(0.0, 0.0, -1.0));
        draw_town(&mut gpu, &Mat4::ortho(-9.0, 9.0, -9.0, 9.0, 1.0, 40.0), &map_view);

        // 2. The top screen: behind the player, with the map as an inset.
        gpu.clear(&top, gpu::rgba(120, 160, 210, 255), 0)?;
        gpu.bind_target(&top);
        let eye = Vec3::new(px + math::sin(heading) * 4.0, 2.5, pz + math::cos(heading) * 4.0);
        let view = Mat4::look_at(eye, Vec3::new(px, 0.5, pz), Vec3::new(0.0, 1.0, 0.0));
        draw_town(&mut gpu, &Mat4::perspective_tilt(math::to_radians(60.0), 400.0 / 240.0, 0.1, 60.0), &view);
        let draw_map = |gpu: &mut Gpu, width: f32, first: u32| {
            gpu.bind_shader(&flat);
            gpu.set_layout(&flat_layout);
            gpu.bind_vertices(&quads);
            gpu.bind_target_texture(0, &map, Filter::Linear, Wrap::ClampToEdge);
            gpu.set_texenv(0, TexEnv::replace(Source::Texture0));
            gpu.set_depth_test(false, TestFunc::Always, WriteMask::COLOR);
            gpu.set_cull(Cull::None);
            gpu.set_mat4(u_flat_proj, &Mat4::screen_2d(width, 240.0));
            gpu.draw_arrays(Primitive::TriangleStrip, first, 4);
        };
        draw_map(&mut gpu, 400.0, 4);
        screens.show_target(&mut gpu, &top, Screen::Top)?;

        // 3. The bottom screen: the map.
        gpu.clear(&bottom, gpu::rgba(16, 16, 24, 255), 0)?;
        gpu.bind_target(&bottom);
        draw_map(&mut gpu, 320.0, 0);
        screens.show_target(&mut gpu, &bottom, Screen::Bottom)?;

        if frame == 30 {
            // The map spans x 50..270, y 10..230 on the bottom screen: the red
            // tower near the top left, the blue one near the bottom right.
            let c = screens.bottom();
            let near = |x: i32, y: i32, want: fn(Color) -> bool| {
                (-6..=6).step_by(3).any(|dx| (-6..=6).step_by(3).any(|dy| want(c.get(x + dx, y + dy))))
            };
            let red = |p: Color| p.r > 150 && p.g < 90 && p.b < 90;
            let blue = |p: Color| p.b > 150 && p.r < 90;
            let ok = near(50 + 25, 10 + 25, red) && near(270 - 25, 230 - 25, blue);
            log!("minimap orientation {}", if ok { "ok" } else { "wrong" });
        }
        let mut c = screens.bottom();
        c.text(8, 232 - 8, &format!("({:4.1}, {:4.1})", px, pz), Color::WHITE, 1);
        screens.present();
        frame += 1;
    }
    log!("bye: {} frames", frame);
    Ok(())
}
