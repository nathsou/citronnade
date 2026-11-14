//! Vectors and 4x4 matrices for the GPU's shaders (row-major: a shader
//! computes `dp4 out.x, m[0], v` and so on, i.e. `M * v`).

use core::ops::{Add, Mul, Neg, Sub};

use citronnade_rt::math::{cos, sin, sqrt, tan};

/// A 3D vector.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Vec3 {
    pub const ZERO: Vec3 = Vec3::new(0.0, 0.0, 0.0);

    pub const fn new(x: f32, y: f32, z: f32) -> Vec3 {
        Vec3 { x, y, z }
    }

    pub fn dot(self, o: Vec3) -> f32 {
        self.x * o.x + self.y * o.y + self.z * o.z
    }

    pub fn cross(self, o: Vec3) -> Vec3 {
        Vec3::new(self.y * o.z - self.z * o.y, self.z * o.x - self.x * o.z, self.x * o.y - self.y * o.x)
    }

    pub fn length(self) -> f32 {
        sqrt(self.dot(self))
    }

    pub fn normalize(self) -> Vec3 {
        let l = self.length();
        if l > 0.0 { self * (1.0 / l) } else { self }
    }

    pub fn extend(self, w: f32) -> Vec4 {
        Vec4::new(self.x, self.y, self.z, w)
    }
}

impl Add for Vec3 {
    type Output = Vec3;
    fn add(self, o: Vec3) -> Vec3 {
        Vec3::new(self.x + o.x, self.y + o.y, self.z + o.z)
    }
}

impl Sub for Vec3 {
    type Output = Vec3;
    fn sub(self, o: Vec3) -> Vec3 {
        Vec3::new(self.x - o.x, self.y - o.y, self.z - o.z)
    }
}

impl Mul<f32> for Vec3 {
    type Output = Vec3;
    fn mul(self, k: f32) -> Vec3 {
        Vec3::new(self.x * k, self.y * k, self.z * k)
    }
}

impl Neg for Vec3 {
    type Output = Vec3;
    fn neg(self) -> Vec3 {
        Vec3::new(-self.x, -self.y, -self.z)
    }
}

/// A 4D vector (a shader register's x, y, z, w).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vec4 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub w: f32,
}

impl Vec4 {
    pub const fn new(x: f32, y: f32, z: f32, w: f32) -> Vec4 {
        Vec4 { x, y, z, w }
    }

    pub fn dot(self, o: Vec4) -> f32 {
        self.x * o.x + self.y * o.y + self.z * o.z + self.w * o.w
    }

    pub fn to_array(self) -> [f32; 4] {
        [self.x, self.y, self.z, self.w]
    }

    pub fn xyz(self) -> Vec3 {
        Vec3::new(self.x, self.y, self.z)
    }
}

/// A 4x4 matrix, stored as rows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mat4 {
    pub rows: [Vec4; 4],
}

impl Default for Mat4 {
    fn default() -> Mat4 {
        Mat4::IDENTITY
    }
}

impl Mat4 {
    pub const ZERO: Mat4 = Mat4 { rows: [Vec4::new(0.0, 0.0, 0.0, 0.0); 4] };
    pub const IDENTITY: Mat4 = Mat4 {
        rows: [
            Vec4::new(1.0, 0.0, 0.0, 0.0),
            Vec4::new(0.0, 1.0, 0.0, 0.0),
            Vec4::new(0.0, 0.0, 1.0, 0.0),
            Vec4::new(0.0, 0.0, 0.0, 1.0),
        ],
    };

    pub fn from_rows(rows: [[f32; 4]; 4]) -> Mat4 {
        Mat4 { rows: rows.map(|r| Vec4::new(r[0], r[1], r[2], r[3])) }
    }

    pub fn get(&self, row: usize, col: usize) -> f32 {
        self.rows[row].to_array()[col]
    }

