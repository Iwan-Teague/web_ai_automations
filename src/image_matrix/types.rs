use serde::{Deserialize, Serialize};

/// Raw RGB pixel — three bytes, red-first.
pub type Rgb = [u8; 3];

/// Single luminance byte produced from an RGB pixel.
pub type Gray = u8;

/// Per-channel deviation that still counts as a matching pixel.
#[derive(Clone, Copy, Debug)]
pub struct Tolerance(pub u8);

impl Tolerance {
    pub const EXACT: Self = Self(0);
    pub const TIGHT: Self = Self(3);
    pub const NORMAL: Self = Self(10);
    pub const LOOSE: Self = Self(20);
}

/// Position and quality score of a successful template match.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MatchResult {
    /// Top-left column of the needle inside the haystack.
    pub col: usize,
    /// Top-left row of the needle inside the haystack.
    pub row: usize,
    /// Fraction of needle pixels within tolerance (0.0 – 1.0).
    pub score: f32,
}

// ── RGB matrix ────────────────────────────────────────────────────────────────

#[derive(Clone, Serialize, Deserialize)]
pub struct RgbMatrix {
    pub width: usize,
    pub height: usize,
    /// Row-major flat storage: index = y * width + x.
    pub data: Vec<Rgb>,
}

impl RgbMatrix {
    pub fn new(width: usize, height: usize) -> Self {
        Self {
            width,
            height,
            data: vec![[0, 0, 0]; width * height],
        }
    }

    #[inline(always)]
    pub fn get(&self, x: usize, y: usize) -> Rgb {
        self.data[y * self.width + x]
    }

    #[inline(always)]
    pub fn set(&mut self, x: usize, y: usize, value: Rgb) {
        self.data[y * self.width + x] = value;
    }
}

// ── Greyscale matrix ──────────────────────────────────────────────────────────

/// Single-channel luminance matrix.
/// 3× denser than RgbMatrix — optimal for cache and SIMD.
#[derive(Clone, Serialize, Deserialize)]
pub struct GrayMatrix {
    pub width: usize,
    pub height: usize,
    /// Row-major flat storage: index = y * width + x.
    pub data: Vec<Gray>,
}

impl GrayMatrix {
    pub fn new(width: usize, height: usize) -> Self {
        Self {
            width,
            height,
            data: vec![0u8; width * height],
        }
    }

    #[inline(always)]
    pub fn get(&self, x: usize, y: usize) -> Gray {
        self.data[y * self.width + x]
    }

    #[inline(always)]
    pub fn set(&mut self, x: usize, y: usize, value: Gray) {
        self.data[y * self.width + x] = value;
    }
}
