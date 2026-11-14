//! The procedural texture unit ("texture 3"): it computes texels from the
//! texture coordinates instead of reading memory. Coordinates (u, v) are
//! made positive (|u|, |v|), go through optional noise, shifting and
//! clamping, a mapping function combines them
//! into one value (`u`, `u + v`, `sqrt(u² + v²)`, ...), a lookup table
//! reshapes that value, and the result indexes a colour gradient. Combiners
//! read it as `Source::Texture3`.

use alloc::boxed::Box;
use alloc::vec::Vec;

use crate::f32_to_f16;

/// What happens to coordinates outside 0..1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProcClamp {
    ToZero = 0,
    ToEdge = 1,
    Repeat = 2,
    MirroredRepeat = 3,
    /// 0 up to 0.5, 1 above (a square wave once shifted or noisy).
    Pulse = 4,
}

/// How u and v combine into one value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProcMap {
    U = 0,
    U2 = 1,
    V = 2,
    V2 = 3,
    /// (u + v) / 2
    Add = 4,
    /// (u² + v²) / 2
    Add2 = 5,
    /// sqrt(u² + v²) (rings)
    SqrtAdd2 = 6,
    Min = 7,
    Max = 8,
    /// ((u + v) / 2 + sqrt(u² + v²)) / 2
    RMax = 9,
}

/// Shifts every other row or column (bricks).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProcShift {
    None = 0,
    Odd = 1,
    Even = 2,
}

/// A 128-entry table over 0..1 (the noise and mapping tables).
#[derive(Clone, PartialEq, Eq)]
pub struct ProcLut(pub(crate) [u32; 128]);

impl core::fmt::Debug for ProcLut {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "ProcLut([{:#x}, ..])", self.0[0])
    }
}

impl ProcLut {
    /// Samples `f` (clamped to 0..1) at 129 points over 0..1.
    pub fn from_fn(f: impl Fn(f32) -> f32) -> ProcLut {
        let v: [u32; 129] = core::array::from_fn(|i| (4095.0 * f(i as f32 / 128.0).clamp(0.0, 1.0)) as u32);
        ProcLut(core::array::from_fn(|i| v[i] | (v[i + 1].wrapping_sub(v[i]) & 0xFFF) << 12))
    }

    /// The identity.
    pub fn linear() -> ProcLut {
        ProcLut::from_fn(|x| x)
    }
}

/// Noise on one coordinate.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProcNoise {
    pub amplitude: f32,
    pub frequency: f32,
    pub phase: f32,
}

/// A procedural texture.
#[derive(Clone, Debug, PartialEq)]
pub struct ProcTex {
    pub u_clamp: ProcClamp,
    pub v_clamp: ProcClamp,
    pub rgb_map: ProcMap,
    /// A separate mapping for alpha (`None`: alpha comes from the
    /// gradient). With one, alpha is `alpha_lut(alpha_map(u, v))` directly.
    pub alpha_map: Option<ProcMap>,
    pub u_shift: ProcShift,
    pub v_shift: ProcShift,
    /// Noise on u and v (the noise table shapes it).
    pub noise: Option<(ProcNoise, ProcNoise)>,
    pub noise_lut: ProcLut,
    /// Reshapes the mapped value before the colour lookup.
    pub rgb_lut: ProcLut,
    pub alpha_lut: ProcLut,
    /// The gradient (0xAABBGGRR), 1 to 255 entries.
    colors: Vec<u32>,
    /// Which texture coordinates feed it (0-2).
    pub texcoord: u8,
    pub linear_filter: bool,
}

impl ProcTex {
    /// A procedural texture whose mapped value picks from `gradient`
    /// (0xAABBGGRR colours from value 0 to value 1).
    pub fn new(map: ProcMap, gradient: &[u32]) -> ProcTex {
        assert!(!gradient.is_empty() && gradient.len() <= 255, "1 to 255 gradient colours");
        ProcTex {
            u_clamp: ProcClamp::MirroredRepeat,
            v_clamp: ProcClamp::MirroredRepeat,
            rgb_map: map,
            alpha_map: None,
            u_shift: ProcShift::None,
            v_shift: ProcShift::None,
            noise: None,
            noise_lut: ProcLut::linear(),
            rgb_lut: ProcLut::linear(),
            alpha_lut: ProcLut::linear(),
            colors: gradient.to_vec(),
            texcoord: 0,
            linear_filter: true,
        }
    }

