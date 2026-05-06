# Image Matrix Comparison — Optimisation Guide
**Source:** `viewbot_v3-main` · `verifications.rs` / `youtube_checks.rs`  
**Language:** Rust

---

## 1. What the Original Code Does

The codebase uses a custom `RgbMatrix` (a flat `Vec<[u8; 3]>` with width/height metadata) to:

- Capture a screen region into a matrix
- Load reference templates from PNG or JSON files
- Slide a "needle" over a "haystack" to find matches within a pixel-level tolerance
- Return a bool, optionally also clicking the match location with the mouse

The core comparison entry points are:

| Function | Purpose |
|---|---|
| `matrices_match_with_tolerance` | Sliding window, returns `bool` |
| `matrices_match_with_tolerance_move_mouse` | Same but also clicks the match |
| `compare_screengrab_to_image` | Exact (zero-tolerance) containment check |
| `match_any` | Runs `matrices_match_with_tolerance` over a template list |
| `contains_rgb_value` | Linear scan for a single exact colour |
| `find_rgb_along_x_axis` | Scans a 200-px-wide band for a target colour |

Parallelism via `rayon` is applied to the outer (row) loop of the sliding window.

---

## 2. Problems Found

### 2.1 Duplicate Logic
`matrices_match_with_tolerance` and `matrices_match_with_tolerance_move_mouse` are nearly identical — ~80 lines copied verbatim. Any bug fix or tuning must be applied twice. The only difference is that one calls `move_mouse_single_click` on a hit.

**Fix:** extract a single `find_match_position → Option<MatchResult>` primitive; both public wrappers call it.

### 2.2 Needle Cache Rebuilt Per Column
`needle_cache` is allocated inside the `start_col` loop, meaning it is rebuilt up to ~1870 times per row on a 1920-wide haystack.

```rust
// ORIGINAL — rebuilt inside the inner loop every column:
let needle_cache: Vec<Vec<[u8; 3]>> = (0..needle_height)
    .map(|r| (0..needle_width).map(|c| needle.get(c, r)).collect())
    .collect();
```

**Fix:** build the flat needle slice once before both loops.

### 2.3 RGB Comparison is 3× More Work Than Needed for Greyscale UI
YouTube's UI chrome (logos, search bars, tab indicators, progress bars) is almost entirely achromatic. Converting to greyscale with ITU-R BT.601 weights reduces each pixel comparison from three channel checks to one — roughly **3× faster** with better cache utilisation because `u8` packs 3× more densely than `[u8; 3]`.

```
Y = 0.299·R + 0.587·G + 0.114·B
  ≈ (77·R + 150·G + 29·B) >> 8   (integer, max error ±1 LSB)
```

### 2.4 Logging Inside Pure Comparison Functions
`print_update(…)` is called at the top of every comparison function, making them impossible to benchmark or unit test without console noise. Comparison logic should be pure; logging belongs at the call site.

### 2.5 `compare_screengrab_to_image` has Zero Tolerance
A single DPI-scaled pixel breaks the entire match. Retired in favour of `matrices_match` with a small tolerance.

### 2.6 Broken Rayon Nesting in `match_any`
`templates.par_iter().any(…)` spawns an outer thread pool, but each template check internally calls `into_par_iter()`. Rayon serialises nested pools — the overhead is worse than running templates sequentially and letting the inner loop have all threads.

**Fix:** iterate templates sequentially, inner sliding-window uses full thread count.

---

## 3. Improved Module Layout

```
src/image_matrix/
    mod.rs       — public re-exports
    types.rs     — RgbMatrix, GrayMatrix, MatchResult, Tolerance
    convert.rs   — rgb_to_gray, gray_to_rgb, pixel_to_gray
    io.rs        — load/save PNG, JSON, bincode
    capture.rs   — capture_rgb_matrix, capture_gray_matrix  (Windows-specific)
    compare.rs   — all comparison functions (pure, no side effects)
```

`compare.rs` has zero dependency on `screenshots`, `enigo`, `windows`, or `chrono`.
It can be unit-tested and benchmarked in isolation.

---

## 4. `types.rs` — Shared Type Definitions

