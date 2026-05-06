//! Capture the smart-search toggle region in its current state so we can
//! crop fresh, discriminative on/off templates. Run twice: once with
//! Smart-Search ON, once with it OFF. Each run writes a timestamped PNG
//! to `outputs/debug/smart_search_<state>.png`.
//!
//! Usage:
//!   cargo run --bin recapture_smart_search -- on
//!   cargo run --bin recapture_smart_search -- off
//!
//! After both files exist, hand the paths back to the assistant which
//! crops a tight, high-contrast template for each state.

use std::path::PathBuf;
use std::time::Duration;

use web_ai_automation::image_matrix::capture::capture_rgb_matrix;
use web_ai_automation::webui::deepseek::regions::Regions;

fn main() {
    let state = std::env::args().nth(1).unwrap_or_else(|| "on".to_string());
    if state != "on" && state != "off" {
        eprintln!("usage: cargo run --bin recapture_smart_search -- on|off");
        std::process::exit(2);
    }

    // Bring Chrome forward and pause briefly so the screen settles.
    let _ = web_ai_automation::webui::browser_state::focus_app_by_name("Chrome");
    std::thread::sleep(Duration::from_millis(800));

    let regions = Regions::for_primary_screen();
    let r = regions.smart_search_scan;
    println!(
        "smart_search_scan = ({},{})-({},{})  size {}x{}",
        r.x1,
        r.y1,
        r.x2,
        r.y2,
        r.x2 - r.x1,
        r.y2 - r.y1
    );

    let cap = capture_rgb_matrix(r.x1, r.y1, r.x2, r.y2).expect("capture");
    let dir = PathBuf::from("outputs/debug");
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join(format!("smart_search_{state}.png"));
    let flat: Vec<u8> = cap
        .data
        .into_iter()
        .flat_map(|[r, g, b]| [r, g, b])
        .collect();
    image::RgbImage::from_raw(cap.width as u32, cap.height as u32, flat)
        .expect("from_raw")
        .save(&path)
        .expect("save");
    println!("saved {}", path.display());
}