    fn set(&mut self, row: usize, col: usize, v: f32) {
        let r = &mut self.rows[row];
        match col {
            0 => r.x = v,
            1 => r.y = v,
            2 => r.z = v,
            _ => r.w = v,
        }
    }

    pub fn transpose(&self) -> Mat4 {
        let mut m = Mat4::ZERO;
        for i in 0..4 {
            for j in 0..4 {
                m.set(i, j, self.get(j, i));
            }
        }
        m
    }

    pub fn translation(x: f32, y: f32, z: f32) -> Mat4 {
        let mut m = Mat4::IDENTITY;
        m.rows[0].w = x;
        m.rows[1].w = y;
        m.rows[2].w = z;
        m
    }

    pub fn scaling(x: f32, y: f32, z: f32) -> Mat4 {
        let mut m = Mat4::IDENTITY;
        m.rows[0].x = x;
        m.rows[1].y = y;
        m.rows[2].z = z;
        m
    }

    /// Rotation around X by `a` radians.
    pub fn rotation_x(a: f32) -> Mat4 {
        let (s, c) = (sin(a), cos(a));
        Mat4::from_rows([[1.0, 0.0, 0.0, 0.0], [0.0, c, -s, 0.0], [0.0, s, c, 0.0], [0.0, 0.0, 0.0, 1.0]])
    }

    /// Rotation around Y by `a` radians.
    pub fn rotation_y(a: f32) -> Mat4 {
        let (s, c) = (sin(a), cos(a));
        Mat4::from_rows([[c, 0.0, s, 0.0], [0.0, 1.0, 0.0, 0.0], [-s, 0.0, c, 0.0], [0.0, 0.0, 0.0, 1.0]])
    }

    /// Rotation around Z by `a` radians.
    pub fn rotation_z(a: f32) -> Mat4 {
        let (s, c) = (sin(a), cos(a));
        Mat4::from_rows([[c, -s, 0.0, 0.0], [s, c, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]])
    }

    /// An orthographic projection for a 3DS screen: maps `left..right`,
    /// `bottom..top` to the screen (x along the long side) and depth
    /// `near..far` to the GPU's 0..-1 range, rotated for the screens'
    /// sideways framebuffers. Right-handed (looking down -z).
    pub fn ortho_tilt(left: f32, right: f32, bottom: f32, top: f32, near: f32, far: f32) -> Mat4 {
        let mut m = Mat4::ZERO;
        m.rows[0].y = 2.0 / (top - bottom);
        m.rows[0].w = (bottom + top) / (bottom - top);
        m.rows[1].x = 2.0 / (left - right);
        m.rows[1].w = (left + right) / (right - left);
        m.rows[2].z = 1.0 / (near - far);
        m.rows[2].w = 0.5 * (near + far) / (near - far) - 0.5;
        m.rows[3].w = 1.0;
        m
    }

    /// A pixel-space projection for a screen `width` x `height` (400x240 or
    /// 320x240) with the origin at the top left and y going down, for 2D.
    pub fn screen_2d(width: f32, height: f32) -> Mat4 {
        Mat4::ortho_tilt(0.0, width, height, 0.0, -1.0, 1.0)
    }

    /// A perspective projection for a 3DS screen: `fovy` is the vertical
    /// field of view in radians and `aspect` the width over the height
    /// (400.0 / 240.0 for the top screen). Right-handed; depth maps to the
    /// GPU's 0..-1 range.
    pub fn perspective_tilt(fovy: f32, aspect: f32, near: f32, far: f32) -> Mat4 {
        // citro3d's Mtx_PerspTilt takes the same arguments.
        let t = tan(fovy / 2.0);
        let mut m = Mat4::ZERO;
        m.rows[0].y = 1.0 / t;
        m.rows[1].x = -1.0 / (t * aspect);
        m.rows[2].w = far * near / (near - far);
        m.rows[3].z = -1.0;
        m.rows[2].z = -m.rows[3].z * near / (near - far);
        m
    }