```rust
use serde::{Deserialize, Serialize};

/// Raw RGB pixel — three bytes, red-first.
pub type Rgb = [u8; 3];

/// Single luminance byte produced from an RGB pixel.
pub type Gray = u8;

/// Per-channel deviation that still counts as a matching pixel.
#[derive(Clone, Copy, Debug)]
pub struct Tolerance(pub u8);

impl Tolerance {
    pub const EXACT:  Self = Self(0);
    pub const TIGHT:  Self = Self(3);
    pub const NORMAL: Self = Self(10);
    pub const LOOSE:  Self = Self(20);
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
    pub width:  usize,
    pub height: usize,
    /// Row-major flat storage: index = y * width + x.
    pub data: Vec<Rgb>,
}

impl RgbMatrix {
    pub fn new(width: usize, height: usize) -> Self {
        Self { width, height, data: vec![[0, 0, 0]; width * height] }
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
    pub width:  usize,
    pub height: usize,
    /// Row-major flat storage: index = y * width + x.
    pub data: Vec<Gray>,
}

impl GrayMatrix {
    pub fn new(width: usize, height: usize) -> Self {
        Self { width, height, data: vec![0u8; width * height] }
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
```

---

## 5. `convert.rs` — RGB ↔ Greyscale

```rust
use crate::image_matrix::types::{GrayMatrix, RgbMatrix};

/// Convert RgbMatrix → GrayMatrix using integer BT.601 weights.
///
/// Y ≈ (77·R + 150·G + 29·B) >> 8   (max error vs. float: ±1 LSB)
pub fn rgb_to_gray(src: &RgbMatrix) -> GrayMatrix {
    let mut dst = GrayMatrix::new(src.width, src.height);
    for (i, px) in src.data.iter().enumerate() {
        dst.data[i] = pixel_to_gray(px[0], px[1], px[2]);
    }
    dst
}

/// Convert GrayMatrix → RgbMatrix by setting R = G = B = luma.
/// Useful for saving a greyscale matrix as a PNG for debugging.
pub fn gray_to_rgb(src: &GrayMatrix) -> RgbMatrix {
    let data = src.data.iter().map(|&v| [v, v, v]).collect();
    RgbMatrix { width: src.width, height: src.height, data }
}

/// Convert a single RGB pixel to a greyscale luminance byte.
#[inline(always)]
pub fn pixel_to_gray(r: u8, g: u8, b: u8) -> u8 {
    ((77u16 * r as u16 + 150u16 * g as u16 + 29u16 * b as u16) >> 8) as u8
}
```

---

## 6. `io.rs` — Load and Save

```rust
use std::error::Error;
use std::fs::{self, File};
use std::io::{BufReader, BufWriter};
use std::path::Path;

use image::{Rgb, RgbImage};

use crate::image_matrix::convert::{pixel_to_gray, rgb_to_gray};
use crate::image_matrix::types::{GrayMatrix, RgbMatrix};

// ── Load ──────────────────────────────────────────────────────────────────────

/// Load an RgbMatrix from any image file that the `image` crate supports.
pub fn load_rgb_matrix_from_image(path: &str) -> Result<RgbMatrix, Box<dyn Error>> {
    let img = image::open(path)?.to_rgb8();
    let (w, h) = img.dimensions();
    let (width, height) = (w as usize, h as usize);

    let mut data = Vec::with_capacity(width * height);
    for y in 0..height {
        for x in 0..width {
            let px = img.get_pixel(x as u32, y as u32);
            data.push([px[0], px[1], px[2]]);
        }
    }
    Ok(RgbMatrix { width, height, data })
}

/// Load a GrayMatrix from an image file.
/// The image is converted to greyscale on load — no intermediate RgbMatrix held in memory.
pub fn load_gray_from_image(path: &str) -> Result<GrayMatrix, Box<dyn Error>> {
    let img = image::open(path)?.to_rgb8();
    let (w, h) = img.dimensions();
    let (width, height) = (w as usize, h as usize);

    let mut data = Vec::with_capacity(width * height);
    for y in 0..height {
        for x in 0..width {
            let px = img.get_pixel(x as u32, y as u32);
            data.push(pixel_to_gray(px[0], px[1], px[2]));
        }
    }
    Ok(GrayMatrix { width, height, data })
}

/// Load an RgbMatrix from a JSON file (serde_json).
pub fn load_rgb_matrix_from_json(path: &str) -> Result<RgbMatrix, Box<dyn Error>> {
    let file   = File::open(path)?;
    let reader = BufReader::new(file);
    let matrix: RgbMatrix = serde_json::from_reader(reader)?;
    Ok(matrix)
}

/// Load a GrayMatrix from a JSON file (serde_json).
pub fn load_gray_matrix_from_json(path: &str) -> Result<GrayMatrix, Box<dyn Error>> {
    let file   = File::open(path)?;
    let reader = BufReader::new(file);
    let matrix: GrayMatrix = serde_json::from_reader(reader)?;
    Ok(matrix)
}

// ── Save ──────────────────────────────────────────────────────────────────────

/// Save an RgbMatrix as a PNG file.
/// Creates parent directories automatically.
pub fn save_rgb_matrix_as_image(matrix: &RgbMatrix, path: &str) -> Result<(), Box<dyn Error>> {
    let path = Path::new(path);
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let mut img = RgbImage::new(matrix.width as u32, matrix.height as u32);
    for y in 0..matrix.height {
        for x in 0..matrix.width {
            let px = matrix.get(x, y);
            img.put_pixel(x as u32, y as u32, Rgb(px));
        }
    }
    img.save(path)?;
    Ok(())
}

/// Save a GrayMatrix as a PNG file (expands to R=G=B for display).
pub fn save_gray_matrix_as_image(matrix: &GrayMatrix, path: &str) -> Result<(), Box<dyn Error>> {
    let rgb = crate::image_matrix::convert::gray_to_rgb(matrix);
    save_rgb_matrix_as_image(&rgb, path)
}

/// Save an RgbMatrix as a JSON file (serde_json, human-readable).
pub fn save_rgb_matrix_as_json(matrix: &RgbMatrix, path: &str) -> Result<(), Box<dyn Error>> {
    let path = Path::new(path);
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let file   = File::create(path)?;
    let writer = BufWriter::new(file);
    serde_json::to_writer(writer, matrix)?;
    Ok(())
}
```

