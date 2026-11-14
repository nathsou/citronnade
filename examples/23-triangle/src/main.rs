//! A spinning triangle on each screen: a vertex shader written in PICA200
//! assembly (assembled at compile time by `shader!`), a vertex buffer in
//! linear memory, a projection matrix uniform, and one texture combiner
//! stage passing the vertex colour through. START exits.
#![no_std]
#![no_main]

use citronnade::gpu::{self, shader, AttribType, Cull, Gpu, Primitive, RenderTarget, Shader, Source, TexEnv, TestFunc, VertexLayout, WriteMask};
use citronnade::math::Mat4;
use citronnade::prelude::*;

/// Transforms positions by `projection` and passes colours through.
const VERTEX_SHADER: &[u8] = shader!(
    r#"
    ; uniforms
    .fvec projection[4]
    ; constants
    .constf ones(1.0, 1.0, 1.0, 1.0)
    ; inputs (vertex attributes) and outputs
    .alias inpos v0
    .alias inclr v1
    .out outpos position
    .out outclr color

    .proc main
        ; r0 = (x, y, z, 1)
        mov r0.xyz, inpos
        mov r0.w, ones
        ; outpos = projection * r0
        dp4 outpos.x, projection[0], r0
        dp4 outpos.y, projection[1], r0
        dp4 outpos.z, projection[2], r0
        dp4 outpos.w, projection[3], r0
        mov outclr, inclr
        end
    .end
    "#
);

#[derive(Clone, Copy)]
#[repr(C)]
struct Vertex {
    pos: [f32; 3],
    color: [f32; 4],
}

const TRIANGLE: [Vertex; 3] = [
    Vertex { pos: [0.0, -80.0, 0.0], color: [1.0, 0.2, 0.2, 1.0] },
    Vertex { pos: [-80.0, 60.0, 0.0], color: [0.2, 1.0, 0.2, 1.0] },
    Vertex { pos: [80.0, 60.0, 0.0], color: [0.2, 0.4, 1.0, 1.0] },
];

#[citronnade::main]
fn main(sys: System) -> Result<()> {
    let System { apt, gsp, hid, .. } = sys;
    let mut screens = gsp.screens()?;
    let mut input = hid.input()?;
    let mut gpu = Gpu::new()?;
    let top = RenderTarget::top()?;
    let bottom = RenderTarget::bottom()?;

    let shader = Shader::new(VERTEX_SHADER).expect("shader");
    let projection = shader.uniform("projection").expect("projection uniform");
    log!("triangle: shader has {} instructions, {} operand descriptors", shader.program().code.len(), shader.program().opdescs.len());
    let vertices = gpu::LinearBuffer::from_slice(&TRIANGLE)?;
    let layout = VertexLayout::new().attrib(0, AttribType::Float, 3).attrib(1, AttribType::Float, 4);

    let mut angle = 0.0f32;
    let mut frame = 0u32;
    while apt.main_loop() {
        if input.scan().down().contains(Buttons::START) {
            break;
        }
        // Freeze for the readback check.
        if frame != 30 {
            angle += 0.03;
        }
        for (target, screen, w, spin) in [(&top, Screen::Top, 400.0, angle), (&bottom, Screen::Bottom, 320.0, -angle * 2.0)] {
            gpu.clear(target, gpu::rgba(16, 20, 40, 255), 0)?;
            gpu.bind_target(target);
            gpu.bind_shader(&shader);
            gpu.set_layout(&layout);
            gpu.bind_vertices(&vertices);
            gpu.set_texenv(0, TexEnv::replace(Source::PrimaryColor));
            gpu.set_depth_test(false, TestFunc::Always, WriteMask::COLOR);
            gpu.set_cull(Cull::None);
            // Pixel coordinates, origin at the centre of the screen.
            let m = Mat4::screen_2d(w, 240.0) * Mat4::translation(w / 2.0, 120.0, 0.0) * Mat4::rotation_z(spin);
            gpu.set_mat4(projection, &m);
            gpu.draw_arrays(Primitive::Triangles, 0, 3);
            screens.show_target(&mut gpu, target, screen)?;
        }
        if frame == 30 {
            // With angle frozen, the centre of the top screen is inside the
            // triangle and the corners outside.
            let top_canvas = screens.top();
            let centre = top_canvas.get(200, 120);
            let corner = top_canvas.get(5, 5);
            let bg = (16, 20, 40);
            let is_bg = |c: Color| c.r.abs_diff(bg.0) <= 2 && c.g.abs_diff(bg.1) <= 2 && c.b.abs_diff(bg.2) <= 2;
            log!("centre {} {:?}", if is_bg(centre) { "missing" } else { "covered" }, (centre.r, centre.g, centre.b));
            log!("corner {} {:?}", if is_bg(corner) { "clear" } else { "missing background" }, (corner.r, corner.g, corner.b));
        }
        screens.present();
        frame += 1;
    }
    log!("bye: {} command lists, last {} words", gpu.lists_run(), gpu.last_list_words());
    Ok(())
}