    /// A perspective projection for one eye of the stereoscopic top screen
    /// (citro3d's `Mtx_PerspStereoTilt`): `iod` is the distance between the
    /// eyes in world units (negative for the left eye, positive for the
    /// right, usually `±slider * k`), and `screen` the distance of the plane
    /// that appears at the screen's depth: nearer objects pop out.
    pub fn perspective_stereo_tilt(fovy: f32, aspect: f32, near: f32, far: f32, iod: f32, screen: f32) -> Mat4 {
        let t = tan(fovy / 2.0);
        let ta = t * aspect;
        let shift = iod / (2.0 * screen);
        let mut m = Mat4::ZERO;
        m.rows[0].y = 1.0 / t;
        m.rows[1].x = -1.0 / ta;
        m.rows[1].w = iod / 2.0;
        m.rows[2].w = near * far / (near - far);
        m.rows[3].z = -1.0;
        m.rows[1].z = -m.rows[3].z * shift / ta;
        m.rows[2].z = -m.rows[3].z * near / (near - far);
        m
    }

    /// A view matrix looking from `eye` at `target`.
    pub fn look_at(eye: Vec3, target: Vec3, up: Vec3) -> Mat4 {
        let f = (target - eye).normalize();
        let s = f.cross(up).normalize();
        let u = s.cross(f);
        Mat4::from_rows([
            [s.x, s.y, s.z, -s.dot(eye)],
            [u.x, u.y, u.z, -u.dot(eye)],
            [-f.x, -f.y, -f.z, f.dot(eye)],
            [0.0, 0.0, 0.0, 1.0],
        ])
    }

    /// The inverse, or `None` if the matrix is singular.
    pub fn inverse(&self) -> Option<Mat4> {
        let m: [[f32; 4]; 4] = self.rows.map(|r| r.to_array());
        let mut inv = [[0.0f32; 4]; 4];
        // Cofactors.
        let s0 = m[0][0] * m[1][1] - m[1][0] * m[0][1];
        let s1 = m[0][0] * m[1][2] - m[1][0] * m[0][2];
        let s2 = m[0][0] * m[1][3] - m[1][0] * m[0][3];
        let s3 = m[0][1] * m[1][2] - m[1][1] * m[0][2];
        let s4 = m[0][1] * m[1][3] - m[1][1] * m[0][3];
        let s5 = m[0][2] * m[1][3] - m[1][2] * m[0][3];
        let c5 = m[2][2] * m[3][3] - m[3][2] * m[2][3];
        let c4 = m[2][1] * m[3][3] - m[3][1] * m[2][3];
        let c3 = m[2][1] * m[3][2] - m[3][1] * m[2][2];
        let c2 = m[2][0] * m[3][3] - m[3][0] * m[2][3];
        let c1 = m[2][0] * m[3][2] - m[3][0] * m[2][2];
        let c0 = m[2][0] * m[3][1] - m[3][0] * m[2][1];
        let det = s0 * c5 - s1 * c4 + s2 * c3 + s3 * c2 - s4 * c1 + s5 * c0;
        if det.abs() < 1e-12 {
            return None;
        }
        let d = 1.0 / det;
        inv[0][0] = (m[1][1] * c5 - m[1][2] * c4 + m[1][3] * c3) * d;
        inv[0][1] = (-m[0][1] * c5 + m[0][2] * c4 - m[0][3] * c3) * d;
        inv[0][2] = (m[3][1] * s5 - m[3][2] * s4 + m[3][3] * s3) * d;
        inv[0][3] = (-m[2][1] * s5 + m[2][2] * s4 - m[2][3] * s3) * d;
        inv[1][0] = (-m[1][0] * c5 + m[1][2] * c2 - m[1][3] * c1) * d;
        inv[1][1] = (m[0][0] * c5 - m[0][2] * c2 + m[0][3] * c1) * d;
        inv[1][2] = (-m[3][0] * s5 + m[3][2] * s2 - m[3][3] * s1) * d;
        inv[1][3] = (m[2][0] * s5 - m[2][2] * s2 + m[2][3] * s1) * d;
        inv[2][0] = (m[1][0] * c4 - m[1][1] * c2 + m[1][3] * c0) * d;
        inv[2][1] = (-m[0][0] * c4 + m[0][1] * c2 - m[0][3] * c0) * d;
        inv[2][2] = (m[3][0] * s4 - m[3][1] * s2 + m[3][3] * s0) * d;
        inv[2][3] = (-m[2][0] * s4 + m[2][1] * s2 - m[2][3] * s0) * d;
        inv[3][0] = (-m[1][0] * c3 + m[1][1] * c1 - m[1][2] * c0) * d;
        inv[3][1] = (m[0][0] * c3 - m[0][1] * c1 + m[0][2] * c0) * d;
        inv[3][2] = (-m[3][0] * s3 + m[3][1] * s1 - m[3][2] * s0) * d;
        inv[3][3] = (m[2][0] * s3 - m[2][1] * s1 + m[2][2] * s0) * d;
        Some(Mat4::from_rows(inv))
    }