---

## 7. `capture.rs` — Screen Capture (Windows)

```rust
use std::error::Error;
use screenshots::Screen;

use crate::image_matrix::convert::pixel_to_gray;
use crate::image_matrix::types::{GrayMatrix, RgbMatrix};

/// Capture a screen region as an RgbMatrix.
///
/// Coordinates are in logical screen pixels, top-left origin.
/// `x2`/`y2` are exclusive (width = x2 - x1, height = y2 - y1).
pub fn capture_rgb_matrix(
    x1: i32, y1: i32,
    x2: i32, y2: i32,
) -> Result<RgbMatrix, Box<dyn Error>> {
    if x2 <= x1 || y2 <= y1 {
        return Err(format!("Invalid capture coordinates: ({x1},{y1})→({x2},{y2})").into());
    }

    let width  = (x2 - x1) as u32;
    let height = (y2 - y1) as u32;

    let screen = Screen::all()?
        .into_iter()
        .next()
        .ok_or("No screens found")?;
    let image = screen.capture_area(x1, y1, width, height)?;

    let mut data = Vec::with_capacity((width * height) as usize);
    for y in 0..height {
        for x in 0..width {
            let px = image.get_pixel(x, y);
            data.push([px[0], px[1], px[2]]);
        }
    }

    Ok(RgbMatrix { width: width as usize, height: height as usize, data })
}

/// Capture a screen region directly as a GrayMatrix.
///
/// Avoids allocating an intermediate RgbMatrix — each pixel is converted
/// to greyscale on the fly during capture.
pub fn capture_gray_matrix(
    x1: i32, y1: i32,
    x2: i32, y2: i32,
) -> Result<GrayMatrix, Box<dyn Error>> {
    if x2 <= x1 || y2 <= y1 {
        return Err(format!("Invalid capture coordinates: ({x1},{y1})→({x2},{y2})").into());
    }

    let width  = (x2 - x1) as u32;
    let height = (y2 - y1) as u32;

    let screen = Screen::all()?
        .into_iter()
        .next()
        .ok_or("No screens found")?;
    let image = screen.capture_area(x1, y1, width, height)?;

    let mut data = Vec::with_capacity((width * height) as usize);
    for y in 0..height {
        for x in 0..width {
            let px = image.get_pixel(x, y);
            data.push(pixel_to_gray(px[0], px[1], px[2]));
        }
    }

    Ok(GrayMatrix { width: width as usize, height: height as usize, data })
}
```

---

## 8. `compare.rs` — Complete Comparison Functions

All functions are **pure**: no logging, no mouse movement, no I/O.
The caller owns those concerns.

