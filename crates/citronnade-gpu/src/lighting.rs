//! Fragment lighting: per-pixel lighting computed by fixed-function
//! hardware from interpolated normals (the vertex shader outputs a
//! `normalquat` and a `view` vector), up to eight lights, and lookup tables
//! (LUTs) that shape specular highlights, spotlights, distance attenuation
//! and Fresnel effects. The results feed the texture combiners as
//! `Source::FragmentPrimaryColor` (diffuse) and
//! `Source::FragmentSecondaryColor` (specular).
//!
//! Colours are linear RGB in 0..1. Light positions and directions are in
//! view space (the space of the shader's `view` output).

use alloc::boxed::Box;
use alloc::vec::Vec;

use citronnade_rt::math::{cos, pow};

use crate::math::{Vec3, Vec4};
use crate::{f32_to_f16, f32_to_f20};

/// Lookup tables.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LutId {
    /// Distribution 0 (specular 0's shape).
    D0 = 0,
    /// Distribution 1 (specular 1's shape).
    D1 = 1,
    /// Spotlight attenuation (per light).
    Spot = 2,
    /// Fresnel factor.
    Fresnel = 3,
    /// Reflection for blue, green and red (specular 1's colour).
    ReflectBlue = 4,
    ReflectGreen = 5,
    ReflectRed = 6,
    /// Distance attenuation (per light).
    DistanceAttenuation = 7,
}

/// What indexes a LUT: dot products of the normal (N), the light vector
/// (L), the view vector (V), the half vector (H) and the spotlight
/// direction (P).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LutInput {
    NH = 0,
    VH = 1,
    NV = 2,
    LN = 3,
    /// -L . P (spotlights).
    SP = 4,
    /// cos phi (the angle of H projected on the tangent plane).
    CP = 5,
}

/// Which alphas the Fresnel LUT drives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fresnel {
    None = 0,
    PrimaryAlpha = 1,
    SecondaryAlpha = 2,
    Both = 3,
}

/// A lighting lookup table: 256 entries with their slopes.
#[derive(Clone, PartialEq, Eq)]
pub struct LightLut {
    pub(crate) data: [u32; 256],
}

impl core::fmt::Debug for LightLut {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "LightLut([{:#x}, {:#x}, ..])", self.data[0], self.data[1])
    }
}

impl LightLut {
    /// From 256 values and the 256 differences to the next value.
    fn from_values(values: &[f32; 256], diffs: &[f32; 256]) -> LightLut {
        let mut data = [0u32; 256];
        for i in 0..256 {
            let v = if values[i] > 0.0 { ((values[i] * 4096.0) as u32).min(0xFFF) } else { 0 };
            let mut d = diffs[i];
            let mut dv = 0;
            if d != 0.0 {
                if d < 0.0 {
                    d = -d;
                    dv = 0x800;
                }
                dv |= ((d * 2048.0) as u32).min(0x7FF);
            }
            data[i] = v | dv << 12;
        }
        LightLut { data }
    }

    /// Samples `f` over the input's range: 0..1, or -1..1 when `negative`
    /// (for inputs that can be negative, with the absolute value disabled).
    pub fn from_fn(f: impl Fn(f32) -> f32, negative: bool) -> LightLut {
        let mut values = [0.0f32; 256];
        let mut diffs = [0.0f32; 256];
        let (min, max) = if negative { (-128i32, 128i32) } else { (0, 256) };
        let mut prev = [0.0f32; 256];
        for i in min..=max {
            let x = i as f32 / max as f32;
            let v = f(x);
            let idx = if negative { (i & 0xFF) as usize } else { i as usize };
            if i < max {
                values[idx] = v;
                prev[idx] = v;
            }
            if i > min {
                // The slope belongs to the previous entry.
                let p = if negative { ((i - 1) & 0xFF) as usize } else { (i - 1) as usize };
                diffs[p] = v - prev[p];
            }
        }
        LightLut::from_values(&values, &diffs)
    }

    /// `x^shininess`: Phong/Blinn highlights (with input NH).
    pub fn phong(shininess: f32) -> LightLut {
        LightLut::from_fn(|x| pow(x, shininess), false)
    }

    /// A hard-edged spotlight cone of half-angle `angle` radians (input SP).
    pub fn spotlight(angle: f32) -> LightLut {
        let cutoff = cos(angle);
        LightLut::from_fn(|x| if x >= cutoff { 1.0 } else { 0.0 }, true)
    }

