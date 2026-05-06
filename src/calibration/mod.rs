//! Reference-matrix calibration store + matching engine.
//!
//! The pipeline performs **two kinds** of pixel verification:
//!
//! 1. **Same-session diff** — capture pre-state, act, capture post-state,
//!    confirm the change matches expectation. No reference needed.
//!    Implemented inline at action sites in `webui::arena::session_setup`.
//!
//! 2. **Reference match** — captured state compared to a saved reference
//!    PNG ("is the model dropdown header showing 'Direct'?"). Reference
//!    PNGs are produced by the `calibrate` subcommand and stored at
//!    `assets/macos/arena.ai/calibration/<state>.png`. Matched via tolerance-aware
//!    per-pixel score.
//!
//! Storage format = PNG so reference matrices are human-inspectable.
//! On disk → loaded as `GrayMatrix` for matching.

pub mod store;
pub mod walkthrough;

use crate::image_matrix::types::GrayMatrix;

/// Score in `[0.0, 1.0]` — fraction of pixels in `captured` that fall
/// within ±`tolerance` of the corresponding pixel in `reference`.
///
/// Returns `0.0` if dimensions don't match.
pub fn match_score(captured: &GrayMatrix, reference: &GrayMatrix, tolerance: u8) -> f32 {
    if captured.width != reference.width || captured.height != reference.height {
        return 0.0;
    }
    let n = captured.data.len();
    if n == 0 {
        return 0.0;
    }
    let mut hits: usize = 0;
    for (a, b) in captured.data.iter().zip(reference.data.iter()) {
        if a.abs_diff(*b) <= tolerance {
            hits += 1;
        }
    }
    hits as f32 / n as f32
}

/// Score same-shape matrices (any size). Wrapper that downsamples both
/// matrices to the smaller of the two dimensions before scoring — useful
/// when the captured region is slightly off in size from the reference.
///
/// For now this only handles equal sizes. Returns 0.0 on mismatch; caller
/// should ensure sizes line up at capture time.
pub fn matches_at_threshold(
    captured: &GrayMatrix,
    reference: &GrayMatrix,
    tolerance: u8,
    threshold: f32,
) -> bool {
    match_score(captured, reference, tolerance) >= threshold
}

/// Per-pixel diff count: how many pixels differ by more than `tolerance`.
/// Useful for "is this region UNCHANGED between two same-session captures?"
pub fn diff_pixel_count(a: &GrayMatrix, b: &GrayMatrix, tolerance: u8) -> usize {
    if a.width != b.width || a.height != b.height {
        return usize::MAX;
    }
    a.data
        .iter()
        .zip(b.data.iter())
        .filter(|(av, bv)| av.abs_diff(**bv) > tolerance)
        .count()
}

/// "Did this region change significantly between two same-session captures?"
/// Returns true when at least `min_change_fraction` of pixels differ by
/// more than `tolerance`.
pub fn region_changed(
    a: &GrayMatrix,
    b: &GrayMatrix,
    tolerance: u8,
    min_change_fraction: f32,
) -> bool {
    if a.width != b.width || a.height != b.height {
        return true;
    }
    let n = a.data.len();
    if n == 0 {
        return false;
    }
    let diff = diff_pixel_count(a, b, tolerance);
    (diff as f32 / n as f32) >= min_change_fraction
}
