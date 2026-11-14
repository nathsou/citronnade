//! A particle fountain. Each particle is a single point in the vertex
//! buffer; the vertex shader moves it into view space and a geometry shader
//! turns it into a quad facing the camera (four emitted vertices, two
//! triangles), projected and textured with a soft dot. The floor uses a plain
//! vertex shader in the same frame. START exits.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use alloc::vec::Vec;
use citronnade::gpu::{
    self, shader, AttribType, Blend, BlendFactor, Cull, Gpu, LinearBuffer, Primitive, RenderTarget, Shader, Source,
    TexEnv, TexFormat, TestFunc, Texture, VertexLayout, WriteMask,
};
use citronnade::math::{self, Mat4, Vec3};
use citronnade::prelude::*;

/// Particles: to view space, colour from bytes, size passed through.
const PARTICLE_VS: &[u8] = shader!(
    r#"
    .fvec modelview[4]
    .constf consts(0.0, 1.0, 0.0039215686, 0.0)
    .alias inpos v0
    .alias inclr v1
    .alias insize v2
    .out outview position
    .out outclr color
    .out outsize texcoord0

    .proc main
        mov r0.xyz, inpos
        mov r0.w, consts.y
        dp4 outview.x, modelview[0], r0
        dp4 outview.y, modelview[1], r0
        dp4 outview.z, modelview[2], r0
        dp4 outview.w, modelview[3], r0
        mul outclr, consts.zzzz, inclr
        mov outsize, insize
        end
    .end
    "#
);

/// One point in, one quad (two triangles) out.
const PARTICLE_GS: &[u8] = shader!(
    r#"
    .gsh point c0
    .fvec projection[4]
    ; corner offsets (x, y) and texture coordinates (u, v)
    .constf bottom_left(-1.0, -1.0, 0.0, 0.0)
    .constf bottom_right(1.0, -1.0, 1.0, 0.0)
    .constf top_left(-1.0, 1.0, 0.0, 1.0)
    .constf top_right(1.0, 1.0, 1.0, 1.0)
    .out outpos position
    .out outclr color
    .out outtc texcoord0
    .alias inpos v0
    .alias inclr v1
    .alias insize v2

    .proc main
        setemit 0
        mov r1, bottom_left
        call corner
        setemit 1
        mov r1, bottom_right
        call corner
        setemit 2, prim
        mov r1, top_left
        call corner
        ; second triangle: slots now hold top right, bottom right, top left
        setemit 0, prim inv
        mov r1, top_right
        call corner
        end
    .end

    ; emits the corner whose offset and uv are in r1
    .proc corner
        mov r0, inpos
        mad r0.xy, r1.xy, insize.xx, r0.xy
        dp4 outpos.x, projection[0], r0
        dp4 outpos.y, projection[1], r0
        dp4 outpos.z, projection[2], r0
        dp4 outpos.w, projection[3], r0
        mov outclr, inclr
        mov outtc, r1.zwzw
        emit
    .end
    "#
);

/// The floor: position and colour.
const FLOOR_VS: &[u8] = shader!(
    r#"
    .fvec projection[4], modelview[4]
    .constf ones(1.0, 1.0, 1.0, 1.0)
    .alias inpos v0
    .alias inclr v1
    .out outpos position
    .out outclr color
    .proc main
        mov r0.xyz, inpos
        mov r0.w, ones
        dp4 r1.x, modelview[0], r0
        dp4 r1.y, modelview[1], r0
        dp4 r1.z, modelview[2], r0
        dp4 r1.w, modelview[3], r0
        dp4 outpos.x, projection[0], r1
        dp4 outpos.y, projection[1], r1
        dp4 outpos.z, projection[2], r1
        dp4 outpos.w, projection[3], r1
        mov outclr, inclr
        end
    .end
    "#
);

#[derive(Clone, Copy, Default)]
#[repr(C)]
struct Point {
    pos: [f32; 3],
    color: [u8; 4],
    size: f32,
}

