//! The drawing context: state, the command list, and running it.

use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, Ordering};

use citronnade_services::apt;
use citronnade_shader::{ShaderKind, UniformKind};

use crate::cmd::CommandList;
use crate::lighting::{LightEnv, LightingWords};
use crate::linear::LinearBuffer;
use crate::math::Mat4;
use crate::regs;
use crate::shader::{Shader, UniformLocation};
use crate::state::{Blend, Cull, StencilOp, TestFunc, TexEnv, WriteMask};
use crate::target::RenderTarget;
use crate::texture::Texture;
use crate::{f32_to_f16, f32_to_f24, f32_to_f31, gx, Result};

/// Attribute component types.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttribType {
    Byte = 0,
    UByte = 1,
    Short = 2,
    Float = 3,
}

impl AttribType {
    pub const fn size(self) -> usize {
        match self {
            AttribType::Byte | AttribType::UByte => 1,
            AttribType::Short => 2,
            AttribType::Float => 4,
        }
    }
}

/// One vertex attribute: `count` components of `ty`, fed to input
/// register `v<register>`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Attrib {
    pub register: u8,
    pub ty: AttribType,
    pub count: u8,
}

/// The attributes of an interleaved vertex, in memory order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VertexLayout {
    attribs: Vec<Attrib>,
}

impl VertexLayout {
    pub fn new() -> VertexLayout {
        VertexLayout { attribs: Vec::new() }
    }

    /// Adds an attribute: `count` (1-4) components of `ty` for `v<register>`.
    pub fn attrib(mut self, register: u8, ty: AttribType, count: u8) -> VertexLayout {
        assert!(self.attribs.len() < 12, "at most 12 attributes");
        assert!((1..=4).contains(&count) && register < 16);
        self.attribs.push(Attrib { register, ty, count });
        self
    }

    /// Bytes per vertex.
    pub fn stride(&self) -> usize {
        self.attribs.iter().map(|a| a.ty.size() * a.count as usize).sum()
    }

    pub fn attribs(&self) -> &[Attrib] {
        &self.attribs
    }
}

/// Primitive types.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Primitive {
    Triangles = 0x000,
    TriangleStrip = 0x100,
    TriangleFan = 0x200,
    /// Whatever the geometry shader makes of the vertices.
    Geometry = 0x300,
}

const FB: u32 = 1 << 0;
const VIEWPORT: u32 = 1 << 1;
const SCISSOR: u32 = 1 << 2;
const PROGRAM: u32 = 1 << 3;
const ATTRIBS: u32 = 1 << 4;
const BUFFERS: u32 = 1 << 5;
const EFFECT: u32 = 1 << 6;
const TEXTURES: u32 = 1 << 7;
const TEXENV_BUFFER: u32 = 1 << 8;
const TEXENV: u32 = 1 << 9;
const LIGHTING: u32 = 1 << 10;
const FOG: u32 = 1 << 11;
const PROCTEX: u32 = 1 << 12;
const ALL: u32 = (1 << 13) - 1;

/// Where buffer offsets are counted from (the start of VRAM; linear memory
/// is above it).
const BUFFER_BASE: u32 = 0x1800_0000;

struct FbState {
    color: u32,
    depth: u32,
    width: u16,
    height: u16,
    color_format: u32,
    color_size: u32,
    depth_format: u32,
    depth_mask: u32,
}

/// One stage's code and output map.
struct StageCode {
    id: u32,
    code: Vec<u32>,
    opdescs: Vec<u32>,
    entry: u32,
    outmap: [u32; 8],
    outmap_mask: u32,
    outmap_mode: u32,
    outmap_clock: u32,
    merge: bool,
    geometry: Option<citronnade_shader::GeometryMode>,
}

impl StageCode {
    fn of(shader: &Shader) -> StageCode {
        let p = &shader.program;
        StageCode {
            id: shader.id,
            code: p.code.clone(),
            opdescs: p.opdescs.clone(),
            entry: p.entry,
            outmap: shader.outmap,
            outmap_mask: shader.outmap_mask,
            outmap_mode: shader.outmap_mode,
            outmap_clock: shader.outmap_clock,
            merge: p.outputs.iter().any(|o| o.kind == citronnade_shader::OutputKind::Dummy),
            geometry: p.geometry,
        }
    }
}

struct ProgramState {
    vertex: StageCode,
    geometry: Option<StageCode>,
}

/// A shader stage's uniforms as last set, and which need sending.
struct Uniforms {
    floats: [[f32; 4]; 96],
    float_dirty: u128,
    float_set: u128,
    ints: [u32; 4],
    int_dirty: u8,
    bools: u16,
    bool_dirty: bool,
}

impl Uniforms {
    const fn new() -> Uniforms {
        Uniforms { floats: [[0.0; 4]; 96], float_dirty: 0, float_set: 0, ints: [0; 4], int_dirty: 0, bools: 0, bool_dirty: false }
    }

    fn dirty_all(&mut self) {
        self.float_dirty = self.float_set;
        self.int_dirty = 0xF;
        self.bool_dirty = true;
    }

    fn set_float(&mut self, reg: u8, v: [f32; 4]) {
        if reg < 96 {
            self.floats[reg as usize] = v;
            self.float_dirty |= 1 << reg;
            self.float_set |= 1 << reg;
        }
    }

    fn set_int(&mut self, reg: u8, v: [u8; 4]) {
        if reg < 4 {
            self.ints[reg as usize] = u32::from_le_bytes(v);
            self.int_dirty |= 1 << reg;
        }
    }

    fn set_bool(&mut self, reg: u8, v: bool) {
        if reg < 16 {
            self.bools = (self.bools & !(1 << reg)) | (v as u16) << reg;
            self.bool_dirty = true;
        }
    }

    fn load_constants(&mut self, shader: &Shader) {
        for &(reg, v) in &shader.float_constants {
            self.set_float(reg, v);
        }
        for &(reg, v) in &shader.int_constants {
            self.set_int(reg, v);
        }
        for &(reg, v) in &shader.bool_constants {
            self.set_bool(reg, v);
        }
    }

