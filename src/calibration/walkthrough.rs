//! Interactive `cargo run -- calibrate` walkthrough.
//!
//! Prompts the user to place arena.ai's UI into each known state, then
//! captures the configured screen region and saves it via `store::save`.
//!
//! State list intentionally covers BOTH:
//!   - **Header element states** (mode dropdown showing each mode, model
//!     header showing current model) — narrow regions, used to verify
//!     "is mode == Direct?" / "is the right model selected?"
//!   - **Open menu states** — wider regions, used to verify "did the
//!     dropdown actually open?" beyond the brightness/variance heuristic.
//!   - **The captcha modal** — full-screen capture used as a rejection
//!     reference (any session screenshot scoring high against this
//!     template = block).

use std::error::Error;
use std::io::{self, BufRead, Write};

use crate::calibration::store::CalibrationStore;
use crate::image_matrix::capture::capture_gray_matrix;
use crate::webui::adapter::Region;
use crate::webui::arena::ArenaAdapter;

/// One step in the calibration walkthrough.
struct Step {
    /// Identifier — becomes `<name>.png` in the store.
    name: &'static str,
    /// Human prompt shown to the user before capture.
    instruction: &'static str,
    /// Region to capture. Resolved at runtime from `ArenaAdapter::regions`.
    region: RegionSelector,
}

#[derive(Clone, Copy)]
enum RegionSelector {
    ModeDropdown,
    ModeMenuExpanded,
    ModelDropdown,
    ModelMenuExpanded,
    SidebarExpanded,
    SidebarCollapsed,
    InputBox,
    SendOrStop,
    SendOrStopInChat,
    SendOrStopCodeMode,
    CodePanel,
    DownloadButton,
    PreviewToggle,
    CodeToggle,
    /// Full primary screen (for captcha and other full-page modals).
    FullScreen,
}

impl RegionSelector {
    fn resolve(&self, adapter: &ArenaAdapter) -> Result<Region, String> {
        let r = adapter.regions();
        Ok(match self {
            RegionSelector::ModeDropdown => r.mode_dropdown,
            RegionSelector::ModeMenuExpanded => r.mode_menu_expanded,
            RegionSelector::ModelDropdown => r.model_dropdown,
            RegionSelector::ModelMenuExpanded => r.model_menu_expanded,
            RegionSelector::SidebarExpanded => r.sidebar_expanded,
            RegionSelector::SidebarCollapsed => r.sidebar_collapsed,
            RegionSelector::InputBox => r.input_box,
            RegionSelector::SendOrStop => r.send_or_stop,
            RegionSelector::SendOrStopInChat => r.send_or_stop_inchat,
            RegionSelector::SendOrStopCodeMode => r.send_or_stop_code_mode,
            RegionSelector::CodePanel => r.code_panel,
            RegionSelector::DownloadButton => r.download_button,
            RegionSelector::PreviewToggle => r.preview_toggle,
            RegionSelector::CodeToggle => r.code_toggle,
            RegionSelector::FullScreen => {
                let (w, h) = crate::image_matrix::capture::primary_screen_size()
                    .map_err(|e| format!("primary_screen_size: {e}"))?;
                Region {
                    x1: 0,
                    y1: 0,
                    x2: w,
                    y2: h,
                }
            }
        })
    }
}

