//! Float functions `core` lacks (it has no `floor`, `sqrt` or `sin`).
//! `sqrt` uses the VFP's square root instruction; the others are accurate
//! to a few units in the last place for typical game use (about 1e-6).

use core::f32::consts::{FRAC_PI_2, PI, TAU};

/// Square root (VFP `vsqrt.f32`).
#[inline]
pub fn sqrt(x: f32) -> f32 {
    #[cfg(target_arch = "arm")]
    {
        let r: f32;
        unsafe { core::arch::asm!("vsqrt.f32 {0}, {1}", out(sreg) r, in(sreg) x, options(pure, nomem, nostack)) };
        r
    }
    #[cfg(not(target_arch = "arm"))]
    {
        let mut g = if x > 1.0 { x } else { 1.0 };
        for _ in 0..20 {
            g = 0.5 * (g + x / g);
        }
        g
    }
}

/// The largest integer not greater than `x`.
#[inline]
pub fn floor(x: f32) -> f32 {
    if x.abs() >= 8_388_608.0 {
        return x; // already an integer
    }
    let t = x as i32 as f32;
    if t > x { t - 1.0 } else { t }
}

/// The smallest integer not less than `x`.
#[inline]
pub fn ceil(x: f32) -> f32 {
    -floor(-x)
}

/// The nearest integer (halves away from zero).
#[inline]
pub fn round(x: f32) -> f32 {
    if x >= 0.0 { floor(x + 0.5) } else { ceil(x - 0.5) }
}

/// `x - floor(x)`, in `[0, 1)`.
#[inline]
pub fn fract(x: f32) -> f32 {
    x - floor(x)
}

/// `x` modulo `m`, in `[0, m)`.
#[inline]
pub fn rem_euclid(x: f32, m: f32) -> f32 {
    let r = x - m * floor(x / m);
    if r >= m { 0.0 } else { r }
}

/// Sine (range reduction, then an odd polynomial on [-pi/2, pi/2]).
pub fn sin(x: f32) -> f32 {
    let mut x = rem_euclid(x + PI, TAU) - PI; // [-pi, pi)
    if x > FRAC_PI_2 {
        x = PI - x;
    } else if x < -FRAC_PI_2 {
        x = -PI - x;
    }
    let x2 = x * x;
    x * (1.0 + x2 * (-1.0 / 6.0 + x2 * (1.0 / 120.0 + x2 * (-1.0 / 5040.0 + x2 * (1.0 / 362_880.0)))))
}

/// Cosine.
#[inline]
pub fn cos(x: f32) -> f32 {
    sin(x + FRAC_PI_2)
}

/// Tangent.
#[inline]
pub fn tan(x: f32) -> f32 {
    sin(x) / cos(x)
}

/// Arctangent of `y / x` in the right quadrant, in `(-pi, pi]`.
pub fn atan2(y: f32, x: f32) -> f32 {
    if x == 0.0 && y == 0.0 {
        return 0.0;
    }
    let (ax, ay) = (x.abs(), y.abs());
    let (a, swapped) = if ay > ax { (ax / ay, true) } else { (ay / ax, false) };
    // atan on [0, 1]: a minimax polynomial.
    let s = a * a;
    let mut r = ((-0.046_496_475 * s + 0.159_314_22) * s - 0.327_622_76) * s * a + a;
    if swapped {
        r = FRAC_PI_2 - r;
    }
    if x < 0.0 {
        r = PI - r;
    }
    if y < 0.0 { -r } else { r }
}

/// Arctangent.
#[inline]
pub fn atan(x: f32) -> f32 {
    atan2(x, 1.0)
}

/// `e^x`.
pub fn exp(x: f32) -> f32 {
    // e^x = 2^(x / ln 2) = 2^k * 2^f
    let t = x * core::f32::consts::LOG2_E;
    let k = floor(t);
    let f = t - k;
    let p = 1.0 + f * (0.693_147_2 + f * (0.240_226_5 + f * (0.055_504_1 + f * (0.009_618_1 + f * 0.001_333_6))));
    let k = (k as i32).clamp(-126, 127);
    p * f32::from_bits(((k + 127) as u32) << 23)
}

/// Linear interpolation.
#[inline]
pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;

    #[test]
    fn close_to_std() {
        for i in -2000..2000 {
            let x = i as f32 * 0.0137;
            assert!((sin(x) - x.sin()).abs() < 2e-5, "sin {x}");
            assert!((cos(x) - x.cos()).abs() < 2e-5, "cos {x}");
            assert!((floor(x) - x.floor()).abs() == 0.0, "floor {x}");
            assert!((sqrt(x.abs()) - x.abs().sqrt()).abs() < 1e-4, "sqrt {x}");
            let y = (i as f32 * 0.31).sin() * 3.0;
            assert!((atan2(y, x) - y.atan2(x)).abs() < 1e-4, "atan2 {y} {x}");
            if x.abs() < 20.0 {
                assert!((exp(x) - x.exp()).abs() / x.exp() < 1e-5, "exp {x}");
            }
        }
    }
}

/// Degrees to radians.
#[inline]
pub fn to_radians(degrees: f32) -> f32 {
    degrees * (PI / 180.0)
}