    /// A soft spotlight: full inside `inner`, fading to zero at `outer`
    /// (half-angles in radians).
    pub fn soft_spotlight(inner: f32, outer: f32) -> LightLut {
        let (ci, co) = (cos(inner), cos(outer));
        LightLut::from_fn(move |x| ((x - co) / (ci - co)).clamp(0.0, 1.0), true)
    }
}

/// Attenuation by distance: a LUT over `from..to` view-space units.
#[derive(Clone, Debug, PartialEq)]
pub struct DistanceAttenuation {
    lut: LightLut,
    bias: f32,
    scale: f32,
}

impl DistanceAttenuation {
    /// `f(distance)` sampled between `from` and `to`.
    pub fn from_fn(from: f32, to: f32, f: impl Fn(f32) -> f32) -> DistanceAttenuation {
        let range = to - from;
        let scale = 1.0 / range;
        let mut values = [0.0f32; 256];
        let mut diffs = [0.0f32; 256];
        for i in 0..=256 {
            let v = f(from + range * i as f32 / 256.0);
            if i < 256 {
                values[i] = v;
            }
            if i > 0 {
                diffs[i - 1] = v - values[i - 1];
            }
        }
        DistanceAttenuation { lut: LightLut::from_values(&values, &diffs), bias: -from * scale, scale }
    }

    /// `1 / (1 + linear * d + quadratic * d^2)`.
    pub fn quadratic(from: f32, to: f32, linear: f32, quadratic: f32) -> DistanceAttenuation {
        DistanceAttenuation::from_fn(from, to, |d| 1.0 / (1.0 + linear * d + quadratic * d * d))
    }
}

/// How a surface reflects light.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Material {
    pub ambient: [f32; 3],
    pub diffuse: [f32; 3],
    pub specular0: [f32; 3],
    pub specular1: [f32; 3],
    pub emission: [f32; 3],
}

impl Default for Material {
    fn default() -> Material {
        Material { ambient: [0.2; 3], diffuse: [0.8; 3], specular0: [0.5; 3], specular1: [0.0; 3], emission: [0.0; 3] }
    }
}

/// One light.
#[derive(Clone, Debug, PartialEq)]
pub struct Light {
    pub enabled: bool,
    /// View-space position; `w = 0` makes a directional light shining from
    /// `(x, y, z)` towards the origin.
    pub position: Vec4,
    pub ambient: [f32; 3],
    pub diffuse: [f32; 3],
    pub specular0: [f32; 3],
    pub specular1: [f32; 3],
    /// Light both sides of surfaces.
    pub two_sided: bool,
    /// Spotlight direction (view space) and cone.
    pub spot: Option<(Vec3, LightLut)>,
    pub distance_attenuation: Option<DistanceAttenuation>,
}

impl Default for Light {
    fn default() -> Light {
        Light {
            enabled: true,
            position: Vec4::new(0.0, 0.0, 1.0, 0.0),
            ambient: [0.0; 3],
            diffuse: [1.0; 3],
            specular0: [1.0; 3],
            specular1: [1.0; 3],
            two_sided: false,
            spot: None,
            distance_attenuation: None,
        }
    }
}

impl Light {
    /// A directional light shining from `direction` (view space) towards
    /// the origin, with `color` for diffuse and specular.
    pub fn directional(direction: Vec3, color: [f32; 3]) -> Light {
        Light { position: direction.extend(0.0), diffuse: color, specular0: color, specular1: color, ..Light::default() }
    }

    /// A point light at `position` (view space).
    pub fn point(position: Vec3, color: [f32; 3]) -> Light {
        Light { position: position.extend(1.0), diffuse: color, specular0: color, specular1: color, ..Light::default() }
    }
}

/// The lights, the material and the LUTs, bound with
/// [`crate::Gpu::set_lighting`].
#[derive(Clone, Debug)]
pub struct LightEnv {
    pub material: Material,
    /// The scene's ambient light (multiplies the material's ambient).
    pub ambient: [f32; 3],
    /// Up to eight lights.
    pub lights: Vec<Light>,
    /// LUTs D0, D1, Fresnel, reflection blue/green/red: (table, input,
    /// whether the input may be negative).
    luts: [Option<Box<(LightLut, LutInput, bool)>>; 6],
    pub fresnel: Fresnel,
    /// Keep specular highlights off surfaces facing away from the light.
    pub clamp_highlights: bool,
}

impl Default for LightEnv {
    fn default() -> LightEnv {
        LightEnv::new()
    }
}