    /// The rotation `q` as a matrix.
    pub fn from_quat(q: Quat) -> Mat4 {
        let Quat { x, y, z, w } = q;
        Mat4::from_rows([
            [1.0 - 2.0 * (y * y + z * z), 2.0 * (x * y - z * w), 2.0 * (x * z + y * w), 0.0],
            [2.0 * (x * y + z * w), 1.0 - 2.0 * (x * x + z * z), 2.0 * (y * z - x * w), 0.0],
            [2.0 * (x * z - y * w), 2.0 * (y * z + x * w), 1.0 - 2.0 * (x * x + y * y), 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ])
    }

    pub fn transform(&self, v: Vec4) -> Vec4 {
        Vec4::new(self.rows[0].dot(v), self.rows[1].dot(v), self.rows[2].dot(v), self.rows[3].dot(v))
    }
}

/// A rotation as a unit quaternion.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Quat {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub w: f32,
}

impl Default for Quat {
    fn default() -> Quat {
        Quat::IDENTITY
    }
}

impl Quat {
    pub const IDENTITY: Quat = Quat { x: 0.0, y: 0.0, z: 0.0, w: 1.0 };

    /// A rotation of `angle` radians around `axis`.
    pub fn from_axis_angle(axis: Vec3, angle: f32) -> Quat {
        let a = axis.normalize() * sin(angle / 2.0);
        Quat { x: a.x, y: a.y, z: a.z, w: cos(angle / 2.0) }
    }

    pub fn length(self) -> f32 {
        sqrt(self.x * self.x + self.y * self.y + self.z * self.z + self.w * self.w)
    }

    pub fn normalize(self) -> Quat {
        let l = self.length();
        if l > 0.0 { Quat { x: self.x / l, y: self.y / l, z: self.z / l, w: self.w / l } } else { Quat::IDENTITY }
    }

    pub fn conjugate(self) -> Quat {
        Quat { x: -self.x, y: -self.y, z: -self.z, w: self.w }
    }

    /// Rotates a vector.
    pub fn rotate(self, v: Vec3) -> Vec3 {
        let q = Vec3::new(self.x, self.y, self.z);
        let t = q.cross(v) * 2.0;
        v + t * self.w + q.cross(t)
    }

    /// Spherical interpolation from `self` (t = 0) to `o` (t = 1).
    pub fn slerp(self, o: Quat, t: f32) -> Quat {
        let mut d = self.x * o.x + self.y * o.y + self.z * o.z + self.w * o.w;
        let o = if d < 0.0 {
            d = -d;
            Quat { x: -o.x, y: -o.y, z: -o.z, w: -o.w }
        } else {
            o
        };
        if d > 0.9995 {
            let l = |a: f32, b: f32| a + (b - a) * t;
            return Quat { x: l(self.x, o.x), y: l(self.y, o.y), z: l(self.z, o.z), w: l(self.w, o.w) }.normalize();
        }
        let theta = citronnade_rt::math::atan2(sqrt(1.0 - d * d), d);
        let s = sin(theta);
        let (a, b) = (sin((1.0 - t) * theta) / s, sin(t * theta) / s);
        Quat { x: self.x * a + o.x * b, y: self.y * a + o.y * b, z: self.z * a + o.z * b, w: self.w * a + o.w * b }
    }
}