    /// Sends what changed, to the geometry shader's registers when
    /// `geometry`.
    fn emit(&mut self, l: &mut CommandList, geometry: bool) {
        let (bools, ints, fconfig, fdata) = if geometry {
            (regs::GSH_BOOLUNIFORM, regs::GSH_INTUNIFORM_I0, regs::GSH_FLOATUNIFORM_CONFIG, regs::GSH_FLOATUNIFORM_DATA)
        } else {
            (regs::VSH_BOOLUNIFORM, regs::VSH_INTUNIFORM_I0, regs::VSH_FLOATUNIFORM_CONFIG, regs::VSH_FLOATUNIFORM_DATA)
        };
        if self.bool_dirty {
            l.write(bools, 0x7FFF_0000 | self.bools as u32);
            self.bool_dirty = false;
        }
        if self.int_dirty != 0 {
            l.write_incremental(ints, &self.ints);
            self.int_dirty = 0;
        }
        let mut dirty = self.float_dirty;
        while dirty != 0 {
            let start = dirty.trailing_zeros() as usize;
            let run = (dirty >> start).trailing_ones() as usize;
            let mut words = [0u32; 4 * 32];
            let n = run.min(32);
            for i in 0..n {
                let [x, y, z, w] = self.floats[start + i];
                words[i * 4..i * 4 + 4].copy_from_slice(&[w.to_bits(), z.to_bits(), y.to_bits(), x.to_bits()]);
            }
            l.write(fconfig, 0x8000_0000 | start as u32);
            l.write_repeated(fdata, &words[..n * 4]);
            dirty &= !(((1u128 << n) - 1) << start);
        }
        self.float_dirty = 0;
    }
}

struct Effect {
    depth_map: bool,
    z_scale: u32,
    z_offset: u32,
    cull: u32,
    alpha_test: u32,
    stencil_test: u32,
    stencil_op: u32,
    depth_test: u32,
    blend_color: u32,
    blend: u32,
    logic_op: u32,
    frag_op_mode: u32,
    frag_op_shadow: u32,
}

#[derive(Clone, Copy)]
struct TexState {
    words: [u32; 5],
    format: u32,
}

static RESTORED: AtomicBool = AtomicBool::new(false);

fn apt_hook(hook: apt::Hook) {
    if matches!(hook, apt::Hook::Restore) {
        RESTORED.store(true, Ordering::Release);
    }
}

/// The GPU: records state changes and draws into a command list, and runs
/// it with [`Gpu::finish`] (or implicitly, when clearing or transferring).
pub struct Gpu {
    list: CommandList,
    dirty: u32,
    drawn: bool,
    fb: Option<FbState>,
    viewport: [u32; 5],
    scissor: [u32; 3],
    program: Option<ProgramState>,
    attrib_formats: [u32; 2],
    attrib_permutation: u64,
    attrib_count: u32,
    buffers: [[u32; 3]; 12],
    effect: Effect,
    textures: [Option<TexState>; 3],
    texenv: [TexEnv; 6],
    texenv_dirty: u8,
    texenv_buffer: u32,
    texenv_buffer_color: u32,
    fog_color: u32,
    fog_lut: Option<crate::fog::FogLut>,
    proctex: Option<crate::proctex::ProcTex>,
    proctex_sent: Option<crate::proctex::ProcTex>,
    vertex_uniforms: Uniforms,
    geometry_uniforms: Uniforms,
    lighting: Option<LightEnv>,
    /// What the GPU has, to upload only what changed.
    lighting_sent: Option<LightingWords>,
    lists: u64,
    last_words: usize,
}

impl Gpu {
    /// A context with a command list of 64 Ki words (256 KiB), flushed
    /// automatically when it fills up. GSP must be initialised (the screens
    /// do that).
    pub fn new() -> Result<Gpu> {
        Gpu::with_capacity(0x10000)
    }

    /// A context whose command list holds `words` words.
    pub fn with_capacity(words: usize) -> Result<Gpu> {
        let list = CommandList::new(words)?;
        apt::add_hook(apt_hook);
        Ok(Gpu {
            list,
            dirty: ALL,
            drawn: false,
            fb: None,
            viewport: [0; 5],
            scissor: [0; 3],
            program: None,
            attrib_formats: [0, 0xFFF << 16],
            attrib_permutation: 0,
            attrib_count: 0,
            buffers: [[0; 3]; 12],
            effect: Effect {
                depth_map: true,
                z_scale: f32_to_f24(-1.0),
                z_offset: f32_to_f24(0.0),
                cull: Cull::BackCcw as u32,
                alpha_test: (TestFunc::Always as u32) << 4,
                stencil_test: (TestFunc::Always as u32) << 4 | 0xFF << 24,
                stencil_op: 0,
                depth_test: 1 | (TestFunc::Greater as u32) << 4 | (WriteMask::ALL.0 as u32) << 8,
                blend_color: 0,
                blend: Blend::ALPHA.word(),
                logic_op: 3,
                frag_op_mode: 0xE4_0100,
                frag_op_shadow: f32_to_f16(1.0) | f32_to_f16(-0.0) << 16,
            },
            textures: [None; 3],
            texenv: [TexEnv::PASS; 6],
            texenv_dirty: 0x3F,
            texenv_buffer: 0,
            texenv_buffer_color: 0xFFFF_FFFF,
            fog_color: 0,
            fog_lut: None,
            proctex: None,
            proctex_sent: None,
            vertex_uniforms: Uniforms::new(),
            geometry_uniforms: Uniforms::new(),
            lighting: None,
            lighting_sent: None,
            lists: 0,
            last_words: 0,
        })
    }

    /// Re-sends all state with the next command (the GPU may have lost it,
    /// as when another application ran; done automatically after HOME).
    pub fn reset_state(&mut self) {
        self.dirty = ALL;
        self.lighting_sent = None;
        self.proctex_sent = None;
        self.texenv_dirty = 0x3F;
        self.vertex_uniforms.dirty_all();
        self.geometry_uniforms.dirty_all();
    }

    /// The command list (to add raw register writes).
    pub fn commands(&mut self) -> &mut CommandList {
        &mut self.list
    }

