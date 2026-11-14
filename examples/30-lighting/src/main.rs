//! Fragment lighting: the GPU lights every pixel from interpolated normals.
//! A sphere and a floor are lit by a directional key light, a spotlight
//! sweeping over the floor and a green point light that fades with
//! distance. The specular highlight's shape comes from a lookup table (D0)
//! that you draw on the touch screen.
//!
//! A toggles the spotlight, B the point light, X makes a toon-like LUT, Y
//! restores the Phong one. START exits.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use alloc::vec::Vec;
use citronnade::gpu::{
    self, shader, AttribType, Combine, Cull, DistanceAttenuation, Gpu, Light, LightEnv, LightLut, LinearBuffer, LutId, LutInput,
    Material, Primitive, RenderTarget, Shader, Source, TexEnv, TestFunc, VertexLayout, WriteMask,
};
use citronnade::math::{self, Mat4, Vec3};
use citronnade::prelude::*;

const SHADER: &[u8] = shader!(
    r#"
    .fvec projection[4], modelview[4]
    .constf myconst(0.0, 1.0, -1.0, 0.5)
    .alias zeros myconst.xxxx
    .alias ones myconst.yyyy
    .alias half myconst.wwww
    .alias inpos v0
    .alias innrm v1
    .alias inclr v2
    .out outpos position
    .out outclr color
    .out outview view
    .out outnq normalquat

    .proc main
        mov r0.xyz, inpos
        mov r0.w, ones
        ; r1 = view-space position
        dp4 r1.x, modelview[0], r0
        dp4 r1.y, modelview[1], r0
        dp4 r1.z, modelview[2], r0
        dp4 r1.w, modelview[3], r0
        ; the view vector points from the surface to the eye
        mov outview, -r1
        dp4 outpos.x, projection[0], r1
        dp4 outpos.y, projection[1], r1
        dp4 outpos.z, projection[2], r1
        dp4 outpos.w, projection[3], r1
        ; r14 = normalised view-space normal
        dp3 r14.x, modelview[0], innrm
        dp3 r14.y, modelview[1], innrm
        dp3 r14.z, modelview[2], innrm
        dp3 r6.x, r14, r14
        rsq r6.x, r6.x
        mul r14.xyz, r14.xyz, r6.x
        ; the quaternion rotating +z onto the normal; (1, 0, 0, 0) when the
        ; normal is -z
        mov r0, myconst.yxxx
        add r4, ones, r14.z
        mul r4, half, r4
        cmp zeros, ge, ge, r4.x
        rsq r4, r4.x
        mul r5, half, r14
        jmpc cmp.x, degenerate
        rcp r0.z, r4.x
        mul r0.xy, r5, r4
    degenerate:
        mov outnq, r0
        mov outclr, inclr
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

/// A UV sphere of radius 1 and a floor at y = -1.
fn geometry() -> (Vec<Vertex>, Vec<u16>, u32) {
    let (slices, stacks) = (32, 20);
    let mut v = Vec::new();
    let mut i = Vec::new();
    for s in 0..=stacks {
        let phi = math::PI * s as f32 / stacks as f32;
        for k in 0..=slices {
            let theta = 2.0 * math::PI * k as f32 / slices as f32;
            let n = [math::sin(phi) * math::cos(theta), math::cos(phi), math::sin(phi) * math::sin(theta)];
            v.push(Vertex { pos: n, normal: n, color: [0.9, 0.85, 0.8] });
        }
    }
    for s in 0..stacks {
        for k in 0..slices {
            let a = (s * (slices + 1) + k) as u16;
            let b = a + slices as u16 + 1;
            i.extend_from_slice(&[a, a + 1, b, b, a + 1, b + 1]);
        }
    }
    let sphere = i.len() as u32;
    // The floor, finely divided so the spotlight's edge stays round.
    let n = 24;
    let base = v.len() as u16;
    for z in 0..=n {
        for x in 0..=n {
            let p = [-6.0 + 12.0 * x as f32 / n as f32, -1.0, -6.0 + 12.0 * z as f32 / n as f32];
            v.push(Vertex { pos: p, normal: [0.0, 1.0, 0.0], color: [0.6, 0.65, 0.75] });
        }
    }
    for z in 0..n {
        for x in 0..n {
            let a = base + (z * (n + 1) + x) as u16;
            let b = a + n as u16 + 1;
            i.extend_from_slice(&[a, b, a + 1, a + 1, b, b + 1]);
        }
    }
    (v, i, sphere)
}

fn phong_curve(shininess: f32) -> [f32; 256] {
    core::array::from_fn(|i| math::pow(i as f32 / 255.0, shininess))
}

fn lut_from_curve(curve: &[f32; 256]) -> LightLut {
    LightLut::from_fn(|x| curve[((x * 255.0) as usize).min(255)], false)
}

const GRAPH_X: i32 = 32;
const GRAPH_Y: i32 = 50;
const GRAPH_H: i32 = 140;

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
    let layout = VertexLayout::new().attrib(0, AttribType::Float, 3).attrib(1, AttribType::Float, 3).attrib(2, AttribType::Float, 3);
    let (verts, idx, sphere_count) = geometry();
    let vertices = LinearBuffer::from_slice(&verts)?;
    let sphere_indices = LinearBuffer::from_slice(&idx[..sphere_count as usize])?;
    let floor_indices = LinearBuffer::from_slice(&idx[sphere_count as usize..])?;
    log!("lighting: sphere and floor, {} vertices, {} triangles", verts.len(), idx.len() / 3);

    let mut curve = phong_curve(24.0);
    let mut env = LightEnv::new();
    env.material = Material { ambient: [0.15; 3], diffuse: [0.85; 3], specular0: [0.7; 3], specular1: [0.0; 3], emission: [0.0; 3] };
    env.set_lut(LutId::D0, LutInput::NH, false, Some(lut_from_curve(&curve)));
    let (mut spot_on, mut point_on) = (true, true);

    // Diffuse x vertex colour, plus specular.
    let stage0 = TexEnv::modulate(Source::FragmentPrimaryColor, Source::PrimaryColor);
    let stage1 = TexEnv::PASS.rgb_sources(Source::Previous, Source::FragmentSecondaryColor, Source::PrimaryColor).rgb_func(Combine::Add);

    let projection = Mat4::perspective_tilt(math::to_radians(55.0), 400.0 / 240.0, 0.1, 50.0);
    let view = Mat4::look_at(Vec3::new(0.0, 1.6, 4.8), Vec3::new(0.0, -0.3, 0.0), Vec3::new(0.0, 1.0, 0.0));
    let to_view_dir = |d: Vec3| (view * d.extend(0.0)).xyz();
    let to_view_pos = |p: Vec3| (view * p.extend(1.0)).xyz();

    let mut frame = 0u32;
    let mut edited = false;
    let mut logged_edit = false;
    while apt.main_loop() {
        let state = input.scan();
        let down = state.down();
        if down.contains(Buttons::START) {
            break;
        }
        if down.contains(Buttons::A) {
            spot_on = !spot_on;
            log!("spotlight {}", if spot_on { "on" } else { "off" });
        }
        if down.contains(Buttons::B) {
            point_on = !point_on;
            log!("point light {}", if point_on { "on" } else { "off" });
        }
        if down.contains(Buttons::X) {
            curve = core::array::from_fn(|i| if i > 230 { 1.0 } else if i > 180 { 0.3 } else { 0.0 });
            edited = true;
        }
        if down.contains(Buttons::Y) {
            curve = phong_curve(24.0);
            edited = true;
        }
        if let Some((tx, ty)) = state.touch() {
            let x = tx as i32 - GRAPH_X;
            if (0..256).contains(&x) {
                let v = (1.0 - (ty as i32 - GRAPH_Y) as f32 / GRAPH_H as f32).clamp(0.0, 1.0);
                // A soft brush over neighbouring entries.
                for dx in -6..=6i32 {
                    let i = x + dx;
                    if (0..256).contains(&i) {
                        let w = 1.0 - dx.abs() as f32 / 7.0;
                        let c = &mut curve[i as usize];
                        *c += (v - *c) * w;
                    }
                }
                if !logged_edit {
                    log!("lut edited at entry {}", x);
                    logged_edit = true;
                }
                edited = true;
            }
        }
        if edited {
            env.set_lut(LutId::D0, LutInput::NH, false, Some(lut_from_curve(&curve)));
            edited = false;
        }

        let t = frame as f32 / 60.0;
        env.lights.clear();
        env.lights.push(Light::directional(to_view_dir(Vec3::new(-0.6, 0.8, 0.5)), [0.55, 0.6, 0.75]));
        if spot_on {
            let pos = Vec3::new(2.5 * math::cos(t * 0.8), 3.5, 2.5 * math::sin(t * 0.8));
            let target = Vec3::new(1.6 * math::cos(t * 0.8 + 1.2), -1.0, 1.6 * math::sin(t * 0.8 + 1.2));
            let mut spot = Light::point(to_view_pos(pos), [1.0, 0.75, 0.35]);
            spot.spot = Some((to_view_dir((target - pos).normalize()), LightLut::soft_spotlight(0.25, 0.4)));
            env.lights.push(spot);
        }
        if point_on {
            let pos = Vec3::new(1.6 * math::sin(t * 1.7), 0.2, 1.6 * math::cos(t * 1.7));
            let mut point = Light::point(to_view_pos(pos), [0.3, 1.0, 0.45]);
            point.specular0 = [0.3, 1.0, 0.45];
            point.distance_attenuation = Some(DistanceAttenuation::quadratic(0.0, 8.0, 0.2, 0.8));
            env.lights.push(point);
        }

        gpu.clear(&top, gpu::rgba(10, 12, 20, 255), 0)?;
        gpu.bind_target(&top);
        gpu.bind_shader(&shader);
        gpu.set_layout(&layout);
        gpu.bind_vertices(&vertices);
        gpu.set_texenv(0, stage0);
        gpu.set_texenv(1, stage1);
        gpu.reset_texenv(2);
        gpu.set_lighting(Some(&env));
        gpu.set_depth_test(true, TestFunc::Greater, WriteMask::ALL);
        gpu.set_cull(Cull::BackCcw);
        gpu.set_blend(None);
        gpu.set_mat4(u_projection, &projection);
        gpu.set_mat4(u_modelview, &view);
        gpu.draw_elements(Primitive::Triangles, &floor_indices, idx.len() as u32 - sphere_count);
        gpu.set_mat4(u_modelview, &(view * Mat4::rotation_y(t * 0.3)));
        gpu.draw_elements(Primitive::Triangles, &sphere_indices, sphere_count);
        screens.show_target(&mut gpu, &top, Screen::Top)?;

        if frame == 30 {
            // The sphere covers the middle of the screen: it must be lit and
            // somewhere brighter than its average (a specular highlight).
            let c = screens.top();
            let (mut sum, mut max, mut n) = (0u32, 0u32, 0u32);
            for y in (70..170).step_by(4) {
                for x in (150..250).step_by(4) {
                    let p = c.get(x, y);
                    let l = p.r as u32 + p.g as u32 + p.b as u32;
                    sum += l;
                    max = max.max(l);
                    n += 1;
                }
            }
            let avg = sum / n;
            log!("sphere {} (average {}, brightest {})", if avg > 60 { "lit" } else { "unlit" }, avg, max);
            log!("{}", if max > avg + 120 { "highlight ok" } else { "no highlight" });
        }

        // The LUT graph.
        let mut b = screens.bottom();
        b.clear(Color::rgb(16, 16, 24));
        b.text(8, 6, "Specular LUT (D0, input N.H)", Color::WHITE, 1);
        b.text(8, 20, "draw on the graph to reshape the highlight", Color::rgb(170, 180, 200), 1);
        b.rect(GRAPH_X - 1, GRAPH_Y - 1, 258, GRAPH_H as u32 + 2, Color::rgb(70, 70, 90));
        for x in 0..256 {
            let y = GRAPH_Y + GRAPH_H - (curve[x] * GRAPH_H as f32) as i32;
            b.fill_rect(GRAPH_X + x as i32, y, 1, (GRAPH_Y + GRAPH_H - y) as u32, Color::rgb(60, 90, 140));
            b.set(GRAPH_X + x as i32, y, Color::rgb(140, 200, 255));
        }
        b.text(GRAPH_X, GRAPH_Y + GRAPH_H + 4, "0", Color::WHITE, 1);
        b.text(GRAPH_X + 250, GRAPH_Y + GRAPH_H + 4, "1", Color::WHITE, 1);
        b.text(8, 206, &format!("A: spotlight {}  B: point {}", if spot_on { "on" } else { "off" }, if point_on { "on" } else { "off" }), Color::WHITE, 1);
        b.text(8, 220, "X: toon  Y: Phong  START: exit", Color::WHITE, 1);
        screens.present();
        frame += 1;
    }
    log!("bye: {} frames", frame);
    Ok(())
}
