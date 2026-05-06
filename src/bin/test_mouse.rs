//! Diagnostic binary that proves mouse control works on the host.
//!
//! Run with `cargo run --bin test_mouse`.
//!
//! On macOS the OS will prompt for **Accessibility** permission the first
//! time it runs (System Settings → Privacy & Security → Accessibility).
//! Until granted, `enigo` silently no-ops and the cursor will not move.
//!
//! What it does:
//! 1. Reads the primary display's logical size.
//! 2. Walks the cursor around a 200×200 square at the centre of the screen.
//! 3. Performs one click at the centre of the square.
//! 4. Reads + writes the clipboard to verify clipboard plumbing.
//! 5. Reports success/failure of each step.

use std::thread::sleep;
use std::time::Duration;

use web_ai_automation::human_simulations as hs;
use web_ai_automation::image_matrix::capture::primary_screen_size;
use web_ai_automation::webui::browser_state;

fn main() {
    println!("test_mouse — diagnostic for input + clipboard plumbing.");
    println!("If this is your first run on macOS, grant Accessibility");
    println!("permission when prompted (System Settings → Privacy &");
    println!("Security → Accessibility).\n");
    println!("Move your hands away from the mouse — starting in 3s.\n");
    sleep(Duration::from_secs(3));

    // 1. Screen size.
    let (w, h) = match primary_screen_size() {
        Ok(s) => {
            println!("[ok] primary screen: {} × {} logical px", s.0, s.1);
            s
        }
        Err(e) => {
            eprintln!("[err] primary_screen_size: {e}");
            (1512, 982)
        }
    };

    // 2. Walk a square at screen centre.
    let cx = w / 2;
    let cy = h / 2;
    let radius = 150;
    let path = [
        (cx - radius, cy - radius),
        (cx + radius, cy - radius),
        (cx + radius, cy + radius),
        (cx - radius, cy + radius),
        (cx - radius, cy - radius),
        (cx, cy),
    ];

    println!("[..] walking the cursor around a {radius}×{radius}px square at ({cx}, {cy}).");
    for (i, (x, y)) in path.iter().enumerate() {
        match hs::move_mouse(*x, *y) {
            Ok(_) => println!("    step {} → ({x}, {y})", i + 1),
            Err(e) => eprintln!("    step {} → ({x}, {y}) FAILED: {e}", i + 1),
        }
        sleep(Duration::from_millis(450));
    }

    // 3. Single click at centre.
    println!("[..] left-click at centre.");
    match hs::move_mouse_single_click(cx, cy) {
        Ok(_) => println!("[ok] click executed"),
        Err(e) => eprintln!("[err] click failed: {e}"),
    }
    sleep(Duration::from_millis(300));

    // 4. Clipboard round-trip.
    let canary = "web_ai_automation::test_mouse · clipboard canary";
    println!("[..] clipboard round-trip");
    match hs::write_clipboard(canary) {
        Ok(_) => println!("    wrote canary"),
        Err(e) => eprintln!("    write_clipboard failed: {e}"),
    }
    match hs::read_clipboard() {
        Ok(read) if read == canary => println!("    read matches: ok"),
        Ok(read) => eprintln!("    read mismatch: {read:?}"),
        Err(e) => eprintln!("    read_clipboard failed: {e}"),
    }

    // 5. Browser-state probes (best-effort; don't fail the run if Chrome
    // isn't running).
    println!("[..] browser-state probes");
    match browser_state::frontmost_app_name() {
        Ok(s) => println!("    frontmost: {s}"),
        Err(e) => println!("    frontmost: (unavailable: {e})"),
    }
    match browser_state::active_chrome_url() {
        Ok(s) => println!("    chrome url: {s}"),
        Err(e) => println!("    chrome url: (unavailable: {e})"),
    }
    match browser_state::active_chrome_title() {
        Ok(s) => println!("    chrome title: {s}"),
        Err(e) => println!("    chrome title: (unavailable: {e})"),
    }

    // 6. Tab discovery — list every Chrome tab and report the count.
    println!("[..] tab discovery");
    match browser_state::list_chrome_tabs() {
        Ok(tabs) => {
            println!("    {} chrome tab(s) found", tabs.len());
            for t in tabs.iter().take(8) {
                println!(
                    "      win {} tab {}: {}",
                    t.window_index,
                    t.tab_index,
                    truncate(&t.url, 80)
                );
            }
            if tabs.len() > 8 {
                println!("      … {} more", tabs.len() - 8);
            }
        }
        Err(e) => println!("    list_chrome_tabs: (unavailable: {e})"),
    }

    // 7. Focus-by-URL probe. Searches for an arena tab without opening
    // a new one; reports whether one was found.
    println!("[..] focus_chrome_tab_by_url(\"arena.ai\")");
    match browser_state::focus_chrome_tab_by_url("arena.ai") {
        Ok(true) => println!("    matched and focused an existing arena tab"),
        Ok(false) => {
            println!("    no arena tab open right now (would fall through to new-tab open)")
        }
        Err(e) => println!("    failed: {e}"),
    }

    println!("\ndone.");
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(max - 1).collect::<String>())
    }
}