impl Mul for Quat {
    type Output = Quat;
    /// `self * o` rotates by `o`, then by `self`.
    fn mul(self, o: Quat) -> Quat {
        Quat {
            w: self.w * o.w - self.x * o.x - self.y * o.y - self.z * o.z,
            x: self.w * o.x + self.x * o.w + self.y * o.z - self.z * o.y,
            y: self.w * o.y - self.x * o.z + self.y * o.w + self.z * o.x,
            z: self.w * o.z + self.x * o.y - self.y * o.x + self.z * o.w,
        }
    }
}

impl Mul for Mat4 {
    type Output = Mat4;
    fn mul(self, o: Mat4) -> Mat4 {
        let mut m = Mat4::ZERO;
        for i in 0..4 {
            for j in 0..4 {
                let mut s = 0.0;
                for k in 0..4 {
                    s += self.get(i, k) * o.get(k, j);
                }
                m.set(i, j, s);
            }
        }
        m
    }
}

impl Mul<Vec4> for Mat4 {
    type Output = Vec4;
    fn mul(self, v: Vec4) -> Vec4 {
        self.transform(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn screen_2d_maps_corners() {
        // The top-left pixel of a 400x240 screen lands at clip (y=+1? no):
        // after the tilt, clip x runs along the framebuffer's 240 side.
        let m = Mat4::screen_2d(400.0, 240.0);
        let tl = m * Vec4::new(0.0, 0.0, 0.0, 1.0);
        let br = m * Vec4::new(400.0, 240.0, 0.0, 1.0);
        // x on screen becomes -y in clip space (rotated), y becomes x.
        assert!(close(tl.y, 1.0) && close(tl.x, 1.0), "{tl:?}");
        assert!(close(br.y, -1.0) && close(br.x, -1.0), "{br:?}");
        // depth 0 maps into the GPU's [-1, 0] range
        assert!(tl.z <= 0.0 && tl.z >= -1.0);
    }

    #[test]
    fn inverse_and_quaternions() {
        let m = Mat4::translation(1.0, -2.0, 3.0) * Mat4::rotation_y(0.7) * Mat4::scaling(2.0, 3.0, 4.0);
        let p = m * m.inverse().unwrap();
        for i in 0..4 {
            for j in 0..4 {
                assert!(close(p.get(i, j), if i == j { 1.0 } else { 0.0 }), "{p:?}");
            }
        }
        let q = Quat::from_axis_angle(Vec3::new(0.0, 0.0, 1.0), core::f32::consts::FRAC_PI_2);
        let v = q.rotate(Vec3::new(1.0, 0.0, 0.0));
        assert!(close(v.x, 0.0) && close(v.y, 1.0), "{v:?}");
        let r = Mat4::from_quat(q) * Vec4::new(1.0, 0.0, 0.0, 1.0);
        assert!(close(r.x, 0.0) && close(r.y, 1.0));
        let half = Quat::IDENTITY.slerp(q, 0.5).rotate(Vec3::new(1.0, 0.0, 0.0));
        assert!(close(half.x, half.y));
    }

    #[test]
    fn products() {
        let m = Mat4::translation(1.0, 2.0, 3.0) * Mat4::scaling(2.0, 2.0, 2.0);
        let v = m * Vec4::new(1.0, 1.0, 1.0, 1.0);
        assert_eq!(v, Vec4::new(3.0, 4.0, 5.0, 1.0));
        assert_eq!(Mat4::IDENTITY * m, m);
        assert_eq!(m.transpose().transpose(), m);
    }
}