    /// Command lists run so far.
    pub fn lists_run(&self) -> u64 {
        self.lists
    }

    /// Words in the last command list run.
    pub fn last_list_words(&self) -> usize {
        self.last_words
    }

    fn ensure(&mut self, words: usize) {
        if self.list.remaining() < words {
            self.finish().expect("running a full command list failed");
        }
    }

    /// Runs the recorded commands and waits for the GPU to finish them.
    pub fn finish(&mut self) -> Result<()> {
        if self.list.is_empty() {
            return Ok(());
        }
        if self.drawn {
            self.list.write(regs::FRAMEBUFFER_FLUSH, 1);
            self.list.write(regs::FRAMEBUFFER_INVALIDATE, 1);
            self.list.write(regs::EARLYDEPTH_CLEAR, 1);
            self.drawn = false;
        }
        let (ptr, bytes) = self.list.finish();
        self.last_words = bytes / 4;
        crate::flush_linear_heap();
        let r = gx::process_command_list_sync(ptr, bytes);
        self.list.reset();
        self.lists += 1;
        self.dirty |= FB;
        r
    }

    /// Fills a target's colour buffer with `color` (0xAABBGGRR) and its depth
    /// buffer with `depth` (with the stencil value in bits 24-31). The GPU
    /// finishes pending drawing first.
    pub fn clear(&mut self, target: &RenderTarget, color: u32, depth: u32) -> Result<()> {
        self.finish()?;
        let color_fill = gx::Fill {
            start: target.color.as_ptr(),
            end: unsafe { target.color.as_ptr().add(target.color_bytes()) },
            value: target.color_format.fill_value(color),
            width: match target.color_format.bytes_per_pixel() {
                4 => gx::FillWidth::Bits32,
                3 => gx::FillWidth::Bits24,
                _ => gx::FillWidth::Bits16,
            },
        };
        let depth_fill = target.depth.as_ref().map(|d| gx::Fill {
            start: d.as_ptr(),
            end: unsafe { d.as_ptr().add(target.depth_bytes()) },
            value: depth,
            width: match target.depth_format.map_or(4, |f| f.bytes_per_pixel()) {
                4 => gx::FillWidth::Bits32,
                3 => gx::FillWidth::Bits24,
                _ => gx::FillWidth::Bits16,
            },
        });
        gx::memory_fill_sync(color_fill, depth_fill)?;
        self.dirty |= FB;
        Ok(())
    }

    /// Fills only the colour buffer.
    pub fn clear_color(&mut self, target: &RenderTarget, color: u32) -> Result<()> {
        self.finish()?;
        let fill = gx::Fill {
            start: target.color.as_ptr(),
            end: unsafe { target.color.as_ptr().add(target.color_bytes()) },
            value: target.color_format.fill_value(color),
            width: match target.color_format.bytes_per_pixel() {
                4 => gx::FillWidth::Bits32,
                3 => gx::FillWidth::Bits24,
                _ => gx::FillWidth::Bits16,
            },
        };
        gx::memory_fill_sync(fill, None)?;
        self.dirty |= FB;
        Ok(())
    }

    /// Converts a target into a linear framebuffer at `dst` (`dst_format`
    /// pixels, the target's size, rotated as the screens expect), after the
    /// GPU finishes pending drawing. `flags` adds [`gx::flags`] such as
    /// scaling.
    pub fn transfer(&mut self, target: &RenderTarget, dst: *mut u8, dst_format: gx::TransferFormat, flags: u32) -> Result<()> {
        self.finish()?;
        let dim = gx::dim(target.fb_width as u32, target.fb_height as u32);
        let mut out_dim = dim;
        if flags & gx::flags::SCALE_XY != 0 {
            out_dim = gx::dim(target.fb_width as u32 / 2, target.fb_height as u32 / 2);
        } else if flags & gx::flags::SCALE_X != 0 {
            out_dim = gx::dim(target.fb_width as u32 / 2, target.fb_height as u32);
        }
        let f = flags | gx::flags::input_format(target.color_format.transfer_format()) | gx::flags::output_format(dst_format);
        gx::display_transfer_sync(target.color.as_ptr(), dim, dst, out_dim, f)
    }

    /// Draws into `target` from now on (the viewport covers all of it).
    pub fn bind_target(&mut self, target: &RenderTarget) {
        let depth_mask = match target.depth_format {
            None => 0,
            Some(crate::DepthFormat::Depth24Stencil8) => 3,
            Some(_) => 2,
        };
        self.fb = Some(FbState {
            color: target.color.phys(),
            depth: target.depth.as_ref().map_or(0, |d| d.phys()),
            width: target.fb_width,
            height: target.fb_height,
            color_format: target.color_format as u32,
            color_size: target.color_format.size_code(),
            depth_format: target.depth_format.map_or(0, |d| d as u32),
            depth_mask,
        });
        self.dirty |= FB;
        self.set_viewport(0, 0, target.fb_width as u32, target.fb_height as u32);
    }

    /// Sets the viewport, in framebuffer coordinates (x across the 240-pixel
    /// side).
    pub fn set_viewport(&mut self, x: u32, y: u32, w: u32, h: u32) {
        self.viewport = [
            f32_to_f24(w as f32 / 2.0),
            f32_to_f31(2.0 / w as f32) << 1,
            f32_to_f24(h as f32 / 2.0),
            f32_to_f31(2.0 / h as f32) << 1,
            (y << 16) | (x & 0xFFFF),
        ];
        self.scissor = [0, 0, 0];
        self.dirty |= VIEWPORT | SCISSOR;
    }

    /// Limits drawing to a rectangle (framebuffer coordinates), or lifts the
    /// limit with `None`. `invert` keeps the outside instead.
    pub fn set_scissor(&mut self, rect: Option<(u32, u32, u32, u32)>, invert: bool) {
        self.scissor = match rect {
            None => [0, 0, 0],
            Some((left, top, right, bottom)) => {
                [if invert { 1 } else { 3 }, (top << 16) | (left & 0xFFFF), ((bottom - 1) << 16) | ((right - 1) & 0xFFFF)]
            }
        };
        self.dirty |= SCISSOR;
    }

