//! A model made for this example (a lemon, `assets/make_lemon.py`), stored
//! as OBJ, converted to a mesh by `build.rs` at build time, packed into the
//! RomFS, loaded at startup and drawn with fragment lighting, one material
//! per submesh (body, stem, leaf). The Circle Pad turns it. START exits.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use citronnade::gpu::{
    self, shader, Combine, Cull, Gpu, Light, LightEnv, LightLut, LutId, LutInput, Material, Mesh, RenderTarget, Shader, Source, TexEnv,
    TestFunc, WriteMask,
};
use citronnade::math::{self, Mat4, Vec3};
use citronnade::prelude::*;
use citronnade::romfs::RomFs;

const SHADER: &[u8] = shader!(
    r#"
    .fvec projection[4], modelview[4]
    .constf myconst(0.0, 1.0, -1.0, 0.5)
    .alias zeros myconst.xxxx
    .alias ones myconst.yyyy
    .alias half myconst.wwww
    .alias inpos v0
    .alias innrm v1
    .out outpos position
    .out outclr color
    .out outview view
    .out outnq normalquat

    .proc main
        mov r0.xyz, inpos
        mov r0.w, ones
        dp4 r1.x, modelview[0], r0
        dp4 r1.y, modelview[1], r0
        dp4 r1.z, modelview[2], r0
        dp4 r1.w, modelview[3], r0
        mov outview, -r1
        dp4 outpos.x, projection[0], r1
        dp4 outpos.y, projection[1], r1
        dp4 outpos.z, projection[2], r1
        dp4 outpos.w, projection[3], r1
        dp3 r14.x, modelview[0], innrm
        dp3 r14.y, modelview[1], innrm
        dp3 r14.z, modelview[2], innrm
        dp3 r6.x, r14, r14
        rsq r6.x, r6.x
        mul r14.xyz, r14.xyz, r6.x
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
        mov outclr, ones
        end
    .end
    "#
);

fn material(diffuse: [f32; 3], specular: f32) -> Material {
    Material {
        ambient: diffuse.map(|c| c * 0.3),
        diffuse,
        specular0: [specular; 3],
        specular1: [0.0; 3],
        emission: [0.0; 3],
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
    let u_proj = shader.uniform("projection").unwrap();
    let u_mv = shader.uniform("modelview").unwrap();

    let mut romfs = RomFs::mount().expect("RomFS");
    let bytes = romfs.read("/lemon.cmsh").expect("lemon.cmsh in the RomFS");
    let mesh = Mesh::from_cmsh(&bytes).expect("a valid mesh");
    let names: Vec<String> = mesh.submeshes.iter().map(|s| s.name.clone()).collect();
    log!(
        "model: {} submeshes ({}), {} vertices, {} triangles, {} KiB",
        names.len(),
        names.join(", "),
        mesh.vertices.len(),
        mesh.indices.len() / 3,
        bytes.len() / 1024
    );

    let materials = [("body", material([0.98, 0.82, 0.12], 0.6)), ("stem", material([0.45, 0.32, 0.15], 0.1)), ("leaf", material([0.25, 0.6, 0.2], 0.35))];
    let mut env = LightEnv::new();
    env.set_lut(LutId::D0, LutInput::NH, false, Some(LightLut::phong(18.0)));
    env.lights.push(Light::directional(Vec3::new(-0.5, 0.7, 0.6), [1.0, 0.97, 0.9]));
    let mut fill = Light::directional(Vec3::new(0.7, -0.2, 0.4), [0.25, 0.3, 0.45]);
    fill.specular0 = [0.0; 3];
    env.lights.push(fill);
    let stage1 = TexEnv::PASS.rgb_sources(Source::Previous, Source::FragmentSecondaryColor, Source::PrimaryColor).rgb_func(Combine::Add);

    let projection = Mat4::perspective_tilt(math::to_radians(45.0), 400.0 / 240.0, 0.1, 30.0);
    let view = Mat4::look_at(Vec3::new(0.0, 0.6, 4.2), Vec3::new(0.15, 0.1, 0.0), Vec3::new(0.0, 1.0, 0.0));
    let (mut yaw, mut pitch) = (0.4f32, 0.25f32);
    let mut frame = 0u32;
    while apt.main_loop() {
        let state = input.scan();
        if state.down().contains(Buttons::START) {
            break;
        }
        let (sx, sy) = state.circle_pad();
        yaw += 0.006 + sx as f32 / 156.0 * 0.05;
        pitch = (pitch + sy as f32 / 156.0 * 0.03).clamp(-1.2, 1.2);
        if frame == 30 {
            yaw = 0.4;
        }
        let model = Mat4::rotation_x(pitch) * Mat4::rotation_y(yaw);

        gpu.clear(&top, gpu::rgba(30, 34, 46, 255), 0)?;
        gpu.bind_target(&top);
        gpu.bind_shader(&shader);
        mesh.bind(&mut gpu);
        gpu.set_texenv(0, TexEnv::replace(Source::FragmentPrimaryColor));
        gpu.set_texenv(1, stage1);
        gpu.reset_texenv(2);
        gpu.set_depth_test(true, TestFunc::Greater, WriteMask::ALL);
        gpu.set_cull(Cull::BackCcw);
        gpu.set_blend(None);
        gpu.set_mat4(u_proj, &projection);
        gpu.set_mat4(u_mv, &(view * model));
        for s in &mesh.submeshes {
            env.material = materials.iter().find(|m| m.0 == s.name).map_or(Material::default(), |m| m.1);
            // The leaf is thin: show both of its sides.
            gpu.set_cull(if s.name == "leaf" { Cull::None } else { Cull::BackCcw });
            gpu.set_lighting(Some(&env));
            mesh.draw_submesh(&mut gpu, s);
        }
        screens.show_target(&mut gpu, &top, Screen::Top)?;

        if frame == 30 {
            let c = screens.top();
            let yellow = |p: Color| p.r > 150 && p.g > 110 && p.b < p.g / 2;
            let green = |p: Color| p.g as u16 > p.r as u16 + 15 && p.g as u16 > p.b as u16 + 15 && p.g > 50;
            let (mut ys, mut gs) = (0, 0);
            for y in (20..220).step_by(3) {
                for x in (60..340).step_by(3) {
                    let p = c.get(x, y);
                    ys += yellow(p) as u32;
                    gs += green(p) as u32;
                }
            }
            log!("lemon {} ({} yellow samples)", if ys > 300 { "drawn" } else { "missing" }, ys);
            log!("leaf {} ({} green samples)", if gs > 10 { "drawn" } else { "missing" }, gs);
        }
        let mut b = screens.bottom();
        b.clear(Color::rgb(16, 16, 24));
        b.text(8, 8, "Model", Color::WHITE, 2);
        b.text(8, 36, &format!("{} vertices, {} triangles", mesh.vertices.len(), mesh.indices.len() / 3), Color::rgb(200, 210, 230), 1);
        for (i, s) in mesh.submeshes.iter().enumerate() {
            b.text(8, 54 + i as i32 * 12, &format!("{:5}: {} triangles", s.name, s.count / 3), Color::rgb(200, 210, 230), 1);
        }
        b.text(8, 206, "OBJ -> .cmsh in build.rs -> RomFS", Color::rgb(170, 180, 200), 1);
        b.text(8, 220, "Circle Pad: turn  START: exit", Color::WHITE, 1);
        screens.present();
        frame += 1;
    }
    log!("bye: {} frames", frame);
    Ok(())
}
