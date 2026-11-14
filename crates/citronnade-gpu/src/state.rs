//! Fixed-function state: texture combiners, blending, depth, stencil,
//! culling.

/// Comparison functions (depth, alpha and stencil tests).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TestFunc {
    Never = 0,
    Always = 1,
    Equal = 2,
    NotEqual = 3,
    Less = 4,
    LessEqual = 5,
    Greater = 6,
    GreaterEqual = 7,
}

/// Which triangles to drop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cull {
    None = 0,
    /// Drop counter-clockwise triangles (front faces in a right-handed scene).
    FrontCcw = 1,
    /// Drop clockwise triangles.
    BackCcw = 2,
}

/// Blend equations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlendEquation {
    Add = 0,
    Subtract = 1,
    ReverseSubtract = 2,
    Min = 3,
    Max = 4,
}

/// Blend factors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlendFactor {
    Zero = 0,
    One = 1,
    SrcColor = 2,
    OneMinusSrcColor = 3,
    DstColor = 4,
    OneMinusDstColor = 5,
    SrcAlpha = 6,
    OneMinusSrcAlpha = 7,
    DstAlpha = 8,
    OneMinusDstAlpha = 9,
    ConstantColor = 10,
    OneMinusConstantColor = 11,
    ConstantAlpha = 12,
    OneMinusConstantAlpha = 13,
    SrcAlphaSaturate = 14,
}

/// How a fragment combines with the colour buffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Blend {
    pub color_eq: BlendEquation,
    pub alpha_eq: BlendEquation,
    pub src_color: BlendFactor,
    pub dst_color: BlendFactor,
    pub src_alpha: BlendFactor,
    pub dst_alpha: BlendFactor,
}

impl Blend {
    /// Ordinary transparency: `src * a + dst * (1 - a)`.
    pub const ALPHA: Blend = Blend {
        color_eq: BlendEquation::Add,
        alpha_eq: BlendEquation::Add,
        src_color: BlendFactor::SrcAlpha,
        dst_color: BlendFactor::OneMinusSrcAlpha,
        src_alpha: BlendFactor::SrcAlpha,
        dst_alpha: BlendFactor::OneMinusSrcAlpha,
    };
    /// Additive: `src * a + dst`.
    pub const ADD: Blend = Blend {
        color_eq: BlendEquation::Add,
        alpha_eq: BlendEquation::Add,
        src_color: BlendFactor::SrcAlpha,
        dst_color: BlendFactor::One,
        src_alpha: BlendFactor::Zero,
        dst_alpha: BlendFactor::One,
    };
    /// No blending: the fragment replaces the pixel.
    pub const REPLACE: Blend = Blend {
        color_eq: BlendEquation::Add,
        alpha_eq: BlendEquation::Add,
        src_color: BlendFactor::One,
        dst_color: BlendFactor::Zero,
        src_alpha: BlendFactor::One,
        dst_alpha: BlendFactor::Zero,
    };

    pub(crate) fn word(&self) -> u32 {
        self.color_eq as u32
            | (self.alpha_eq as u32) << 8
            | (self.src_color as u32) << 16
            | (self.dst_color as u32) << 20
            | (self.src_alpha as u32) << 24
            | (self.dst_alpha as u32) << 28
    }
}

/// What a fragment may write.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WriteMask(pub u8);

impl WriteMask {
    pub const RED: WriteMask = WriteMask(1);
    pub const GREEN: WriteMask = WriteMask(2);
    pub const BLUE: WriteMask = WriteMask(4);
    pub const ALPHA: WriteMask = WriteMask(8);
    pub const DEPTH: WriteMask = WriteMask(0x10);
    pub const COLOR: WriteMask = WriteMask(0xF);
    pub const ALL: WriteMask = WriteMask(0x1F);
    pub const NONE: WriteMask = WriteMask(0);
}

impl core::ops::BitOr for WriteMask {
    type Output = WriteMask;
    fn bitor(self, o: WriteMask) -> WriteMask {
        WriteMask(self.0 | o.0)
    }
}

/// Stencil operations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StencilOp {
    Keep = 0,
    Zero = 1,
    Replace = 2,
    Increment = 3,
    Decrement = 4,
    Invert = 5,
    IncrementWrap = 6,
    DecrementWrap = 7,
}

/// Where a texture combiner stage reads from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// The vertex colour.
    PrimaryColor = 0x0,
    /// Fragment lighting's primary (diffuse) colour.
    FragmentPrimaryColor = 0x1,
    /// Fragment lighting's secondary (specular) colour.
    FragmentSecondaryColor = 0x2,
    Texture0 = 0x3,
    Texture1 = 0x4,
    Texture2 = 0x5,
    /// The procedural texture.
    Texture3 = 0x6,
    /// The combiner buffer (results of earlier stages kept aside).
    PreviousBuffer = 0xD,
    /// The stage's constant colour.
    Constant = 0xE,
    /// The previous stage's result.
    Previous = 0xF,
}

/// Which part of a colour a combiner reads (for the RGB inputs).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RgbOperand {
    Color = 0x0,
    OneMinusColor = 0x1,
    Alpha = 0x2,
    OneMinusAlpha = 0x3,
    Red = 0x4,
    OneMinusRed = 0x5,
    Green = 0x8,
    OneMinusGreen = 0x9,
    Blue = 0xC,
    OneMinusBlue = 0xD,
}