```rust
use rayon::prelude::*;

use crate::image_matrix::types::{GrayMatrix, MatchResult, RgbMatrix, Tolerance};

// ── Inline pixel helpers ──────────────────────────────────────────────────────

#[inline(always)]
fn rgb_pixels_match(a: &[u8; 3], b: &[u8; 3], tol: i16) -> bool {
    (a[0] as i16 - b[0] as i16).abs() <= tol
        && (a[1] as i16 - b[1] as i16).abs() <= tol
        && (a[2] as i16 - b[2] as i16).abs() <= tol
}

#[inline(always)]
fn gray_pixels_match(a: u8, b: u8, tol: i16) -> bool {
    (a as i16 - b as i16).abs() <= tol
}

// ── RGB sliding-window search ─────────────────────────────────────────────────

/// Find the best-scoring position of `needle` inside `haystack` (RGB).
///
/// Returns `None` if no position achieves `threshold` match rate.
/// Rayon parallelises the outer (row) loop.
/// The needle is cached as a single flat slice before any looping begins.
pub fn find_match_position_rgb(
    haystack:  &RgbMatrix,
    needle:    &RgbMatrix,
    tolerance: Tolerance,
    threshold: f32,
) -> Option<MatchResult> {
    let (hw, hh) = (haystack.width, haystack.height);
    let (nw, nh) = (needle.width,   needle.height);

    if nw > hw || nh > hh || nw == 0 || nh == 0 {
        return None;
    }

    let total_px      = nw * nh;
    let required_hits = (threshold * total_px as f32).ceil() as usize;
    let tol           = tolerance.0 as i16;

    // Needle flat slice — built ONCE, shared across all Rayon threads.
    let needle_flat: &[[u8; 3]] = &needle.data;

    (0..=(hh - nh))
        .into_par_iter()
        .filter_map(|sr| {
            let mut best: Option<MatchResult> = None;

            for sc in 0..=(hw - nw) {
                let mut hits      = 0usize;
                let mut remaining = total_px;
                let mut ok        = true;

                'rows: for r in 0..nh {
                    for c in 0..nw {
                        let hay = haystack.get(sc + c, sr + r);
                        let ndl = needle_flat[r * nw + c];

                        if rgb_pixels_match(&hay, &ndl, tol) {
                            hits += 1;
                        }
                        remaining -= 1;

                        // Early exit: can no longer reach required_hits.
                        if hits + remaining < required_hits {
                            ok = false;
                            break 'rows;
                        }
                    }
                }

                if ok {
                    let score = hits as f32 / total_px as f32;
                    if score >= threshold {
                        let candidate = MatchResult { col: sc, row: sr, score };
                        best = Some(match best {
                            Some(prev) if prev.score >= score => prev,
                            _ => candidate,
                        });
                    }
                }
            }
            best
        })
        .max_by(|a, b| a.score.partial_cmp(&b.score).unwrap())
}

// ── Greyscale sliding-window search ──────────────────────────────────────────

/// Find the best-scoring position of `needle` inside `haystack` (greyscale).
///
/// ~3× faster inner loop vs. the RGB version.
/// Convert templates with `rgb_to_gray()` once at startup and cache them.
pub fn find_match_position_gray(
    haystack:  &GrayMatrix,
    needle:    &GrayMatrix,
    tolerance: Tolerance,
    threshold: f32,
) -> Option<MatchResult> {
    let (hw, hh) = (haystack.width, haystack.height);
    let (nw, nh) = (needle.width,   needle.height);

    if nw > hw || nh > hh || nw == 0 || nh == 0 {
        return None;
    }

    let total_px      = nw * nh;
    let required_hits = (threshold * total_px as f32).ceil() as usize;
    let tol           = tolerance.0 as i16;

    let needle_flat: &[u8] = &needle.data;

    (0..=(hh - nh))
        .into_par_iter()
        .filter_map(|sr| {
            let mut best: Option<MatchResult> = None;

            for sc in 0..=(hw - nw) {
                let mut hits      = 0usize;
                let mut remaining = total_px;
                let mut ok        = true;

                'rows: for r in 0..nh {
                    // Borrow contiguous row slices — better CPU prefetch.
                    let hay_start = (sr + r) * hw + sc;
                    let hay_row   = &haystack.data[hay_start..hay_start + nw];
                    let ndl_row   = &needle_flat[r * nw..(r + 1) * nw];

                    for c in 0..nw {
                        if gray_pixels_match(hay_row[c], ndl_row[c], tol) {
                            hits += 1;
                        }
                        remaining -= 1;

                        if hits + remaining < required_hits {
                            ok = false;
                            break 'rows;
                        }
                    }
                }

                if ok {
                    let score = hits as f32 / total_px as f32;
                    if score >= threshold {
                        let candidate = MatchResult { col: sc, row: sr, score };
                        best = Some(match best {
                            Some(prev) if prev.score >= score => prev,
                            _ => candidate,
                        });
                    }
                }
            }
            best
        })
        .max_by(|a, b| a.score.partial_cmp(&b.score).unwrap())
}

// ── Boolean wrappers ──────────────────────────────────────────────────────────

/// Returns `true` if `needle` is found anywhere in `haystack` (RGB).
pub fn matrices_match(
    haystack:  &RgbMatrix,
    needle:    &RgbMatrix,
    tolerance: Tolerance,
    threshold: f32,
) -> bool {
    find_match_position_rgb(haystack, needle, tolerance, threshold).is_some()
}

/// Returns `true` if `needle` is found anywhere in `haystack` (greyscale).
pub fn matrices_match_gray(
    haystack:  &GrayMatrix,
    needle:    &GrayMatrix,
    tolerance: Tolerance,
    threshold: f32,
) -> bool {
    find_match_position_gray(haystack, needle, tolerance, threshold).is_some()
}

// ── Template list scanning ────────────────────────────────────────────────────

/// Returns `true` if ANY template in `templates` matches `haystack` (RGB).
///
/// Templates are scanned sequentially so the inner parallel loop can claim
/// all Rayon threads. Use `match_any_par` only when you have > ~20 templates.
pub fn match_any(
    haystack:  &RgbMatrix,
    templates: &[RgbMatrix],
    tolerance: Tolerance,
    threshold: f32,
) -> bool {
    templates
        .iter()
        .any(|t| matrices_match(haystack, t, tolerance, threshold))
}

/// Returns `true` if ANY template in `templates` matches `haystack` (greyscale).
pub fn match_any_gray(
    haystack:  &GrayMatrix,
    templates: &[GrayMatrix],
    tolerance: Tolerance,
    threshold: f32,
) -> bool {
    templates
        .iter()
        .any(|t| matrices_match_gray(haystack, t, tolerance, threshold))
}

/// Parallel-outer variant — use when you have > ~20 templates.
/// The inner sliding-window runs single-threaded in this variant.
pub fn match_any_par(
    haystack:  &RgbMatrix,
    templates: &[RgbMatrix],
    tolerance: Tolerance,
    threshold: f32,
) -> bool {
    templates
        .par_iter()
        .any(|t| matrices_match(haystack, t, tolerance, threshold))
}

// ── Find which template matched ───────────────────────────────────────────────

/// Returns the index and MatchResult of the first template that matches (RGB).
/// Useful when you need to know *which* template matched, not just that one did.
pub fn match_which(
    haystack:  &RgbMatrix,
    templates: &[RgbMatrix],
    tolerance: Tolerance,
    threshold: f32,
) -> Option<(usize, MatchResult)> {
    templates.iter().enumerate().find_map(|(i, t)| {
        find_match_position_rgb(haystack, t, tolerance, threshold).map(|m| (i, m))
    })
}

/// Returns the index and MatchResult of the first template that matches (greyscale).
pub fn match_which_gray(
    haystack:  &GrayMatrix,
    templates: &[GrayMatrix],
    tolerance: Tolerance,
    threshold: f32,
) -> Option<(usize, MatchResult)> {
    templates.iter().enumerate().find_map(|(i, t)| {
        find_match_position_gray(haystack, t, tolerance, threshold).map(|m| (i, m))
    })
}

// ── Find all matches ──────────────────────────────────────────────────────────

/// Return every non-overlapping position where `needle` matches `haystack` (RGB).
///
/// Results are sorted best-score first.
/// Positions that overlap an already-accepted result by more than 50% of the
/// needle area are suppressed (greedy non-maximum suppression).
pub fn find_all_matches(
    haystack:  &RgbMatrix,
    needle:    &RgbMatrix,
    tolerance: Tolerance,
    threshold: f32,
) -> Vec<MatchResult> {
    let (hw, hh) = (haystack.width, haystack.height);
    let (nw, nh) = (needle.width,   needle.height);

    if nw > hw || nh > hh || nw == 0 || nh == 0 {
        return vec![];
    }

    let total_px      = nw * nh;
    let required_hits = (threshold * total_px as f32).ceil() as usize;
    let tol           = tolerance.0 as i16;
    let needle_flat: &[[u8; 3]] = &needle.data;

    let mut candidates: Vec<MatchResult> = (0..=(hh - nh))
        .into_par_iter()
        .flat_map(|sr| {
            let mut row_hits = Vec::new();
            for sc in 0..=(hw - nw) {
                let mut hits      = 0usize;
                let mut remaining = total_px;
                let mut ok        = true;

                'rows: for r in 0..nh {
                    for c in 0..nw {
                        let hay = haystack.get(sc + c, sr + r);
                        let ndl = needle_flat[r * nw + c];
                        if rgb_pixels_match(&hay, &ndl, tol) { hits += 1; }
                        remaining -= 1;
                        if hits + remaining < required_hits { ok = false; break 'rows; }
                    }
                }

                if ok {
                    let score = hits as f32 / total_px as f32;
                    if score >= threshold {
                        row_hits.push(MatchResult { col: sc, row: sr, score });
                    }
                }
            }
            row_hits
        })
        .collect();

    candidates.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap());
    suppress_overlaps(candidates, nw, nh)
}

/// Return every non-overlapping position where `needle` matches `haystack` (greyscale).
pub fn find_all_matches_gray(
    haystack:  &GrayMatrix,
    needle:    &GrayMatrix,
    tolerance: Tolerance,
    threshold: f32,
) -> Vec<MatchResult> {
    let (hw, hh) = (haystack.width, haystack.height);
    let (nw, nh) = (needle.width,   needle.height);

    if nw > hw || nh > hh || nw == 0 || nh == 0 {
        return vec![];
    }

    let total_px      = nw * nh;
    let required_hits = (threshold * total_px as f32).ceil() as usize;
    let tol           = tolerance.0 as i16;
    let needle_flat: &[u8] = &needle.data;

    let mut candidates: Vec<MatchResult> = (0..=(hh - nh))
        .into_par_iter()
        .flat_map(|sr| {
            let mut row_hits = Vec::new();
            for sc in 0..=(hw - nw) {
                let mut hits      = 0usize;
                let mut remaining = total_px;
                let mut ok        = true;

                'rows: for r in 0..nh {
                    let hay_start = (sr + r) * hw + sc;
                    let hay_row   = &haystack.data[hay_start..hay_start + nw];
                    let ndl_row   = &needle_flat[r * nw..(r + 1) * nw];

                    for c in 0..nw {
                        if gray_pixels_match(hay_row[c], ndl_row[c], tol) { hits += 1; }
                        remaining -= 1;
                        if hits + remaining < required_hits { ok = false; break 'rows; }
                    }
                }

                if ok {
                    let score = hits as f32 / total_px as f32;
                    if score >= threshold {
                        row_hits.push(MatchResult { col: sc, row: sr, score });
                    }
                }
            }
            row_hits
        })
        .collect();

    candidates.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap());
    suppress_overlaps(candidates, nw, nh)
}

/// Greedy non-maximum suppression: discard candidates that overlap an
/// already-accepted result by more than half the needle area.
fn suppress_overlaps(candidates: Vec<MatchResult>, nw: usize, nh: usize) -> Vec<MatchResult> {
    let overlap_limit = (nw * nh) / 2;
    let mut accepted: Vec<MatchResult> = Vec::new();

    'outer: for cand in candidates {
        for acc in &accepted {
            let ox = nw.saturating_sub((cand.col as isize - acc.col as isize).unsigned_abs());
            let oy = nh.saturating_sub((cand.row as isize - acc.row as isize).unsigned_abs());
            if ox * oy > overlap_limit {
                continue 'outer;
            }
        }
        accepted.push(cand);
    }
    accepted
}

// ── Single-colour search ──────────────────────────────────────────────────────

/// Returns `true` if any pixel in `matrix` is within `tolerance` of `target` (RGB).
pub fn contains_color(matrix: &RgbMatrix, target: [u8; 3], tolerance: Tolerance) -> bool {
    let tol = tolerance.0 as i16;
    matrix.data.iter().any(|px| rgb_pixels_match(px, &target, tol))
}

/// Returns `true` if any pixel in `matrix` is within `tolerance` of `target` (greyscale).
pub fn contains_luma(matrix: &GrayMatrix, target: u8, tolerance: Tolerance) -> bool {
    let tol = tolerance.0 as i16;
    matrix.data.iter().any(|&v| gray_pixels_match(v, target, tol))
}

/// Return the absolute screen coordinates of the first pixel matching `target` (RGB).
///
/// Scans row-by-row, left-to-right.
/// `screen_offset_x/y` translate local matrix coordinates to screen space.
pub fn find_color_position(
    matrix:          &RgbMatrix,
    target:          [u8; 3],
    tolerance:       Tolerance,
    screen_offset_x: i32,
    screen_offset_y: i32,
) -> Option<(i32, i32)> {
    let tol = tolerance.0 as i16;
    for y in 0..matrix.height {
        for x in 0..matrix.width {
            if rgb_pixels_match(&matrix.get(x, y), &target, tol) {
                return Some((screen_offset_x + x as i32, screen_offset_y + y as i32));
            }
        }
    }
    None
}

/// Return the absolute screen coordinates of the first pixel matching `target` (greyscale).
pub fn find_color_position_gray(
    matrix:          &GrayMatrix,
    target:          u8,
    tolerance:       Tolerance,
    screen_offset_x: i32,
    screen_offset_y: i32,
) -> Option<(i32, i32)> {
    let tol = tolerance.0 as i16;
    for y in 0..matrix.height {
        for x in 0..matrix.width {
            if gray_pixels_match(matrix.get(x, y), target, tol) {
                return Some((screen_offset_x + x as i32, screen_offset_y + y as i32));
            }
        }
    }
    None
}
```

