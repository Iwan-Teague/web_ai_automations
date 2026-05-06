//! Capture a screen region and save it as a template PNG.
//!
//! ```
//! cargo run --bin capture_template -- <name> <x1> <y1> <x2> <y2>
//! ```
//!
//! Coordinates are in logical pixels (top-left origin), `x2`/`y2`
//! exclusive. On a Mac, **Cmd+Shift+4** shows the cursor's logical
//! pixel position before you click — perfect for measuring `(x1, y1)`
//! and `(x2, y2)`. Press `Esc` to abort the screenshot.
//!
//! What it does:
//!
//! 1. Validates the args.
//! 2. Counts down 3 s so you can switch to the target screen and move
//!    your cursor out of the capture region.
//! 3. Captures the region and saves it to
//!    `assets/macos/arena.ai/templates/<name>.png`.
//! 4. Self-test: re-captures the same region as greyscale, loads the
//!    saved PNG, and runs the same `match_any_gray` the adapter uses.
//!    Reports whether the template matches itself at the configured
//!    threshold — a sanity check that the file is usable.
//!
//! Naming:
//!
//! - Pass `<name>` without the `.png` extension; it's appended.
//! - The file lands under `assets/macos/arena.ai/templates/`. If you need
//!   another subdirectory (e.g. `popups`), use slashes:
//!   `--name popups/foo` → `assets/macos/arena.ai/templates/popups/foo.png`.
//! - Adopt the project naming convention:
//!   `{element}_{theme}_{zoom}` (e.g. `send_button_dark_100`).

use std::path::PathBuf;
use std::process::ExitCode;
use std::thread::sleep;
use std::time::Duration;

use web_ai_automation::image_matrix::capture::{capture_gray_matrix, capture_rgb_matrix};
use web_ai_automation::image_matrix::compare::match_any_gray;
use web_ai_automation::image_matrix::io::{
    load_gray_from_image, save_rgb_matrix_as_image, save_rgb_matrix_binary,
};
use web_ai_automation::image_matrix::types::Tolerance;

const ASSET_DIR: &str = "assets/macos/arena.ai/templates";
const SELF_TEST_THRESHOLD: f32 = 0.85;
const COUNTDOWN_SECS: u64 = 3;

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().collect();
    if argv.len() != 6 {
        eprintln!("usage: cargo run --bin capture_template -- <name> <x1> <y1> <x2> <y2>");
        eprintln!();
        eprintln!("  name        Output filename (no extension). Slashes allowed for subdirs.");
        eprintln!("  x1 y1       Top-left corner, logical pixels.");
        eprintln!("  x2 y2       Bottom-right corner, exclusive, logical pixels.");
        eprintln!();
        eprintln!("On macOS use Cmd+Shift+4 to read pixel coordinates without clicking.");
        return ExitCode::from(2);
    }

    let name = argv[1].clone();
    let coords: Result<Vec<i32>, _> = argv[2..6].iter().map(|s| s.parse::<i32>()).collect();
    let coords = match coords {
        Ok(v) => v,
        Err(e) => {
            eprintln!("error: coordinates must be integers ({e})");
            return ExitCode::from(2);
        }
    };
    let (x1, y1, x2, y2) = (coords[0], coords[1], coords[2], coords[3]);
    if x2 <= x1 || y2 <= y1 {
        eprintln!("error: x2 must be > x1 and y2 must be > y1");
        return ExitCode::from(2);
    }
    if name.is_empty() {
        eprintln!("error: name is empty");
        return ExitCode::from(2);
    }

    // ── Build the output path ────────────────────────────────────────────
    let mut filename = name.clone();
    if !filename.ends_with(".png") {
        filename.push_str(".png");
    }
    let out_path = PathBuf::from(ASSET_DIR).join(&filename);
    println!("target file: {}", out_path.display());
    println!(
        "region:      ({x1}, {y1}) → ({x2}, {y2})  =  {}×{} px",
        x2 - x1,
        y2 - y1
    );

    // ── Countdown ────────────────────────────────────────────────────────
    println!();
    println!("Switch to the target screen now. Move your cursor OUT of");
    println!("the capture region (the cursor would be saved into the PNG).");
    for s in (1..=COUNTDOWN_SECS).rev() {
        println!("  capturing in {s}…");
        sleep(Duration::from_secs(1));
    }

    // ── Capture + save ───────────────────────────────────────────────────
    println!("[..] capturing");
    let rgb = match capture_rgb_matrix(x1, y1, x2, y2) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("[err] capture failed: {e}");
            eprintln!("       on macOS this often means Screen Recording");
            eprintln!("       permission has not been granted yet.");
            return ExitCode::from(1);
        }
    };
    println!("    captured {}×{} pixels", rgb.width, rgb.height);

    if let Err(e) = save_rgb_matrix_as_image(&rgb, out_path.to_str().unwrap()) {
        eprintln!("[err] save failed: {e}");
        return ExitCode::from(1);
    }
    println!("[ok] saved   → {}", out_path.display());
    let matrix_path = out_path.with_extension("rgbm");
    if let Err(e) = save_rgb_matrix_binary(&rgb, &matrix_path) {
        eprintln!("[err] matrix save failed: {e}");
        return ExitCode::from(1);
    }
    println!("[ok] matrix  → {}", matrix_path.display());

    // ── Self-test ────────────────────────────────────────────────────────
    println!("[..] self-test (load PNG, recapture region, match)");
    let saved = match load_gray_from_image(out_path.to_str().unwrap()) {
        Ok(g) => g,
        Err(e) => {
            eprintln!("[err] could not reload saved PNG: {e}");
            return ExitCode::from(1);
        }
    };
    let recap = match capture_gray_matrix(x1, y1, x2, y2) {
        Ok(g) => g,
        Err(e) => {
            eprintln!("[err] re-capture failed: {e}");
            return ExitCode::from(1);
        }
    };
    let hit = match_any_gray(
        &recap,
        std::slice::from_ref(&saved),
        Tolerance::NORMAL,
        SELF_TEST_THRESHOLD,
    );
    if hit {
        println!("[ok] self-test passed — template matches at threshold {SELF_TEST_THRESHOLD:.2}");
    } else {
        println!("[warn] self-test FAILED — saved template did not match the live region");
        println!("       at threshold {SELF_TEST_THRESHOLD:.2}. The PNG was still saved; common");
        println!("       causes: the screen changed between save and self-test (animation,");
        println!("       cursor moved into the region), or the threshold is too high for");
        println!("       this element. Re-run, or tune Tolerance/threshold in the adapter.");
    }
    ExitCode::SUCCESS
}