    /// Uses `shader` (a vertex shader) for the following draws, without a
    /// geometry shader. Its constants are loaded.
    pub fn bind_shader(&mut self, shader: &Shader) {
        self.bind_program(shader, None);
    }

    /// Uses a vertex shader and optionally a geometry shader, which receives
    /// the vertex shader's outputs (in output register order) as its
    /// inputs. Draw with [`Primitive::Geometry`] when it is set.
    pub fn bind_program(&mut self, vertex: &Shader, geometry: Option<&Shader>) {
        assert!(!vertex.is_geometry(), "the first shader must be a vertex shader");
        assert!(geometry.is_none_or(|g| g.is_geometry()), "the second shader must be a geometry shader (.gsh)");
        let same = self.program.as_ref().is_some_and(|p| {
            p.vertex.id == vertex.id && p.geometry.as_ref().map(|g| g.id) == geometry.map(|g| g.id)
        });
        if !same {
            self.program = Some(ProgramState { vertex: StageCode::of(vertex), geometry: geometry.map(StageCode::of) });
            self.dirty |= PROGRAM | ATTRIBS;
        }
        self.vertex_uniforms.load_constants(vertex);
        if let Some(g) = geometry {
            self.geometry_uniforms.load_constants(g);
        }
    }

    fn uniforms(&mut self, loc: UniformLocation) -> &mut Uniforms {
        match loc.stage {
            ShaderKind::Vertex => &mut self.vertex_uniforms,
            ShaderKind::Geometry => &mut self.geometry_uniforms,
        }
    }

    /// Sets a float uniform (`.fvec name`).
    pub fn set_vec4(&mut self, loc: UniformLocation, v: [f32; 4]) {
        debug_assert_eq!(loc.kind, UniformKind::Float);
        self.uniforms(loc).set_float(loc.register, v);
    }

    /// Sets four float uniform registers from a matrix's rows
    /// (`.fvec name[4]`).
    pub fn set_mat4(&mut self, loc: UniformLocation, m: &Mat4) {
        debug_assert_eq!(loc.kind, UniformKind::Float);
        let u = self.uniforms(loc);
        for (i, row) in m.rows.iter().enumerate() {
            u.set_float(loc.register + i as u8, row.to_array());
        }
    }

    /// Sets one register of a float uniform array.
    pub fn set_vec4_at(&mut self, loc: UniformLocation, index: u8, v: [f32; 4]) {
        self.uniforms(loc).set_float(loc.register + index, v);
    }

    /// Sets an integer uniform (`.ivec name`): for loops use x as the count,
    /// y as the start and z as the step.
    pub fn set_ivec4(&mut self, loc: UniformLocation, v: [u8; 4]) {
        debug_assert_eq!(loc.kind, UniformKind::Int);
        self.uniforms(loc).set_int(loc.register, v);
    }

    /// Sets a boolean uniform (`.bool name`).
    pub fn set_bool(&mut self, loc: UniformLocation, v: bool) {
        debug_assert_eq!(loc.kind, UniformKind::Bool);
        self.uniforms(loc).set_bool(loc.register, v);
    }

    /// Describes the vertices of the following draws.
    pub fn set_layout(&mut self, layout: &VertexLayout) {
        let mut formats = [0u32, 0xFFF << 16];
        let mut perm = 0u64;
        for (id, a) in layout.attribs.iter().enumerate() {
            let f = ((a.count as u32 - 1) << 2 | a.ty as u32) << ((id % 8) * 4);
            formats[id / 8] |= f;
            formats[1] = (formats[1] & !(0xF000_0000 | 1 << (id + 16))) | (id as u32) << 28;
            perm |= (a.register as u64) << (id * 4);
        }
        self.attrib_formats = formats;
        self.attrib_permutation = perm;
        self.attrib_count = layout.attribs.len() as u32;
        self.dirty |= ATTRIBS;
        // One interleaved buffer by default.
        self.buffers = [[0; 3]; 12];
        let n = layout.attribs.len() as u64;
        let order: u64 = (0..n).map(|i| i << (i * 4)).sum();
        self.buffers[0] = [0, order as u32, (order >> 32) as u32 | (layout.stride() as u32) << 16 | (n as u32) << 28];
        self.dirty |= BUFFERS;
    }

    /// Reads the vertices of the following draws from `data` (laid out as
    /// the last [`Gpu::set_layout`] said, interleaved).
    pub fn bind_vertices<T: Copy>(&mut self, data: &LinearBuffer<T>) {
        self.bind_vertex_addr(data.phys());
    }

    /// Like [`Gpu::bind_vertices`], for raw linear or VRAM memory.
    pub fn bind_vertex_addr(&mut self, phys: u32) {
        assert!(phys >= BUFFER_BASE, "vertex data must be in linear memory or VRAM");
        self.buffers[0][0] = phys - BUFFER_BASE;
        self.dirty |= BUFFERS;
    }

    /// Samples `texture` in unit 0-2 (`None` disables the unit).
    pub fn bind_texture(&mut self, unit: usize, texture: Option<&Texture>) {
        self.textures[unit] = texture.map(|t| TexState {
            words: [t.border, (t.width as u32) << 16 | t.height as u32, t.param, 0, t.data.phys() >> 3],
            format: t.format as u32,
        });
        self.dirty |= TEXTURES;
    }

    /// Samples a render target's colour buffer in unit 0-2 (render to
    /// texture). Its sides must be powers of two; draw into it with the
    /// untilted projections ([`Mat4::ortho`], [`Mat4::perspective`]) so that
    /// texture coordinates follow the image (u = x, v = y).
    pub fn bind_target_texture(&mut self, unit: usize, target: &RenderTarget, filter: crate::Filter, wrap: crate::Wrap) {
        let (w, h) = (target.fb_width as u32, target.fb_height as u32);
        assert!(w.is_power_of_two() && h.is_power_of_two() && w >= 8 && h >= 8, "render target textures need power-of-two sides");
        use crate::{ColorFormat as C, TexFormat as T};
        let format = match target.color_format {
            C::Rgba8 => T::Rgba8,
            C::Rgb8 => T::Rgb8,
            C::Rgba5551 => T::Rgba5551,
            C::Rgb565 => T::Rgb565,
            C::Rgba4 => T::Rgba4,
        };
        let param = (filter as u32) << 1 | (filter as u32) << 2 | (wrap as u32) << 12 | (wrap as u32) << 8;
        self.textures[unit] = Some(TexState { words: [0, w << 16 | h, param, 0, target.color.phys() >> 3], format: format as u32 });
        self.dirty |= TEXTURES;
    }