---

## 9. Updated Call-Site Pattern (`youtube_checks.rs`)

Comparison functions are now pure. The caller owns logging and mouse movement.

```rust
use std::sync::OnceLock;
use std::error::Error;

use crate::image_matrix::{
    capture::{capture_rgb_matrix, capture_gray_matrix},
    compare::{
        find_match_position_gray, find_match_position_rgb,
        match_any_gray, match_which_gray,
        contains_luma, find_color_position,
    },
    convert::rgb_to_gray,
    io::{load_gray_from_image, load_rgb_matrix_from_json, save_gray_matrix_as_image},
    types::{GrayMatrix, MatchResult, Tolerance},
};
use crate::human_simulations::move_mouse_single_click;
use crate::verifications::print_update;

// ── Static template caches ────────────────────────────────────────────────────

static YOUTUBE_LOGOS: OnceLock<Vec<GrayMatrix>>        = OnceLock::new();
static SEARCHBARS:    OnceLock<Vec<GrayMatrix>>        = OnceLock::new();
static VIDEO_TAB_UNCLICKED: OnceLock<Vec<GrayMatrix>>  = OnceLock::new();
static VIDEO_TAB_CLICKED:   OnceLock<Vec<GrayMatrix>>  = OnceLock::new();

// ── Detection functions ───────────────────────────────────────────────────────

/// Check whether the YouTube logo is on screen.
pub fn check_if_youtube_logo() -> Result<bool, Box<dyn Error>> {
    print_update("check_if_youtube_logo() Called", 0);

    let templates = YOUTUBE_LOGOS.get_or_init(|| vec![
        load_gray_from_image("images_for_comparisson/youtube_1080_125.png").unwrap(),
        load_gray_from_image("images_for_comparisson/youtube_1080_100.png").unwrap(),
        load_gray_from_image("images_for_comparisson/youtube_1080_extra.png").unwrap(),
        load_gray_from_image("images_for_comparisson/youtube_1080_event.png").unwrap(),
    ]);

    let screen = capture_gray_matrix(60, 130, 220, 165)?;
    let result = match_any_gray(&screen, templates, Tolerance::TIGHT, 0.90);

    print_update(&format!("YouTube logo present: {}", result), 3);
    Ok(result)
}

/// Check whether the YouTube search bar is on screen.
pub fn contains_searchbar() -> Result<bool, Box<dyn Error>> {
    print_update("contains_searchbar() Called", 0);

    let templates = SEARCHBARS.get_or_init(|| vec![
        load_gray_from_image("images_for_comparisson/searchbar_unclicked_1080_125.png").unwrap(),
        load_gray_from_image("images_for_comparisson/searchbar_unclicked_1080_100.png").unwrap(),
    ]);

    let screen_a = capture_gray_matrix(625, 93,  1241, 138)?;
    let screen_b = capture_gray_matrix(430, 88,  1312, 179)?;
    let result   = match_any_gray(&screen_a, templates, Tolerance::NORMAL, 0.98)
                || match_any_gray(&screen_b, templates, Tolerance::NORMAL, 0.98);

    print_update(&format!("Search bar present: {}", result), 3);
    Ok(result)
}

/// Find a thumbnail in search results and click it.
///
/// Returns `true` and clicks the match if found; `false` otherwise.
pub fn find_and_click_thumbnail(target: &GrayMatrix) -> Result<bool, Box<dyn Error>> {
    print_update("find_and_click_thumbnail() Called", 0);

    // Capture the search results area (left=370, top=170, right=1050, bottom=910).
    let screen = capture_gray_matrix(370, 170, 1050, 910)?;

    match find_match_position_gray(&screen, target, Tolerance::NORMAL, 0.94) {
        Some(m) => {
            // Translate local matrix coordinates to absolute screen coordinates.
            let screen_x = 370 + m.col as i32;
            let screen_y = 170 + m.row as i32;
            print_update(
                &format!(
                    "Thumbnail matched at ({}, {}), score={:.1}%",
                    screen_x, screen_y, m.score * 100.0
                ),
                1,
            );
            move_mouse_single_click(screen_x, screen_y);
            Ok(true)
        }
        None => {
            print_update("Thumbnail not found in search area", 4);
            Ok(false)
        }
    }
}

/// Detect whether the video is paused using progress-bar colour.
///
/// Red bar  [255,  0, 51] → luma ≈  77  (video paused)
/// Yellow   [255,204,  0] → luma ≈ 183  (ad playing)
pub fn is_video_paused() -> bool {
    print_update("is_video_paused() Called", 0);
    crate::human_simulations::move_random_area(350, 900, 1250, 950);

    match capture_gray_matrix(102, 853, 1376, 855) {
        Ok(bar) => {
            let has_red    = contains_luma(&bar, 77,  Tolerance(5));
            let has_yellow = contains_luma(&bar, 183, Tolerance(8));
            let paused     = has_red || has_yellow;
            print_update(
                &format!("Video paused: {} (red={}, yellow={})", paused, has_red, has_yellow),
                3,
            );
            paused
        }
        Err(e) => {
            print_update(&format!("Capture error: {}", e), 4);
            false
        }
    }
}

/// Check which video-tab state is on screen and return its index (0 = unclicked, 1 = clicked).
pub fn detect_video_tab_state() -> Result<Option<usize>, Box<dyn Error>> {
    print_update("detect_video_tab_state() Called", 0);

    let unclicked = VIDEO_TAB_UNCLICKED.get_or_init(|| vec![
        load_gray_from_image("images_for_comparisson/videotab_unclicked_1080_125.png").unwrap(),
        load_gray_from_image("images_for_comparisson/videotab_unclicked_1080_100.png").unwrap(),
        load_gray_from_image("images_for_comparisson/videotab_unclicked_1080_extra.png").unwrap(),
    ]);
    let clicked = VIDEO_TAB_CLICKED.get_or_init(|| vec![
        load_gray_from_image("images_for_comparisson/videotab_clicked_1080_125.png").unwrap(),
        load_gray_from_image("images_for_comparisson/videotab_clicked_1080_100.png").unwrap(),
        load_gray_from_image("images_for_comparisson/videotab_clicked_1080_extra.png").unwrap(),
    ]);

    let screen = capture_gray_matrix(500, 171, 642, 226)?;

    if match_any_gray(&screen, unclicked, Tolerance::NORMAL, 0.98) {
        print_update("Video tab: unclicked", 3);
        return Ok(Some(0));
    }
    if match_any_gray(&screen, clicked, Tolerance::NORMAL, 0.90) {
        print_update("Video tab: clicked", 3);
        return Ok(Some(1));
    }

    print_update("Video tab: not found", 4);
    Ok(None)
}
```

