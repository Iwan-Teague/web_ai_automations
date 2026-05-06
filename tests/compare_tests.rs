use web_ai_automation::image_matrix::compare::{
    contains_color, contains_luma, find_all_matches_gray, find_match_position_gray,
    find_match_position_rgb, match_any, match_any_gray, match_which_gray, matrices_match,
    matrices_match_gray,
};
use web_ai_automation::image_matrix::convert::{pixel_to_gray, rgb_to_gray};
use web_ai_automation::image_matrix::types::{GrayMatrix, RgbMatrix, Tolerance};

// ── Fixture builders (in-memory; mirror tests/fixtures/* but no I/O) ─────────

/// Build a solid-colour RGB haystack with a needle pasted at (paste_x, paste_y).
fn rgb_haystack_with_needle(
    hw: usize,
    hh: usize,
    bg: [u8; 3],
    needle: &RgbMatrix,
    paste_x: usize,
    paste_y: usize,
) -> RgbMatrix {
    let mut h = RgbMatrix {
        width: hw,
        height: hh,
        data: vec![bg; hw * hh],
    };
    for r in 0..needle.height {
        for c in 0..needle.width {
            h.set(paste_x + c, paste_y + r, needle.get(c, r));
        }
    }
    h
}

fn rgb_solid(w: usize, h: usize, color: [u8; 3]) -> RgbMatrix {
    RgbMatrix {
        width: w,
        height: h,
        data: vec![color; w * h],
    }
}

// ── Pixel conversion ─────────────────────────────────────────────────────────

#[test]
fn pixel_to_gray_matches_bt601_within_one_lsb() {
    // White → 255, black → 0, mid-grey → ~128.
    assert_eq!(pixel_to_gray(0, 0, 0), 0);
    assert_eq!(pixel_to_gray(255, 255, 255), 255);
    let m = pixel_to_gray(128, 128, 128);
    assert!((127..=128).contains(&m), "got {m}");
}

#[test]
fn rgb_to_gray_round_trip_on_neutral_image() {
    let rgb = rgb_solid(10, 10, [200, 200, 200]);
    let gray = rgb_to_gray(&rgb);
    assert_eq!(gray.width, 10);
    assert_eq!(gray.height, 10);
    for v in &gray.data {
        assert!((199..=200).contains(v), "got {v}");
    }
}

// ── Sliding window — RGB ─────────────────────────────────────────────────────

#[test]
fn find_match_position_rgb_returns_paste_location() {
    let needle = rgb_solid(10, 8, [255, 0, 0]);
    let hay = rgb_haystack_with_needle(60, 40, [0, 0, 0], &needle, 17, 9);

    let m = find_match_position_rgb(&hay, &needle, Tolerance::EXACT, 1.0)
        .expect("needle should be found at the paste location");
    assert_eq!(m.col, 17);
    assert_eq!(m.row, 9);
    assert!((m.score - 1.0).abs() < 1e-6);
}

#[test]
fn matrices_match_rejects_when_colour_differs_beyond_tolerance() {
    let needle = rgb_solid(5, 5, [255, 0, 0]);
    let hay = rgb_solid(20, 20, [0, 0, 0]);
    assert!(!matrices_match(&hay, &needle, Tolerance::TIGHT, 0.9));
}

#[test]
fn matrices_match_accepts_within_tolerance() {
    let needle = rgb_solid(5, 5, [200, 200, 200]);
    let hay = rgb_solid(20, 20, [205, 200, 198]); // ±5 per channel
    assert!(matrices_match(&hay, &needle, Tolerance::NORMAL, 1.0));
}

#[test]
fn match_any_finds_first_template_present() {
    let red = rgb_solid(4, 4, [255, 0, 0]);
    let green = rgb_solid(4, 4, [0, 255, 0]);
    let blue = rgb_solid(4, 4, [0, 0, 255]);

    let hay = rgb_haystack_with_needle(20, 20, [0, 0, 0], &green, 3, 4);

    assert!(match_any(
        &hay,
        &[red.clone(), green.clone()],
        Tolerance::EXACT,
        0.95
    ));
    assert!(!match_any(&hay, &[red, blue], Tolerance::EXACT, 0.95));
}