fn lut_slot(id: LutId) -> Option<usize> {
    match id {
        LutId::D0 => Some(0),
        LutId::D1 => Some(1),
        LutId::Fresnel => Some(2),
        LutId::ReflectBlue => Some(3),
        LutId::ReflectGreen => Some(4),
        LutId::ReflectRed => Some(5),
        LutId::Spot | LutId::DistanceAttenuation => None,
    }
}

const SLOT_IDS: [LutId; 6] = [LutId::D0, LutId::D1, LutId::Fresnel, LutId::ReflectBlue, LutId::ReflectGreen, LutId::ReflectRed];

/// A colour as the lighting registers take it: 10 bits per channel, blue
/// lowest.
fn pack(r: f32, g: f32, b: f32) -> u32 {
    let c = |v: f32| ((v * 255.0).clamp(0.0, 255.0)) as u32;
    c(b) | c(g) << 10 | c(r) << 20
}

fn fix2_11(x: f32) -> u32 {
    ((x * 2048.0) as i32 as u32) & 0x1FFF
}

/// Register values for the whole environment, ready to emit.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct LightingWords {
    pub ambient: u32,
    pub num_config: [u32; 3],
    pub lut_input: [u32; 3],
    pub permutation: u32,
    /// (light slot, 12 words)
    pub lights: Vec<(usize, [u32; 12])>,
    /// (LUT index register value, table)
    pub luts: Vec<(u32, [u32; 256])>,
}

impl LightEnv {
    pub fn new() -> LightEnv {
        LightEnv {
            material: Material::default(),
            ambient: [1.0; 3],
            lights: Vec::new(),
            luts: [None, None, None, None, None, None],
            fresnel: Fresnel::None,
            clamp_highlights: true,
        }
    }

    /// Uses `lut` for `id` (not `Spot` or `DistanceAttenuation`, which
    /// belong to lights), indexed by `input`; `negative` lets the input go
    /// below zero (the LUT then covers -1..1).
    pub fn set_lut(&mut self, id: LutId, input: LutInput, negative: bool, lut: Option<LightLut>) {
        let slot = lut_slot(id).expect("spot and distance LUTs belong to lights");
        self.luts[slot] = lut.map(|l| Box::new((l, input, negative)));
    }

    /// Words a full upload takes (for the command list's room).
    pub(crate) fn upload_words(&self) -> usize {
        let luts = self.luts.iter().flatten().count()
            + self.lights.iter().map(|l| l.spot.is_some() as usize + l.distance_attenuation.is_some() as usize).sum::<usize>();
        64 + self.lights.len() * 16 + luts * (2 + 32 * 10)
    }