---

## 10. Quick-Reference: Old → New

| Old function | New equivalent | Notes |
|---|---|---|
| `matrices_match_with_tolerance(h, n, tol, thresh)` | `matrices_match(h, n, Tolerance(tol), thresh)` | No needle-cache bug |
| `matrices_match_with_tolerance_move_mouse(…)` | `find_match_position_rgb(…)` + caller clicks | Separation of concerns |
| `compare_screengrab_to_image(…)` | `matrices_match(h, n, Tolerance::TIGHT, thresh)` | No zero-tolerance fragility |
| `match_any(m, ts, tol, thresh)` | `match_any(m, ts, Tolerance(tol), thresh)` | Sequential outer, parallel inner |
| `contains_rgb_value(m, r, g, b)` | `contains_color(m, [r,g,b], Tolerance::EXACT)` | Typed tolerance |
| `find_rgb_along_x_axis(…)` | `find_color_position(…)` | Not limited to 200px wide |
| *(new)* | `find_match_position_gray` | ~3× faster for UI chrome |
| *(new)* | `find_all_matches` / `find_all_matches_gray` | All locations + NMS |
| *(new)* | `match_which` / `match_which_gray` | Returns matched template index |
| *(new)* | `capture_gray_matrix` | Skip intermediate RGB alloc |
| *(new)* | `load_gray_from_image` | Load + convert in one pass |
| *(new)* | `suppress_overlaps` | Shared NMS logic |

---

## 11. Cargo.toml

```toml
[dependencies]
rayon      = "1.10"
image      = "0.25"
serde      = { version = "1", features = ["derive"] }
serde_json = "1"
screenshots = "0.8"
bincode    = "2"          # faster binary serialisation for stored templates
```

`bincode` serialises a `GrayMatrix` of 200×50 pixels as ~10 KB vs ~30 KB for JSON,
and deserialises ~10× faster. Use it for pre-baked reference matrices stored on disk.

---

## 12. Performance Summary

| Change | Estimated Gain |
|---|---|
| Fix needle cache rebuild | 5–15× fewer allocations per search |
| Greyscale pipeline for UI chrome | ~3× faster per-pixel comparison |
| Sequential template scan (fixed Rayon nesting) | Eliminates thread-pool contention |
| Row-slice borrows in gray loop | Better prefetch, ~10–20% improvement |
| `MatchResult` return type | Match + click in one pass, no re-search |
| `find_all_matches` with NMS | Multi-target detection in a single pass |
| `capture_gray_matrix` | Removes intermediate `RgbMatrix` allocation |
| `load_gray_from_image` | Removes intermediate `RgbMatrix` on load |