// ── Sliding window — greyscale (the perf path) ───────────────────────────────

#[test]
fn find_match_position_gray_locates_paste() {
    let needle_rgb = rgb_solid(8, 6, [255, 255, 255]);
    let hay_rgb = rgb_haystack_with_needle(40, 30, [0, 0, 0], &needle_rgb, 11, 7);

    let needle = rgb_to_gray(&needle_rgb);
    let hay = rgb_to_gray(&hay_rgb);

    let m = find_match_position_gray(&hay, &needle, Tolerance::TIGHT, 1.0)
        .expect("gray match should land on the paste location");
    assert_eq!(m.col, 11);
    assert_eq!(m.row, 7);
}

#[test]
fn matrices_match_gray_rejects_when_below_threshold() {
    let needle = GrayMatrix {
        width: 4,
        height: 4,
        data: vec![255; 16],
    };
    let hay = GrayMatrix {
        width: 12,
        height: 12,
        data: vec![0; 144],
    };
    assert!(!matrices_match_gray(&hay, &needle, Tolerance::TIGHT, 0.9));
}

#[test]
fn match_any_gray_returns_true_for_any_present() {
    let a = GrayMatrix {
        width: 3,
        height: 3,
        data: vec![10; 9],
    };
    let b = GrayMatrix {
        width: 3,
        height: 3,
        data: vec![200; 9],
    };

    let mut hay = GrayMatrix {
        width: 12,
        height: 12,
        data: vec![0; 144],
    };
    // Paste b at (4,5).
    for r in 0..3 {
        for c in 0..3 {
            hay.data[(5 + r) * 12 + (4 + c)] = 200;
        }
    }
    assert!(match_any_gray(&hay, &[a, b], Tolerance::TIGHT, 0.95));
}

#[test]
fn match_which_gray_returns_correct_index() {
    let a = GrayMatrix {
        width: 2,
        height: 2,
        data: vec![50; 4],
    };
    let b = GrayMatrix {
        width: 2,
        height: 2,
        data: vec![200; 4],
    };

    let mut hay = GrayMatrix {
        width: 8,
        height: 8,
        data: vec![200; 64],
    };
    // Patch a 2x2 region to 50 at (1,1).
    for r in 0..2 {
        for c in 0..2 {
            hay.data[(1 + r) * 8 + (1 + c)] = 50;
        }
    }
    let (idx, _m) = match_which_gray(&hay, &[a, b], Tolerance::TIGHT, 0.95)
        .expect("either template should match");
    assert_eq!(idx, 0);
}

#[test]
fn find_all_matches_gray_collects_multiple_pastes() {
    let needle = GrayMatrix {
        width: 3,
        height: 3,
        data: vec![255; 9],
    };
    let mut hay = GrayMatrix {
        width: 30,
        height: 10,
        data: vec![0; 300],
    };

    // Paste at (2,2), (12,2), (22,2).
    for &px in &[2usize, 12, 22] {
        for r in 0..3 {
            for c in 0..3 {
                hay.data[(2 + r) * 30 + (px + c)] = 255;
            }
        }
    }

    let matches = find_all_matches_gray(&hay, &needle, Tolerance::TIGHT, 1.0);
    assert_eq!(matches.len(), 3);
}

// ── Single-colour search ─────────────────────────────────────────────────────

#[test]
fn contains_color_finds_target_within_tolerance() {
    let m = RgbMatrix {
        width: 4,
        height: 4,
        data: vec![[10, 10, 10]; 16],
    };
    assert!(contains_color(&m, [12, 8, 11], Tolerance::NORMAL));
    assert!(!contains_color(&m, [200, 50, 50], Tolerance::TIGHT));
}

#[test]
fn contains_luma_finds_target() {
    let m = GrayMatrix {
        width: 4,
        height: 4,
        data: vec![100; 16],
    };
    assert!(contains_luma(&m, 102, Tolerance::TIGHT));
    assert!(!contains_luma(&m, 200, Tolerance::TIGHT));
}