    pub(crate) fn words(&self) -> LightingWords {
        let m = &self.material;
        let ambient = pack(
            m.emission[0] + m.ambient[0] * self.ambient[0],
            m.emission[1] + m.ambient[1] * self.ambient[1],
            m.emission[2] + m.ambient[2] * self.ambient[2],
        );
        let mut config0: u32 = (4 << 8) | 1 << 31 | (self.fresnel as u32) << 2 | (self.clamp_highlights as u32) << 27;
        let mut config1: u32 = !0;
        let mut select: u32 = (LutInput::SP as u32) << (LutId::Spot as u32 * 4);
        let mut abs: u32 = 0x222_2222;
        let mut luts = Vec::new();
        let mut any_cp = false;
        for (slot, entry) in self.luts.iter().enumerate() {
            if let Some(e) = entry {
                let (lut, input, negative) = &**e;
                let id = SLOT_IDS[slot] as u32;
                config1 &= !(1 << (16 + id));
                select = (select & !(0xF << (id * 4))) | (*input as u32) << (id * 4);
                let bit = 1 << (id * 4 + 1);
                abs = if *negative { abs | bit } else { abs & !bit };
                any_cp |= *input == LutInput::CP;
                // Common LUT ids: D0 0, D1 1, FR 3, RB 4, RG 5, RR 6.
                luts.push((id << 8, lut.data));
            }
        }
        let mut lights = Vec::new();
        let (mut count, mut permutation) = (0u32, 0u32);
        for (i, l) in self.lights.iter().enumerate().take(8) {
            if l.enabled {
                permutation |= (i as u32) << (count * 4);
                count += 1;
            }
            let mul = |a: [f32; 3], b: [f32; 3]| pack(a[0] * b[0], a[1] * b[1], a[2] * b[2]);
            let mut words = [0u32; 12];
            words[0] = mul(m.specular0, l.specular0);
            words[1] = mul(m.specular1, l.specular1);
            words[2] = mul(m.diffuse, l.diffuse);
            words[3] = mul(m.ambient, l.ambient);
            words[4] = f32_to_f16(l.position.x) | f32_to_f16(l.position.y) << 16;
            words[5] = f32_to_f16(l.position.z);
            let config = (l.position.w == 0.0) as u32 | (l.two_sided as u32) << 1;
            if let Some((dir, lut)) = &l.spot {
                let d = (-*dir).normalize();
                words[6] = fix2_11(d.x) | fix2_11(d.y) << 16;
                words[7] = fix2_11(d.z);
                config1 &= !(1 << (8 + i));
                luts.push(((8 + i as u32) << 8, lut.data));
            }
            if let Some(da) = &l.distance_attenuation {
                words[10] = f32_to_f20(da.bias);
                words[11] = f32_to_f20(da.scale);
                config1 &= !(1 << (24 + i));
                luts.push(((16 + i as u32) << 8, da.lut.data));
            }
            words[9] = config;
            lights.push((i, words));
        }

        // Pick the smallest LUT layer configuration holding every LUT used.
        const LAYERS: [u8; 7] = [
            1 << 0 | 1 << 6 | 1 << 2 | 1 << 7,
            1 << 3 | 1 << 6 | 1 << 2 | 1 << 7,
            1 << 0 | 1 << 1 | 1 << 6 | 1 << 7,
            1 << 0 | 1 << 1 | 1 << 3 | 1 << 7,
            0xFF & !(1 << 3),
            0xFF & !(1 << 1),
            0xFF & !(1 << 4 | 1 << 5),
        ];
        let mut need = !config1;
        if need & (0xFF << 8) != 0 {
            need |= 1 << (16 + LutId::Spot as u32);
        }
        if need & (0xFF << 24) != 0 {
            need |= 1 << (16 + LutId::DistanceAttenuation as u32);
        }
        let need = ((need >> 16) & 0xFF) as u8;
        let mut layer = 7u32;
        if !any_cp {
            if let Some(i) = LAYERS.iter().position(|&l| l & need == need) {
                layer = i as u32;
            }
        }
        let layer_code = layer + (layer == 7) as u32;
        config0 = (config0 & !(0xF << 4)) | layer_code << 4;

        LightingWords {
            ambient,
            num_config: [count.saturating_sub(1), config0, config1],
            lut_input: [abs, select, 0],
            permutation,
            lights,
            luts,
        }
    }

    pub(crate) fn same_contents(&self, o: &LightEnv) -> bool {
        self.material == o.material
            && self.ambient == o.ambient
            && self.lights == o.lights
            && self.luts == o.luts
            && self.fresnel == o.fresnel
            && self.clamp_highlights == o.clamp_highlights
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phong_lut_entries() {
        let lut = LightLut::phong(1.0);
        // x = i / 256: value i * 16 (of 4096), slope 1/256 (8 of 2048).
        assert_eq!(lut.data[0] & 0xFFF, 0);
        assert_eq!(lut.data[128] & 0xFFF, 2048);
        assert_eq!(lut.data[128] >> 12, 8);
        assert!((lut.data[255] & 0xFFF).abs_diff(4080) <= 1);
    }

    #[test]
    fn environment_words() {
        let mut env = LightEnv::new();
        env.lights.push(Light::directional(Vec3::new(0.0, 0.0, 1.0), [1.0, 0.5, 0.0]));
        env.set_lut(LutId::D0, LutInput::NH, false, Some(LightLut::phong(10.0)));
        let w = env.words();
        assert_eq!(w.num_config[0], 0);
        // D0 enabled: bit 16 cleared.
        assert_eq!(w.num_config[2] & (1 << 16), 0);
        assert_eq!(w.luts.len(), 1);
        assert_eq!(w.luts[0].0, 0);
        // Layer 0 holds D0, RR, SP and DA.
        assert_eq!((w.num_config[1] >> 4) & 0xF, 0);
        // Directional light: config bit 0.
        assert_eq!(w.lights[0].1[9] & 1, 1);
        // diffuse = 0.8 material x light colour (r=1, g=0.5, b=0)
        let d = w.lights[0].1[2];
        assert_eq!((d >> 20) & 0x3FF, 204);
        assert_eq!(d & 0x3FF, 0);
    }
}
