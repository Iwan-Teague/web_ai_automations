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
    haystack: &RgbMatrix,
    needle: &RgbMatrix,
    tolerance: Tolerance,
    threshold: f32,
) -> Option<MatchResult> {
    let (hw, hh) = (haystack.width, haystack.height);
    let (nw, nh) = (needle.width, needle.height);

    if nw > hw || nh > hh || nw == 0 || nh == 0 {
        return None;
    }

    let total_px = nw * nh;
    let required_hits = (threshold * total_px as f32).ceil() as usize;
    let tol = tolerance.0 as i16;

    // Needle flat slice — built ONCE, shared across all Rayon threads.
    let needle_flat: &[[u8; 3]] = &needle.data;

    (0..=(hh - nh))
        .into_par_iter()
        .filter_map(|sr| {
            let mut best: Option<MatchResult> = None;

            for sc in 0..=(hw - nw) {
                let mut hits = 0usize;
                let mut remaining = total_px;
                let mut ok = true;

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
                        let candidate = MatchResult {
                            col: sc,
                            row: sr,
                            score,
                        };
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
    haystack: &GrayMatrix,
    needle: &GrayMatrix,
    tolerance: Tolerance,
    threshold: f32,
) -> Option<MatchResult> {
    let (hw, hh) = (haystack.width, haystack.height);
    let (nw, nh) = (needle.width, needle.height);

    if nw > hw || nh > hh || nw == 0 || nh == 0 {
        return None;
    }

    let total_px = nw * nh;
    let required_hits = (threshold * total_px as f32).ceil() as usize;
    let tol = tolerance.0 as i16;

    let needle_flat: &[u8] = &needle.data;

    (0..=(hh - nh))
        .into_par_iter()
        .filter_map(|sr| {
            let mut best: Option<MatchResult> = None;

            for sc in 0..=(hw - nw) {
                let mut hits = 0usize;
                let mut remaining = total_px;
                let mut ok = true;

                'rows: for r in 0..nh {
                    // Borrow contiguous row slices — better CPU prefetch.
                    let hay_start = (sr + r) * hw + sc;
                    let hay_row = &haystack.data[hay_start..hay_start + nw];
                    let ndl_row = &needle_flat[r * nw..(r + 1) * nw];

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
                        let candidate = MatchResult {
                            col: sc,
                            row: sr,
                            score,
                        };
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
    haystack: &RgbMatrix,
    needle: &RgbMatrix,
    tolerance: Tolerance,
    threshold: f32,
) -> bool {
    find_match_position_rgb(haystack, needle, tolerance, threshold).is_some()
}

/// Returns `true` if `needle` is found anywhere in `haystack` (greyscale).
pub fn matrices_match_gray(
    haystack: &GrayMatrix,
    needle: &GrayMatrix,
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
    haystack: &RgbMatrix,
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
    haystack: &GrayMatrix,
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
    haystack: &RgbMatrix,
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
    haystack: &RgbMatrix,
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
    haystack: &GrayMatrix,
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
    haystack: &RgbMatrix,
    needle: &RgbMatrix,
    tolerance: Tolerance,
    threshold: f32,
) -> Vec<MatchResult> {
    let (hw, hh) = (haystack.width, haystack.height);
    let (nw, nh) = (needle.width, needle.height);

    if nw > hw || nh > hh || nw == 0 || nh == 0 {
        return vec![];
    }

    let total_px = nw * nh;
    let required_hits = (threshold * total_px as f32).ceil() as usize;
    let tol = tolerance.0 as i16;
    let needle_flat: &[[u8; 3]] = &needle.data;

    let mut candidates: Vec<MatchResult> = (0..=(hh - nh))
        .into_par_iter()
        .flat_map(|sr| {
            let mut row_hits = Vec::new();
            for sc in 0..=(hw - nw) {
                let mut hits = 0usize;
                let mut remaining = total_px;
                let mut ok = true;

                'rows: for r in 0..nh {
                    for c in 0..nw {
                        let hay = haystack.get(sc + c, sr + r);
                        let ndl = needle_flat[r * nw + c];
                        if rgb_pixels_match(&hay, &ndl, tol) {
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
                        row_hits.push(MatchResult {
                            col: sc,
                            row: sr,
                            score,
                        });
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
    haystack: &GrayMatrix,
    needle: &GrayMatrix,
    tolerance: Tolerance,
    threshold: f32,
) -> Vec<MatchResult> {
    let (hw, hh) = (haystack.width, haystack.height);
    let (nw, nh) = (needle.width, needle.height);

    if nw > hw || nh > hh || nw == 0 || nh == 0 {
        return vec![];
    }

    let total_px = nw * nh;
    let required_hits = (threshold * total_px as f32).ceil() as usize;
    let tol = tolerance.0 as i16;
    let needle_flat: &[u8] = &needle.data;

    let mut candidates: Vec<MatchResult> = (0..=(hh - nh))
        .into_par_iter()
        .flat_map(|sr| {
            let mut row_hits = Vec::new();
            for sc in 0..=(hw - nw) {
                let mut hits = 0usize;
                let mut remaining = total_px;
                let mut ok = true;

                'rows: for r in 0..nh {
                    let hay_start = (sr + r) * hw + sc;
                    let hay_row = &haystack.data[hay_start..hay_start + nw];
                    let ndl_row = &needle_flat[r * nw..(r + 1) * nw];

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
                        row_hits.push(MatchResult {
                            col: sc,
                            row: sr,
                            score,
                        });
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
    matrix
        .data
        .iter()
        .any(|px| rgb_pixels_match(px, &target, tol))
}

/// Returns `true` if any pixel in `matrix` is within `tolerance` of `target` (greyscale).
pub fn contains_luma(matrix: &GrayMatrix, target: u8, tolerance: Tolerance) -> bool {
    let tol = tolerance.0 as i16;
    matrix
        .data
        .iter()
        .any(|&v| gray_pixels_match(v, target, tol))
}

/// Return the absolute screen coordinates of the first pixel matching `target` (RGB).
///
/// Scans row-by-row, left-to-right.
/// `screen_offset_x/y` translate local matrix coordinates to screen space.
pub fn find_color_position(
    matrix: &RgbMatrix,
    target: [u8; 3],
    tolerance: Tolerance,
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
    matrix: &GrayMatrix,
    target: u8,
    tolerance: Tolerance,
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