    /// Sets texture combiner stage `stage` (0-5).
    pub fn set_texenv(&mut self, stage: usize, env: TexEnv) {
        if self.texenv[stage] != env || self.dirty & TEXENV != 0 {
            self.texenv[stage] = env;
            self.texenv_dirty |= 1 << stage;
        }
    }

    /// Resets stages `from..6` to pass the previous result through.
    pub fn reset_texenv(&mut self, from: usize) {
        for s in from..6 {
            self.set_texenv(s, TexEnv::PASS);
        }
    }

    /// Chooses which stages (1-4) write their result to the combiner buffer
    /// for later stages (RGB and alpha masks, bit n = stage n + 1), and its
    /// initial colour.
    pub fn set_texenv_buffer(&mut self, rgb_mask: u8, alpha_mask: u8, initial: u32) {
        self.texenv_buffer = (self.texenv_buffer & !0xFF00) | ((rgb_mask & 0xF) as u32) << 8 | ((alpha_mask & 0xF) as u32) << 12;
        self.texenv_buffer_color = initial;
        self.dirty |= TEXENV_BUFFER;
    }

    /// Turns fragment lighting on with `env`, or off. The environment is
    /// copied; call again after changing it. Only what changed is sent.
    pub fn set_lighting(&mut self, env: Option<&LightEnv>) {
        match (env, &self.lighting) {
            (None, None) => {}
            (Some(e), Some(cur)) if cur.same_contents(e) => {}
            _ => {
                self.lighting = env.cloned();
                self.dirty |= LIGHTING;
            }
        }
    }

    /// Turns the procedural texture unit (`Source::Texture3`) on with
    /// `proctex`, or off. Only what changed is sent.
    pub fn set_proctex(&mut self, proctex: Option<&crate::proctex::ProcTex>) {
        if self.proctex.as_ref() != proctex {
            self.proctex = proctex.cloned();
            self.dirty |= PROCTEX | TEXTURES;
        }
    }

    /// Fog towards `color` (0xAABBGGRR) by `lut`, or no fog.
    pub fn set_fog(&mut self, fog: Option<(&crate::fog::FogLut, u32)>) {
        match fog {
            Some((lut, color)) => {
                self.texenv_buffer = (self.texenv_buffer & !0x1_00FF) | 5;
                self.fog_color = color;
                if self.fog_lut.as_ref() != Some(lut) {
                    self.fog_lut = Some(lut.clone());
                    self.dirty |= FOG;
                }
            }
            None => self.texenv_buffer &= !0x1_00FF,
        }
        self.dirty |= TEXENV_BUFFER;
    }

    /// The depth test and which components fragments write.
    pub fn set_depth_test(&mut self, enabled: bool, func: TestFunc, write: WriteMask) {
        self.effect.depth_test = enabled as u32 | (func as u32) << 4 | (write.0 as u32) << 8;
        self.dirty |= EFFECT;
    }

    /// Maps clip-space depth to the depth buffer (default -1, 0: the
    /// projection matrices' convention).
    pub fn set_depth_map(&mut self, scale: f32, offset: f32) {
        self.effect.z_scale = f32_to_f24(scale);
        self.effect.z_offset = f32_to_f24(offset);
        self.dirty |= EFFECT;
    }

    pub fn set_cull(&mut self, cull: Cull) {
        self.effect.cull = cull as u32;
        self.dirty |= EFFECT;
    }

    /// Blending, or `None` to write fragments as they are.
    pub fn set_blend(&mut self, blend: Option<Blend>) {
        match blend {
            Some(b) => {
                self.effect.blend = b.word();
                self.effect.frag_op_mode = (self.effect.frag_op_mode & !0xFF00) | 0x100;
            }
            None => {
                self.effect.frag_op_mode &= !0xFF00;
                self.effect.logic_op = 3; // copy
            }
        }
        self.dirty |= EFFECT;
    }

    /// The constant colour of `BlendFactor::Constant*` (0xAABBGGRR).
    pub fn set_blend_color(&mut self, rgba: u32) {
        self.effect.blend_color = rgba;
        self.dirty |= EFFECT;
    }

    /// Drops fragments whose alpha fails `func` against `reference`.
    pub fn set_alpha_test(&mut self, enabled: bool, func: TestFunc, reference: u8) {
        self.effect.alpha_test = enabled as u32 | (func as u32) << 4 | (reference as u32) << 8;
        self.dirty |= EFFECT;
    }

    /// The stencil test: `(stencil & input_mask) func (reference & input_mask)`.
    pub fn set_stencil_test(&mut self, enabled: bool, func: TestFunc, reference: u8, input_mask: u8, write_mask: u8) {
        self.effect.stencil_test = enabled as u32
            | (func as u32) << 4
            | (write_mask as u32) << 8
            | (reference as u32) << 16
            | (input_mask as u32) << 24;
        self.dirty |= EFFECT;
    }

    /// What the stencil test does to the buffer when the stencil test fails,
    /// when the depth test fails, and when both pass.
    pub fn set_stencil_op(&mut self, fail: StencilOp, depth_fail: StencilOp, pass: StencilOp) {
        self.effect.stencil_op = fail as u32 | (depth_fail as u32) << 4 | (pass as u32) << 8;
        self.dirty |= EFFECT;
    }