fn steps() -> Vec<Step> {
    vec![
        Step {
            name: "mode_dropdown_direct_closed",
            instruction: "Open Chrome, fullscreen on arena.ai chat app (any /c/<id> page).\n\
                 Make sure the MODE dropdown at top-left shows 'Direct ▾' (closed).",
            region: RegionSelector::ModeDropdown,
        },
        Step {
            name: "mode_dropdown_battle_closed",
            instruction: "Click the MODE dropdown and pick 'Battle Mode'. Wait for it to close.\n\
                 Header should now show 'Battle Mode ▾' (closed).",
            region: RegionSelector::ModeDropdown,
        },
        Step {
            name: "mode_dropdown_side_by_side_closed",
            instruction: "Click the MODE dropdown and pick 'Side by Side'. Header should\n\
                 now show 'Side by Side ▾' (closed).",
            region: RegionSelector::ModeDropdown,
        },
        Step {
            name: "mode_menu_open_3rows",
            instruction: "Reset to Direct mode. Click the MODE dropdown so it's OPEN, showing\n\
                 the three rows (Battle / Side by Side / Direct). Don't hover any row.",
            region: RegionSelector::ModeMenuExpanded,
        },
        Step {
            name: "model_dropdown_default_closed",
            instruction: "Close any open dropdown. The MODEL dropdown should show whichever\n\
                 model your account defaults to (closed state).",
            region: RegionSelector::ModelDropdown,
        },
        Step {
            name: "model_picker_open_unfiltered",
            instruction: "Click the MODEL dropdown so the picker is OPEN. The search input\n\
                 at top should be empty; full model list visible underneath.",
            region: RegionSelector::ModelMenuExpanded,
        },
        Step {
            name: "sidebar_collapsed",
            instruction: "Make sure the left sidebar is COLLAPSED (just the icon strip).",
            region: RegionSelector::SidebarCollapsed,
        },
        Step {
            name: "sidebar_expanded",
            instruction: "Expand the left sidebar (showing chat list + 'New Chat' button).",
            region: RegionSelector::SidebarExpanded,
        },
        Step {
            name: "input_box_empty",
            instruction: "Collapse sidebar, close any dropdowns. The chat input box at\n\
                 bottom should be empty (placeholder text only).",
            region: RegionSelector::InputBox,
        },
        Step {
            name: "input_box_with_text",
            instruction: "Type some text into the chat input (e.g. 'hello'). Don't send.",
            region: RegionSelector::InputBox,
        },
        Step {
            name: "send_button_enabled",
            instruction: "With text in the chat input, the SEND button should be enabled.",
            region: RegionSelector::SendOrStop,
        },
        Step {
            name: "send_button_inchat_enabled",
            instruction: "In an existing /c/<id> chat, type text into the bottom input.\n\
                 Capture the enabled send button at the active-chat footer.",
            region: RegionSelector::SendOrStopInChat,
        },
        Step {
            name: "send_button_code_mode_enabled",
            instruction: "In a chat with the generated website/code panel open, type text\n\
                 into the compressed left-pane input. Capture its send button.",
            region: RegionSelector::SendOrStopCodeMode,
        },
        Step {
            name: "stop_button_generating",
            instruction: "Submit a prompt and capture while Arena is actively generating,\n\
                 with the stop button visible in the send/stop slot.",
            region: RegionSelector::SendOrStopInChat,
        },
        Step {
            name: "code_panel_visible",
            instruction: "Wait for a coding prompt to complete and show the generated\n\
                 website/code panel on the right.",
            region: RegionSelector::CodePanel,
        },
        Step {
            name: "download_button_visible",
            instruction: "With the generated website/code panel visible, capture the white\n\
                 download button in the panel toolbar.",
            region: RegionSelector::DownloadButton,
        },
        Step {
            name: "preview_toggle_visible",
            instruction: "With the generated website/code panel visible, capture the preview\n\
                 toggle (eye icon) in the panel toolbar.",
            region: RegionSelector::PreviewToggle,
        },
        Step {
            name: "code_toggle_visible",
            instruction: "With the generated website/code panel visible, capture the code\n\
                 toggle (</>) in the panel toolbar.",
            region: RegionSelector::CodeToggle,
        },
        Step {
            name: "captcha_modal",
            instruction: "OPTIONAL but recommended: trigger arena's reCAPTCHA modal\n\
                 (run automation until it appears, OR navigate so it shows).\n\
                 Press Enter when the 'Security Verification' modal is fully visible.\n\
                 If you can't trigger one right now, type 'skip' and Enter.",
            region: RegionSelector::FullScreen,
        },
    ]
}

/// Run the interactive walkthrough.
pub fn run() -> Result<(), Box<dyn Error>> {
    let adapter = ArenaAdapter::new();
    let store = CalibrationStore::default_arena();
    println!();
    println!("─── Arena calibration walkthrough ───");
    println!(
        "Reference matrices will be saved to: {}",
        store.base_dir().display()
    );
    println!("For each state below: arrange Chrome as described, then press Enter.");
    println!("Type 'skip' + Enter to skip a step. Ctrl-C to abort.");
    println!();

    let stdin = io::stdin();
    let mut input = String::new();

    for (i, step) in steps().iter().enumerate() {
        println!("─── [{}/{}] {} ───", i + 1, steps().len(), step.name);
        println!("{}", step.instruction);
        print!("Press Enter when ready (or 'skip'): ");
        io::stdout().flush()?;

        input.clear();
        stdin.lock().read_line(&mut input)?;
        let trimmed = input.trim();
        if trimmed.eq_ignore_ascii_case("skip") {
            println!("  (skipped)");
            println!();
            continue;
        }

        let region = match step.region.resolve(&adapter) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("  region resolve failed: {e} — skipping");
                continue;
            }
        };
        let cap = match capture_gray_matrix(region.x1, region.y1, region.x2, region.y2) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("  capture failed: {e} — skipping");
                continue;
            }
        };
        match store.save(step.name, &cap) {
            Ok(path) => println!(
                "  saved → {}  ({}×{})",
                path.display(),
                cap.width,
                cap.height
            ),
            Err(e) => eprintln!("  save failed: {e}"),
        }
        println!();
    }

    let saved = store.list();
    println!(
        "─── Calibration complete: {} reference(s) saved ───",
        saved.len()
    );
    for s in &saved {
        println!("  {s}");
    }
    Ok(())
}