/// Which part of a colour a combiner reads (for the alpha inputs).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlphaOperand {
    Alpha = 0x0,
    OneMinusAlpha = 0x1,
    Red = 0x2,
    OneMinusRed = 0x3,
    Green = 0x4,
    OneMinusGreen = 0x5,
    Blue = 0x6,
    OneMinusBlue = 0x7,
}

/// How a combiner stage combines its inputs A, B and C.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Combine {
    /// A
    Replace = 0,
    /// A * B
    Modulate = 1,
    /// A + B
    Add = 2,
    /// A + B - 0.5
    AddSigned = 3,
    /// A * C + B * (1 - C)
    Interpolate = 4,
    /// A - B
    Subtract = 5,
    /// 4 * ((A - 0.5) . (B - 0.5)) in RGB
    Dot3Rgb = 6,
    /// the same, in RGBA
    Dot3Rgba = 7,
    /// A * B + C
    MultiplyAdd = 8,
    /// (A + B) * C
    AddMultiply = 9,
}

/// Result scaling.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scale {
    X1 = 0,
    X2 = 1,
    X4 = 2,
}

/// One texture combiner stage (the PICA200 has six and no fragment
/// shaders): `func(A, B, C)` for RGB and alpha separately.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TexEnv {
    pub rgb_sources: [Source; 3],
    pub alpha_sources: [Source; 3],
    pub rgb_operands: [RgbOperand; 3],
    pub alpha_operands: [AlphaOperand; 3],
    pub rgb_func: Combine,
    pub alpha_func: Combine,
    /// The constant colour (0xAABBGGRR, see [`crate::rgba`]).
    pub constant: u32,
    pub rgb_scale: Scale,
    pub alpha_scale: Scale,
}

impl Default for TexEnv {
    fn default() -> TexEnv {
        TexEnv::PASS
    }
}

impl TexEnv {
    /// Passes the previous stage's result through.
    pub const PASS: TexEnv = TexEnv {
        rgb_sources: [Source::Previous, Source::PrimaryColor, Source::PrimaryColor],
        alpha_sources: [Source::Previous, Source::PrimaryColor, Source::PrimaryColor],
        rgb_operands: [RgbOperand::Color; 3],
        alpha_operands: [AlphaOperand::Alpha; 3],
        rgb_func: Combine::Replace,
        alpha_func: Combine::Replace,
        constant: 0xFFFF_FFFF,
        rgb_scale: Scale::X1,
        alpha_scale: Scale::X1,
    };

    /// `source` for both RGB and alpha.
    pub const fn replace(source: Source) -> TexEnv {
        let mut e = TexEnv::PASS;
        e.rgb_sources[0] = source;
        e.alpha_sources[0] = source;
        e
    }

    /// `a * b` for both RGB and alpha.
    pub const fn modulate(a: Source, b: Source) -> TexEnv {
        let mut e = TexEnv::PASS;
        e.rgb_sources = [a, b, Source::PrimaryColor];
        e.alpha_sources = [a, b, Source::PrimaryColor];
        e.rgb_func = Combine::Modulate;
        e.alpha_func = Combine::Modulate;
        e
    }

    /// Sets the sources of RGB and alpha.
    pub const fn sources(mut self, a: Source, b: Source, c: Source) -> TexEnv {
        self.rgb_sources = [a, b, c];
        self.alpha_sources = [a, b, c];
        self
    }

    pub const fn rgb_sources(mut self, a: Source, b: Source, c: Source) -> TexEnv {
        self.rgb_sources = [a, b, c];
        self
    }

    pub const fn alpha_sources(mut self, a: Source, b: Source, c: Source) -> TexEnv {
        self.alpha_sources = [a, b, c];
        self
    }

    pub const fn rgb_operands(mut self, a: RgbOperand, b: RgbOperand, c: RgbOperand) -> TexEnv {
        self.rgb_operands = [a, b, c];
        self
    }

    pub const fn alpha_operands(mut self, a: AlphaOperand, b: AlphaOperand, c: AlphaOperand) -> TexEnv {
        self.alpha_operands = [a, b, c];
        self
    }

    /// Sets the function of RGB and alpha.
    pub const fn func(mut self, f: Combine) -> TexEnv {
        self.rgb_func = f;
        self.alpha_func = f;
        self
    }

    pub const fn rgb_func(mut self, f: Combine) -> TexEnv {
        self.rgb_func = f;
        self
    }

    pub const fn alpha_func(mut self, f: Combine) -> TexEnv {
        self.alpha_func = f;
        self
    }

    pub const fn constant(mut self, color: u32) -> TexEnv {
        self.constant = color;
        self
    }

    pub const fn scale(mut self, rgb: Scale, alpha: Scale) -> TexEnv {
        self.rgb_scale = rgb;
        self.alpha_scale = alpha;
        self
    }

    /// The five register words.
    pub(crate) fn words(&self) -> [u32; 5] {
        let src = |s: &[Source; 3]| s[0] as u32 | (s[1] as u32) << 4 | (s[2] as u32) << 8;
        let op_rgb = self.rgb_operands[0] as u32 | (self.rgb_operands[1] as u32) << 4 | (self.rgb_operands[2] as u32) << 8;
        let op_a = self.alpha_operands[0] as u32 | (self.alpha_operands[1] as u32) << 4 | (self.alpha_operands[2] as u32) << 8;
        [
            src(&self.rgb_sources) | src(&self.alpha_sources) << 16,
            op_rgb | op_a << 12,
            self.rgb_func as u32 | (self.alpha_func as u32) << 16,
            self.constant,
            self.rgb_scale as u32 | (self.alpha_scale as u32) << 16,
        ]
    }
}