    fn update(&mut self) {
        if RESTORED.swap(false, Ordering::AcqRel) {
            self.reset_state();
        }
        // Room for a full shader upload, every uniform, the lighting and
        // the draw itself.
        let lighting_words = match &self.lighting {
            Some(env) if self.dirty & LIGHTING != 0 => env.upload_words(),
            _ => 0,
        };
        self.ensure(2048 + lighting_words);
        let l = &mut self.list;
        let d = self.dirty;
        if d & FB != 0 {
            if let Some(fb) = &self.fb {
                if self.drawn {
                    l.write(regs::FRAMEBUFFER_FLUSH, 1);
                    l.write(regs::EARLYDEPTH_CLEAR, 1);
                    self.drawn = false;
                }
                l.write(regs::FRAMEBUFFER_INVALIDATE, 1);
                let dim = 0x0100_0000 | ((fb.height as u32 - 1) & 0xFFF) << 12 | (fb.width as u32 & 0xFFF);
                l.write_incremental(regs::DEPTHBUFFER_LOC, &[fb.depth >> 3, fb.color >> 3, dim]);
                l.write(regs::RENDERBUF_DIM, dim);
                l.write(regs::DEPTHBUFFER_FORMAT, fb.depth_format);
                l.write(regs::COLORBUFFER_FORMAT, fb.color_size | fb.color_format << 16);
                l.write(regs::FRAMEBUFFER_BLOCK32, 0);
                let dm = if fb.depth != 0 { fb.depth_mask } else { 0 };
                l.write_incremental(regs::COLORBUFFER_READ, &[0xF, 0xF, dm, dm]);
            }
        }
        if d & VIEWPORT != 0 {
            l.write_incremental(regs::VIEWPORT_WIDTH, &self.viewport[..4]);
            l.write(regs::VIEWPORT_XY, self.viewport[4]);
        }
        if d & SCISSOR != 0 {
            l.write_incremental(regs::SCISSORTEST_MODE, &self.scissor);
        }
        if d & PROGRAM != 0 {
            if let Some(p) = &self.program {
                emit_program(l, p);
            }
        }
        if d & ATTRIBS != 0 && self.attrib_count > 0 {
            l.write_incremental(regs::ATTRIBBUFFERS_FORMAT_LOW, &self.attrib_formats);
            l.write_masked(regs::VSH_INPUTBUFFER_CONFIG, 0xB, 0xA000_0000 | (self.attrib_count - 1));
            l.write(regs::VSH_NUM_ATTR, self.attrib_count - 1);
            l.write_incremental(
                regs::VSH_ATTRIBUTES_PERMUTATION_LOW,
                &[self.attrib_permutation as u32, (self.attrib_permutation >> 32) as u32],
            );
        }
        if d & BUFFERS != 0 {
            l.write(regs::ATTRIBBUFFERS_LOC, BUFFER_BASE >> 3);
            let mut words = [0u32; 36];
            for (i, b) in self.buffers.iter().enumerate() {
                words[i * 3..i * 3 + 3].copy_from_slice(b);
            }
            l.write_incremental(regs::ATTRIBBUFFER0_OFFSET, &words);
        }
        if d & EFFECT != 0 {
            let e = &self.effect;
            l.write(regs::DEPTHMAP_ENABLE, e.depth_map as u32);
            l.write(regs::FACECULLING_CONFIG, e.cull & 3);
            l.write_incremental(regs::DEPTHMAP_SCALE, &[e.z_scale, e.z_offset]);
            l.write_incremental(regs::FRAGOP_ALPHA_TEST, &[e.alpha_test, e.stencil_test, e.stencil_op, e.depth_test]);
            l.write(regs::BLEND_COLOR, e.blend_color);
            l.write(regs::BLEND_FUNC, e.blend);
            l.write(regs::LOGIC_OP, e.logic_op);
            l.write_masked(regs::COLOR_OPERATION, 0x7, e.frag_op_mode);
            l.write(regs::FRAGOP_SHADOW, e.frag_op_shadow);
            l.write_masked(regs::EARLYDEPTH_TEST1, 1, 0);
            l.write(regs::EARLYDEPTH_TEST2, 0);
        }
        if d & TEXTURES != 0 {
            let mut units = 0;
            let bases = [
                (regs::TEXUNIT0_BORDER_COLOR, regs::TEXUNIT0_TYPE),
                (regs::TEXUNIT1_BORDER_COLOR, regs::TEXUNIT1_TYPE),
                (regs::TEXUNIT2_BORDER_COLOR, regs::TEXUNIT2_TYPE),
            ];
            for (i, t) in self.textures.iter().enumerate() {
                if let Some(t) = t {
                    units |= 1 << i;
                    l.write_incremental(bases[i].0, &t.words);
                    l.write(bases[i].1, t.format);
                }
            }
            if let Some(p) = &self.proctex {
                units |= 1 << 10 | ((p.texcoord as u32 & 3) << 8);
            }
            l.write_masked(regs::TEXUNIT_CONFIG, 0xB, units | 1 << 12 | 1 << 16);
            l.write_masked(regs::TEXUNIT_CONFIG, 0x4, 1 << 16);
            l.write(regs::TEXUNIT0_SHADOW, 1);
        }
        if d & TEXENV_BUFFER != 0 {
            l.write_masked(regs::TEXENV_UPDATE_BUFFER, 0x7, self.texenv_buffer);
            l.write(regs::TEXENV_BUFFER_COLOR, self.texenv_buffer_color);
            l.write(regs::FOG_COLOR, self.fog_color);
        }
        if d & PROCTEX != 0 {
            if let Some(p) = &self.proctex {
                let sent = self.proctex_sent.as_ref();
                l.write_incremental(regs::TEXUNIT3_PROCTEX0, &p.words());
                let tables = [(0u32, &p.noise_lut), (2, &p.rgb_lut), (3, &p.alpha_lut)];
                for (id, lut) in tables {
                    let old = sent.map(|s| match id {
                        0 => &s.noise_lut,
                        2 => &s.rgb_lut,
                        _ => &s.alpha_lut,
                    });
                    if old != Some(lut) {
                        l.write(regs::PROCTEX_LUT, id << 8);
                        l.write_repeated(regs::PROCTEX_LUT_DATA0, &lut.0);
                    }
                }
                if sent.is_none_or(|s| s.color_tables() != p.color_tables()) {
                    let (colors, diffs) = p.color_tables();
                    l.write(regs::PROCTEX_LUT, 4 << 8);
                    l.write_repeated(regs::PROCTEX_LUT_DATA0, &colors[..]);
                    l.write(regs::PROCTEX_LUT, 5 << 8);
                    l.write_repeated(regs::PROCTEX_LUT_DATA0, &diffs[..]);
                }
                self.proctex_sent = Some(p.clone());
            }
        }
        if d & FOG != 0 && self.texenv_buffer & 7 == 5 {
            if let Some(lut) = &self.fog_lut {
                l.write(regs::FOG_LUT_INDEX, 0);
                l.write_repeated(regs::FOG_LUT_DATA0, &lut.data);
            }
        }
        if d & TEXENV != 0 {
            self.texenv_dirty = 0x3F;
        }
        for stage in 0..6 {
            if self.texenv_dirty & (1 << stage) != 0 {
                let id = if stage >= 4 { stage + 2 } else { stage } as u16;
                l.write_incremental(regs::TEXENV0_SOURCE + id * 8, &self.texenv[stage].words());
            }
        }
        self.texenv_dirty = 0;
        if d & LIGHTING != 0 {
            let enabled = self.lighting.is_some();
            l.write(regs::LIGHTING_ENABLE0, enabled as u32);
            l.write(regs::LIGHTING_ENABLE1, !enabled as u32);
            if let Some(env) = &self.lighting {
                let w = env.words();
                let sent = self.lighting_sent.as_ref();
                l.write(regs::LIGHTING_AMBIENT, w.ambient);
                l.write_incremental(regs::LIGHTING_NUM_LIGHTS, &w.num_config);
                l.write_incremental(regs::LIGHTING_LUTINPUT_ABS, &w.lut_input);
                l.write(regs::LIGHTING_LIGHT_PERMUTATION, w.permutation);
                for (slot, words) in &w.lights {
                    if sent.is_none_or(|s| !s.lights.contains(&(*slot, *words))) {
                        l.write_incremental(regs::LIGHT0_SPECULAR0 + *slot as u16 * 0x10, words);
                    }
                }
                for (index, table) in &w.luts {
                    if sent.is_none_or(|s| !s.luts.iter().any(|(i, t)| i == index && t == table)) {
                        l.write(regs::LIGHTING_LUT_INDEX, *index);
                        for chunk in table.chunks(8) {
                            l.write_repeated(regs::LIGHTING_LUT_DATA0, chunk);
                        }
                    }
                }
                self.lighting_sent = Some(w);
            } else {
                self.lighting_sent = None;
            }
        }
        self.dirty = 0;

        // Uniforms.
        self.vertex_uniforms.emit(l, false);
        if self.program.as_ref().is_some_and(|p| p.geometry.is_some()) {
            self.geometry_uniforms.emit(l, true);
        }
    }

