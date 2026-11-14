//! Fog: the fragment colour blends towards a fog colour by a factor looked
//! up from the fragment's depth (128 entries over the depth buffer's range,
//! 1 = no fog). Depth here is the depth buffer's value, 1 at the near plane
//! and 0 at the far plane with the projections in [`crate::math`].

use citronnade_rt::math::{exp, pow};

/// A fog lookup table.
#[derive(Clone, PartialEq, Eq)]
pub struct FogLut {
    pub(crate) data: [u32; 128],
}

impl core::fmt::Debug for FogLut {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "FogLut([{:#x}, ..])", self.data[0])
    }
}

/// The eye distance of a depth buffer value (citro3d's `FogLut_CalcZ`).
fn eye_distance(depth: f32, near: f32, far: f32) -> f32 {
    far * near / (depth * (far - near) + near)
}

impl FogLut {
    /// From the factor (1 = clear, 0 = fully fogged) as a function of the
    /// eye distance, for a projection with these `near` and `far` planes.
    pub fn from_fn(near: f32, far: f32, f: impl Fn(f32) -> f32) -> FogLut {
        let mut values = [0.0f32; 129];
        for (i, v) in values.iter_mut().enumerate() {
            *v = f(eye_distance(i as f32 / 128.0, near, far)).clamp(0.0, 1.0);
        }
        let mut data = [0u32; 128];
        for i in 0..128 {
            let val = ((values[i] * 2048.0) as u32).min(0x7FF);
            let diff = ((values[i + 1] - values[i]) * 2048.0).clamp(-4096.0, 4095.0) as i32 as u32 & 0x1FFF;
            data[i] = diff | val << 13;
        }
        FogLut { data }
    }

    /// Exponential fog: `exp(-(density * distance)^gradient)`.
    pub fn exp(density: f32, gradient: f32, near: f32, far: f32) -> FogLut {
        FogLut::from_fn(near, far, |d| exp(-pow(density * d, gradient)))
    }

    /// Entry `i`'s factor (0..128, from the far plane to the near plane).
    pub fn factor(&self, i: usize) -> f32 {
        (self.data[i.min(127)] >> 13) as f32 / 2048.0
    }

    /// Linear fog from `start` (clear) to `end` (fully fogged).
    pub fn linear(start: f32, end: f32, near: f32, far: f32) -> FogLut {
        FogLut::from_fn(near, far, |d| 1.0 - (d - start) / (end - start))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linear_fog() {
        let lut = FogLut::linear(0.0, 10.0, 1.0, 10.0);
        // Entry 0 is depth 0: the far plane, fully fogged.
        assert_eq!(lut.data[0] >> 13, 0);
        // Entry 127 is near the near plane: almost clear.
        assert!(lut.data[127] >> 13 > 1700, "{:#x}", lut.data[127]);
    }
}