    /// Replaces the gradient.
    pub fn set_gradient(&mut self, gradient: &[u32]) {
        assert!(!gradient.is_empty() && gradient.len() <= 255, "1 to 255 gradient colours");
        self.colors = gradient.to_vec();
    }

    pub(crate) fn words(&self) -> [u32; 6] {
        let (separate, alpha) = match self.alpha_map {
            Some(a) => (1, a as u32),
            None => (0, 0),
        };
        let ampl = |n: &ProcNoise| ((n.amplitude * 4096.0) as i32 as u32) & 0xFFFF;
        let (u, v) = self.noise.unwrap_or((ProcNoise { amplitude: 0.0, frequency: 0.0, phase: 0.0 }, ProcNoise { amplitude: 0.0, frequency: 0.0, phase: 0.0 }));
        let w0 = self.u_clamp as u32
            | (self.v_clamp as u32) << 3
            | (self.rgb_map as u32) << 6
            | alpha << 10
            | separate << 14
            | (self.noise.is_some() as u32) << 15
            | (self.u_shift as u32) << 16
            | (self.v_shift as u32) << 18;
        let w1 = ampl(&u) | f32_to_f16(u.phase) << 16;
        let w2 = ampl(&v) | f32_to_f16(v.phase) << 16;
        let w3 = f32_to_f16(u.frequency) | f32_to_f16(v.frequency) << 16;
        let w4 = self.linear_filter as u32 | 0x60 << 3 | (self.colors.len() as u32 & 0xFF) << 11;
        let w5 = 0xE0_C080 << 8;
        [w0, w1, w2, w3, w4, w5]
    }

    /// The colour table and its differences.
    pub(crate) fn color_tables(&self) -> (Box<[u32; 256]>, Box<[u32; 256]>) {
        let mut colors = Box::new([0u32; 256]);
        let mut diffs = Box::new([0u32; 256]);
        colors[..self.colors.len()].copy_from_slice(&self.colors);
        let diff = |cur: u32, next: u32, pos: u32| {
            let (c, n) = ((cur >> pos) & 0xFF, (next >> pos) & 0xFF);
            (((n as i32 - c as i32) >> 1) as u32 & 0xFF) << pos
        };
        for i in 0..self.colors.len().saturating_sub(1) {
            let (c, n) = (self.colors[i], self.colors[i + 1]);
            diffs[i] = diff(c, n, 0) | diff(c, n, 8) | diff(c, n, 16) | diff(c, n, 24);
        }
        (colors, diffs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_words() {
        let mut p = ProcTex::new(ProcMap::SqrtAdd2, &[0xFF00_00FF, 0xFFFF_0000]);
        p.u_clamp = ProcClamp::Repeat;
        p.noise = Some((ProcNoise { amplitude: 0.5, frequency: 1.0, phase: 0.0 }, ProcNoise { amplitude: 0.5, frequency: 1.0, phase: 0.0 }));
        let w = p.words();
        assert_eq!(w[0] & 7, 2);
        assert_eq!((w[0] >> 6) & 0xF, 6);
        assert_eq!((w[0] >> 15) & 1, 1);
        assert_eq!(w[1] & 0xFFFF, 0x800);
        assert_eq!(w[3] & 0xFFFF, 0x3C00);
        assert_eq!((w[4] >> 11) & 0xFF, 2);
        let (_, d) = p.color_tables();
        // red 255 -> 0: (0 - 255) >> 1 = -128 -> 0x80
        assert_eq!(d[0] & 0xFF, 0x80);
        let lut = ProcLut::linear();
        assert_eq!(lut.0[0] & 0xFFF, 0);
        assert_eq!(lut.0[64] & 0xFFF, 2047);
    }
}