    /// Draws `count` vertices from `first` of the bound vertex buffer.
    pub fn draw_arrays(&mut self, primitive: Primitive, first: u32, count: u32) {
        assert!(self.fb.is_some(), "no render target bound");
        assert!(self.program.is_some(), "no shader bound");
        self.update();
        let l = &mut self.list;
        l.write_masked(regs::PRIMITIVE_CONFIG, 0x2, primitive as u32);
        l.write(regs::RESTART_PRIMITIVE, 1);
        l.write(regs::INDEXBUFFER_CONFIG, 0x8000_0000);
        l.write(regs::NUMVERTICES, count);
        l.write(regs::VERTEX_OFFSET, first);
        l.write_masked(regs::GEOSTAGE_CONFIG2, 0x1, 1);
        l.write_masked(regs::START_DRAW_FUNC0, 0x1, 0);
        l.write(regs::DRAWARRAYS, 1);
        l.write_masked(regs::START_DRAW_FUNC0, 0x1, 1);
        l.write_masked(regs::GEOSTAGE_CONFIG2, 0x1, 0);
        l.write(regs::VTX_FUNC, 1);
        self.drawn = true;
    }

    /// Draws `count` vertices picked by 16-bit `indices` from the bound
    /// vertex buffer.
    pub fn draw_elements(&mut self, primitive: Primitive, indices: &LinearBuffer<u16>, count: u32) {
        self.draw_elements_range(primitive, indices, 0, count);
    }

    /// Like [`Gpu::draw_elements`], from index `first`.
    pub fn draw_elements_range(&mut self, primitive: Primitive, indices: &LinearBuffer<u16>, first: u32, count: u32) {
        assert!(self.fb.is_some(), "no render target bound");
        assert!(self.program.is_some(), "no shader bound");
        assert!((first + count) as usize <= indices.len(), "index range out of bounds");
        self.update();
        let pa = indices.phys() + first * 2;
        assert!(pa >= BUFFER_BASE);
        let l = &mut self.list;
        let triangles = primitive == Primitive::Triangles;
        l.write_masked(regs::PRIMITIVE_CONFIG, 0x2, if triangles { Primitive::Geometry as u32 } else { primitive as u32 });
        l.write(regs::RESTART_PRIMITIVE, 1);
        l.write(regs::INDEXBUFFER_CONFIG, (pa - BUFFER_BASE) | 1 << 31);
        l.write(regs::NUMVERTICES, count);
        l.write(regs::VERTEX_OFFSET, 0);
        if triangles {
            l.write_masked(regs::GEOSTAGE_CONFIG, 0x2, 0x100);
            l.write_masked(regs::GEOSTAGE_CONFIG2, 0x2, 0x100);
        }
        l.write_masked(regs::START_DRAW_FUNC0, 0x1, 0);
        l.write(regs::DRAWELEMENTS, 1);
        l.write_masked(regs::START_DRAW_FUNC0, 0x1, 1);
        if triangles {
            l.write_masked(regs::GEOSTAGE_CONFIG, 0x2, 0);
            l.write_masked(regs::GEOSTAGE_CONFIG2, 0x2, 0);
        }
        l.write(regs::VTX_FUNC, 1);
        l.write_masked(regs::PRIMITIVE_CONFIG, 0x8, 0);
        l.write_masked(regs::PRIMITIVE_CONFIG, 0x8, 0);
        self.drawn = true;
    }
}