#[derive(Clone, Copy)]
#[repr(C)]
struct FloorVertex {
    pos: [f32; 3],
    color: [f32; 3],
}

#[derive(Clone, Copy)]
struct Particle {
    pos: Vec3,
    vel: Vec3,
    age: f32,
    life: f32,
}

struct Rng(u32);

impl Rng {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        (self.0 % 100_000) as f32 / 100_000.0
    }
}

const COUNT: usize = 600;

fn spawn(rng: &mut Rng) -> Particle {
    let a = rng.next() * math::TAU;
    let spread = rng.next() * 0.9;
    Particle {
        pos: Vec3::new(0.0, -0.9, 0.0),
        vel: Vec3::new(math::cos(a) * spread, 4.0 + rng.next() * 1.5, math::sin(a) * spread),
        age: 0.0,
        life: 1.2 + rng.next() * 0.8,
    }
}

#[citronnade::main]
fn main(sys: System) -> Result<()> {
    let System { apt, gsp, hid, .. } = sys;
    let mut screens = gsp.screens()?;
    let mut input = hid.input()?;
    let mut gpu = Gpu::new()?;
    let top = RenderTarget::top()?;
    let particle_vs = Shader::new(PARTICLE_VS).expect("vertex shader");
    let particle_gs = Shader::new(PARTICLE_GS).expect("geometry shader");
    let floor_vs = Shader::new(FLOOR_VS).expect("floor shader");
    let u_modelview = particle_vs.uniform("modelview").unwrap();
    let u_projection = particle_gs.uniform("projection").unwrap();
    let u_floor_projection = floor_vs.uniform("projection").unwrap();
    let u_floor_modelview = floor_vs.uniform("modelview").unwrap();
    log!(
        "geometry-shader: vertex shader {} instructions, geometry shader {} ({} particles)",
        particle_vs.program().code.len(),
        particle_gs.program().code.len(),
        COUNT
    );

    let point_layout = VertexLayout::new().attrib(0, AttribType::Float, 3).attrib(1, AttribType::UByte, 4).attrib(2, AttribType::Float, 1);
    let floor_layout = VertexLayout::new().attrib(0, AttribType::Float, 3).attrib(1, AttribType::Float, 3);
    let mut points: LinearBuffer<Point> = LinearBuffer::zeroed(COUNT)?;
    let mut floor = Vec::new();
    for z in 0..10 {
        for x in 0..10 {
            let c = if (x + z) % 2 == 0 { [0.25, 0.27, 0.35] } else { [0.15, 0.16, 0.22] };
            let (x0, z0) = (-5.0 + x as f32, -5.0 + z as f32);
            for (dx, dz) in [(0.0, 0.0), (0.0, 1.0), (1.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)] {
                floor.push(FloorVertex { pos: [x0 + dx, -1.0, z0 + dz], color: c });
            }
        }
    }
    let floor = LinearBuffer::from_slice(&floor)?;

    // A soft white dot.
    let mut dot = Vec::with_capacity(32 * 32 * 4);
    for y in 0..32 {
        for x in 0..32 {
            let (dx, dy) = (x as f32 - 15.5, y as f32 - 15.5);
            let a = (1.0 - math::sqrt(dx * dx + dy * dy) / 15.5).max(0.0);
            dot.extend_from_slice(&[255, 255, 255, (a * a * 255.0) as u8]);
        }
    }
    let dot = Texture::from_rgba(32, 32, &dot, TexFormat::Rgba8)?;

    let mut rng = Rng(0xC0FFEE);
    let mut particles: Vec<Particle> = (0..COUNT)
        .map(|i| {
            let mut p = spawn(&mut rng);
            p.age = p.life * i as f32 / COUNT as f32;
            p
        })
        .collect();
    let projection = Mat4::perspective_tilt(math::to_radians(55.0), 400.0 / 240.0, 0.1, 50.0);
    let additive = Blend { src_color: BlendFactor::SrcAlpha, dst_color: BlendFactor::One, ..Blend::ALPHA };

    let mut frame = 0u32;
    while apt.main_loop() {
        if input.scan().down().contains(Buttons::START) {
            break;
        }
        let dt = 1.0 / 60.0;
        let t = frame as f32 * dt;
        for p in particles.iter_mut() {
            p.age += dt;
            if p.age > p.life {
                *p = spawn(&mut rng);
            }
            p.vel.y -= 5.0 * dt;
            p.pos = p.pos + p.vel * dt;
        }
        for (p, out) in particles.iter().zip(points.iter_mut()) {
            let k = p.age / p.life;
            let c = Color::hsv((200.0 + 160.0 * k) as u16 % 360, 200, 255);
            *out = Point { pos: [p.pos.x, p.pos.y, p.pos.z], color: [c.r, c.g, c.b, ((1.0 - k) * 255.0) as u8], size: 0.08 + 0.1 * k };
        }

        let eye = Vec3::new(4.5 * math::sin(t * 0.3), 1.2, 4.5 * math::cos(t * 0.3));
        let view = Mat4::look_at(eye, Vec3::new(0.0, 0.6, 0.0), Vec3::new(0.0, 1.0, 0.0));

        gpu.clear(&top, gpu::rgba(6, 8, 16, 255), 0)?;
        gpu.bind_target(&top);
        // The floor, with the plain vertex shader.
        gpu.bind_shader(&floor_vs);
        gpu.set_layout(&floor_layout);
        gpu.bind_vertices(&floor);
        gpu.bind_texture(0, None);
        gpu.set_texenv(0, TexEnv::replace(Source::PrimaryColor));
        gpu.set_depth_test(true, TestFunc::Greater, WriteMask::ALL);
        gpu.set_cull(Cull::None);
        gpu.set_blend(None);
        gpu.set_mat4(u_floor_projection, &projection);
        gpu.set_mat4(u_floor_modelview, &view);
        gpu.draw_arrays(Primitive::Triangles, 0, 600);
        // The particles: points through both shaders, blended additively,
        // depth-tested but not written so they do not hide each other.
        gpu.bind_program(&particle_vs, Some(&particle_gs));
        gpu.set_layout(&point_layout);
        gpu.bind_vertices(&points);
        gpu.bind_texture(0, Some(&dot));
        gpu.set_texenv(0, TexEnv::modulate(Source::Texture0, Source::PrimaryColor));
        gpu.set_depth_test(true, TestFunc::Greater, WriteMask::COLOR);
        gpu.set_blend(Some(additive));
        gpu.set_mat4(u_modelview, &view);
        gpu.set_mat4(u_projection, &projection);
        gpu.draw_arrays(Primitive::Geometry, 0, COUNT as u32);
        screens.show_target(&mut gpu, &top, Screen::Top)?;

        if frame == 50 {
            let c = screens.top();
            // The fountain's column is bright; the floor's corner is the
            // floor's colour.
            let (mut bright, mut n) = (0, 0);
            for y in (40..160).step_by(4) {
                for x in (180..220).step_by(4) {
                    let p = c.get(x, y);
                    n += 1;
                    if p.r as u32 + p.g as u32 + p.b as u32 > 200 {
                        bright += 1;
                    }
                }
            }
            log!("particles {} ({} of {} samples bright)", if bright > 5 { "drawn" } else { "missing" }, bright, n);
            let f = c.get(30, 225);
            log!("floor {} {:?}", if f.b > 20 && f.b > f.r { "drawn" } else { "missing" }, (f.r, f.g, f.b));
        }

        let mut b = screens.bottom();
        b.clear(Color::rgb(16, 16, 24));
        b.text(8, 8, "Geometry shader", Color::WHITE, 2);
        b.text(8, 36, &format!("{} points in, {} triangles out", COUNT, COUNT * 2), Color::rgb(200, 210, 230), 1);
        b.text(8, 50, "each point becomes a camera-facing quad", Color::rgb(200, 210, 230), 1);
        b.text(8, 220, "START: exit", Color::WHITE, 1);
        screens.present();
        frame += 1;
    }
    log!("bye: {} frames", frame);
    Ok(())
}