fn upload_code(l: &mut CommandList, code: &StageCode, geometry: bool) {
    let (config, data, end, op_config, op_data) = if geometry {
        (regs::GSH_CODETRANSFER_CONFIG, regs::GSH_CODETRANSFER_DATA, regs::GSH_CODETRANSFER_END, regs::GSH_OPDESCS_CONFIG, regs::GSH_OPDESCS_DATA)
    } else {
        (regs::VSH_CODETRANSFER_CONFIG, regs::VSH_CODETRANSFER_DATA, regs::VSH_CODETRANSFER_END, regs::VSH_OPDESCS_CONFIG, regs::VSH_OPDESCS_DATA)
    };
    // The vertex shader units hold 512 instructions.
    let words = if geometry { &code.code[..code.code.len().min(4096)] } else { &code.code[..code.code.len().min(512)] };
    l.write(config, 0);
    for chunk in words.chunks(128) {
        l.write_repeated(data, chunk);
    }
    l.write(end, 1);
    l.write(op_config, 0);
    if !code.opdescs.is_empty() {
        l.write_repeated(op_data, &code.opdescs);
    }
}

/// Merges a vertex and a geometry shader's output maps: shared semantics
/// first, then the geometry shader's, then the vertex shader's (libctru's
/// order).
fn merge_outmaps(vsh: &[u32; 8], gsh: &[u32; 8]) -> [u32; 8] {
    let mut out = [0x1F1F_1F1Fu32; 8];
    out[0] = 0;
    let (mut vsh_common, mut gsh_common) = (0u32, 0u32);
    let push = |out: &mut [u32; 8], v: u32| {
        if out[0] < 7 {
            out[0] += 1;
            out[out[0] as usize] = v;
        }
    };
    for i in 1..8 {
        if gsh[i] == 0x1F1F_1F1F {
            break;
        }
        if let Some(j) = (1..8).find(|&j| vsh[j] == gsh[i]) {
            push(&mut out, gsh[i]);
            vsh_common |= 1 << j;
            gsh_common |= 1 << i;
        }
    }
    for i in 1..8 {
        if gsh[i] == 0x1F1F_1F1F {
            break;
        }
        if gsh_common & (1 << i) == 0 {
            push(&mut out, gsh[i]);
        }
    }
    for i in 1..8 {
        if vsh[i] == 0x1F1F_1F1F {
            break;
        }
        if vsh_common & (1 << i) == 0 {
            push(&mut out, vsh[i]);
        }
    }
    out
}

/// libctru's `shaderProgramConfigure`.
fn emit_program(l: &mut CommandList, p: &ProgramState) {
    let v = &p.vertex;
    let g = p.geometry.as_ref();
    l.write_masked(regs::GEOSTAGE_CONFIG, 0x3, if g.is_some() { 2 } else { 0 });
    l.write_masked(regs::GEOSTAGE_CONFIG2, 0x3, 0);
    l.write_masked(regs::VSH_COM_MODE, 0x1, g.is_some() as u32);
    upload_code(l, v, false);
    l.write(regs::VSH_ENTRYPOINT, 0x7FFF_0000 | (v.entry & 0xFFFF));
    l.write(regs::VSH_OUTMAP_MASK, v.outmap_mask);
    l.write(regs::VSH_OUTMAP_TOTAL1, v.outmap[0].wrapping_sub(1));
    l.write(regs::VSH_OUTMAP_TOTAL2, v.outmap[0].wrapping_sub(1));
    let main = g.unwrap_or(v);
    let (mut outmap, mut mode, mut clock) = (main.outmap, main.outmap_mode, main.outmap_clock);
    if let Some(g) = g {
        upload_code(l, g, true);
        l.write(regs::GSH_ENTRYPOINT, 0x7FFF_0000 | (g.entry & 0xFFFF));
        l.write(regs::GSH_OUTMAP_MASK, g.outmap_mask);
        if g.merge {
            outmap = merge_outmaps(&v.outmap, &g.outmap);
            mode |= v.outmap_mode;
            clock |= v.outmap_clock;
        }
    }
    l.write_masked(regs::PRIMITIVE_CONFIG, 0x1, outmap[0].wrapping_sub(1));
    l.write_incremental(regs::SH_OUTMAP_TOTAL, &outmap);
    l.write(regs::SH_OUTATTR_MODE, mode);
    l.write(regs::SH_OUTATTR_CLOCK, clock);
    match g {
        Some(g) => {
            let gm = g.geometry.unwrap_or(citronnade_shader::GeometryMode { mode: 0, fixed_start: 0, variable_count: 0, fixed_count: 0 });
            // Inputs per vertex: the vertex shader's outputs.
            let stride = v.outmap[0].max(1);
            l.write_masked(regs::GEOSTAGE_CONFIG, 0xA, if gm.mode == 1 { 0x8000_0000 } else { 0 });
            let mut misc = gm.mode as u32;
            if gm.mode == 2 {
                misc |= 0x0100_0000 | (gm.fixed_start as u32) << 16 | (stride - 1) << 12 | (gm.fixed_count as u32).saturating_sub(1) << 8;
            }
            l.write(regs::GSH_MISC0, misc);
            l.write(regs::GSH_MISC1, if gm.mode == 1 { (gm.variable_count as u32).saturating_sub(1) } else { 0 });
            l.write(regs::GSH_INPUTBUFFER_CONFIG, 0x0800_0000 | if gm.mode != 0 { 0x100 } else { 0 } | (stride - 1));
            l.write_incremental(regs::GSH_ATTRIBUTES_PERMUTATION_LOW, &[0x7654_3210, 0xFEDC_BA98]);
        }
        None => {
            l.write_masked(regs::GEOSTAGE_CONFIG, 0xA, 0);
            l.write(regs::GSH_MISC0, 0);
            l.write(regs::GSH_MISC1, 0);
            l.write(regs::GSH_INPUTBUFFER_CONFIG, 0xA000_0000);
        }
    }
}

impl Drop for Gpu {
    fn drop(&mut self) {
        let _ = self.finish();
        apt::remove_hook(apt_hook);
    }
}
