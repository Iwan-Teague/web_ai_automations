//! Bounded screen regions for Arena.ai, calibrated against the screenshots
//! you captured at 1512×982 logical (3024×1964 physical Retina).
//!
//! Regions are stored as a struct so we can carry multiple presets and
//! pick at runtime based on the host's primary display resolution. A
//! Windows 1920×1080 preset is supplied as a proportional scale of the
//! Mac calibration — not as accurate as a real recapture, but a sensible
//! starting point until you re-calibrate on Windows.
//!
//! Click coordinates are computed by callers using `Region::center()`.
//! Don't bake fixed click points into the adapter — when the layout
//! shifts you only need to retune one rectangle here.

use crate::session::PromptMode;
use crate::webui::adapter::Region;

/// Arena has materially different UI surfaces for Code and Text filters.
/// Code is the calibrated surface today; Text is a scaffold until the
/// matching screenshots/templates are captured.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArenaSurface {
    Code,
    Text,
}

impl ArenaSurface {
    pub fn from_prompt_mode(mode: PromptMode) -> Self {
        match mode {
            PromptMode::Coding => Self::Code,
            PromptMode::Research => Self::Text,
        }
    }
}

/// All bounded rectangles the Arena adapter looks at.
#[derive(Clone, Copy, Debug)]
pub struct Regions {
    pub surface: ArenaSurface,
    pub screen_width: i32,
    pub screen_height: i32,

    /// Sidebar when expanded (left column with "New Chat", chat list).
    pub sidebar_expanded: Region,
    /// Sidebar when collapsed to icons only.
    pub sidebar_collapsed: Region,
    /// Catch-all toggle region (covers both states' positions). Kept for
    /// backwards compatibility with `start_new_chat` fallbacks.
    pub sidebar_toggle: Region,
    /// Sidebar collapse/expand toggle when sidebar is currently EXPANDED.
    /// Sits at the right edge of the sidebar header.
    pub sidebar_toggle_when_expanded: Region,
    /// Sidebar collapse/expand toggle when sidebar is currently COLLAPSED.
    /// The single icon at the very top of the sidebar's icon column.
    pub sidebar_toggle_when_collapsed: Region,

    /// **Narrow** rectangle around the New-Chat button when the sidebar
    /// is collapsed. Critical that this be tight — otherwise a 40-px-
    /// tall icon template false-matches lower in the sidebar (e.g.
    /// the Terms-of-Use footer) due to dark-background uniformity.
    pub new_chat_button_when_collapsed: Region,
    /// Same, for the expanded sidebar — the row containing the
    /// "+ New Chat" label.
    pub new_chat_button_when_expanded: Region,

    /// "Direct ▾ / Battle Mode ▾ / Side by Side ▾" dropdown header,
    /// calibrated for **collapsed-sidebar** layout (which session_setup
    /// forces). Position shifts right when sidebar is expanded.
    pub mode_dropdown: Region,
    /// Same dropdown header when sidebar is EXPANDED — shifted right by
    /// the full sidebar width (~220px).
    pub mode_dropdown_expanded: Region,
    /// Wider rectangle covering the dropdown menu's expansion below the
    /// header — searched by template-match after the header is clicked.
    pub mode_menu_expanded: Region,
    /// Model picker dropdown header, collapsed-sidebar layout.
    pub model_dropdown: Region,
    /// Model picker dropdown header when sidebar is EXPANDED.
    pub model_dropdown_expanded: Region,
    /// Wider rectangle covering the model picker's expansion below the header.
    /// Searched by screenshot after the header is clicked to verify open state.
    pub model_menu_expanded: Region,

    /// Whole-page text input area (covers placeholder + bottom action row).
    pub input_box: Region,
    /// Sub-region of `input_box` where the user clicks to focus the field.
    pub input_focus_point: Region,
    /// Same, for an active chat (input at the bottom of the screen).
    pub input_focus_point_inchat: Region,
    /// Where the send/stop button lives (right edge of input box) on
    /// the FRESH-CHAT page (input is in the middle of the screen).
    pub send_or_stop: Region,
    /// Send/stop button when IN AN ACTIVE CHAT — input moves to the
    /// bottom of the screen, so the button is ~280px lower.
    pub send_or_stop_inchat: Region,

    /// Region where the greeting "What would you like to do?" sits — used
    /// as a heuristic for the home/empty state.
    pub greeting: Region,

    /// Conversation column once a chat is active.
    pub conversation: Region,

    /// Region where per-message footer icons appear (copy, regenerate, etc.)
    /// on the FRESH-CHAT page.
    pub message_footer: Region,
    /// Same, when in an active chat (footer moves down with the input).
    pub message_footer_inchat: Region,

    /// Region for error banners ("Failed to generate", "Try again").
    pub error_banner: Region,

    /// Region for the conversation-too-long modal, centred on screen.
    pub chat_full_modal: Region,

    // ── Code-mode right panel ───────────────────────────────────────────
    //
    // These regions ONLY exist when the AI has generated a code project
    // and Arena has revealed the right-hand panel. Detection of "is the
    // right panel visible?" is by template-matching the panel toolbar
    // (preview_toggle anchor); the regions below assume it's open.
    /// Whole right panel (preview/source/files area).
    pub code_panel: Region,

    /// Preview view toggle (eye icon) — leftmost icon at the top of the
    /// right panel. Click to switch to live-preview mode.
    pub preview_toggle: Region,

    /// Source view toggle (`</>` icon) — second icon at the top of the
    /// right panel. Click to switch to file/code-tree mode.
    pub code_toggle: Region,

    /// Download button (top-right of the right panel). Saves the
    /// generated project as a zip.
    pub download_button: Region,

    // ── Code-mode input area ───────────────────────────────────────────
    //
    // When the right panel is open, the chat pane compresses to ~55–815px.
    // The send/stop button shifts left accordingly.
    /// Send/stop button position when the code panel is open (left pane
    /// only, near bottom-right of the narrower chat column).
    pub send_or_stop_code_mode: Region,

    /// Input focus point when code panel is open — the textarea sits at
    /// the bottom of the narrower left chat pane (~55–470px wide).
    pub input_focus_point_code_mode: Region,

    /// Scan region for the file-upload (paperclip) button on the fresh-chat page.
    /// Covers the area just left of the send button action row.
    pub attach_scan_fresh: Region,

    /// Scan region for the paperclip in an active chat.
    pub attach_scan_inchat: Region,
}

impl Regions {
    /// Calibration measured from the user's macOS screenshots (1512×982 logical).
    pub fn mac_1512x982() -> Self {
        Self::mac_1512x982_code()
    }

    /// Calibrated Code-filter layout. These regions back the currently
    /// working matrix comparisons for send/stop/download/code-panel states.
    pub fn mac_1512x982_code() -> Self {
        Self {
            surface: ArenaSurface::Code,
            screen_width: 1512,
            screen_height: 982,

            sidebar_expanded: Region {
                x1: 0,
                y1: 130,
                x2: 220,
                y2: 982,
            },
            sidebar_collapsed: Region {
                x1: 0,
                y1: 130,
                x2: 55,
                y2: 982,
            },
            sidebar_toggle: Region {
                x1: 0,
                y1: 130,
                x2: 230,
                y2: 170,
            },
            sidebar_toggle_when_expanded: Region {
                x1: 180,
                y1: 130,
                x2: 230,
                y2: 170,
            },
            sidebar_toggle_when_collapsed: Region {
                x1: 0,
                y1: 130,
                x2: 55,
                y2: 170,
            },

            // New-Chat button — second icon down in the sidebar (after
            // the toggle). Calibrated against the user's screenshots:
            // collapsed icon centred ~y=195; expanded row centred ~y=200.
            new_chat_button_when_collapsed: Region {
                x1: 0,
                y1: 170,
                x2: 55,
                y2: 230,
            },
            new_chat_button_when_expanded: Region {
                x1: 0,
                y1: 175,
                x2: 220,
                y2: 230,
            },

            mode_dropdown: Region {
                x1: 75,
                y1: 128,
                x2: 165,
                y2: 158,
            },
            mode_dropdown_expanded: Region {
                x1: 240,
                y1: 120,
                x2: 330,
                y2: 155,
            },
            mode_menu_expanded: Region {
                x1: 60,
                y1: 155,
                x2: 360,
                y2: 420,
            },
            model_dropdown: Region {
                x1: 180,
                y1: 128,
                x2: 270,
                y2: 158,
            },
            model_dropdown_expanded: Region {
                x1: 340,
                y1: 120,
                x2: 500,
                y2: 155,
            },
            model_menu_expanded: Region {
                x1: 160,
                y1: 155,
                x2: 600,
                y2: 700,
            },

            input_box: Region {
                x1: 400,
                y1: 535,
                x2: 1170,
                y2: 720,
            },
            input_focus_point: Region {
                x1: 430,
                y1: 537,
                x2: 1140,
                y2: 565,
            },
            input_focus_point_inchat: Region {
                x1: 380,
                y1: 910,
                x2: 990,
                y2: 940,
            },
            // Fresh-code input send button sits lower than the first
            // calibration. The textarea itself is mid-page, but the action
            // row is near the bottom-right of that input container.
            send_or_stop: Region {
                x1: 1110,
                y1: 725,
                x2: 1170,
                y2: 785,
            },
            send_or_stop_inchat: Region {
                x1: 1015,
                y1: 910,
                x2: 1065,
                y2: 950,
            },

            greeting: Region {
                x1: 490,
                y1: 460,
                x2: 1020,
                y2: 510,
            },
            conversation: Region {
                x1: 240,
                y1: 165,
                x2: 1170,
                y2: 700,
            },
            message_footer: Region {
                x1: 240,
                y1: 600,
                x2: 1170,
                y2: 720,
            },
            message_footer_inchat: Region {
                x1: 240,
                y1: 850,
                x2: 1170,
                y2: 960,
            },

            error_banner: Region {
                x1: 240,
                y1: 165,
                x2: 1170,
                y2: 320,
            },
            chat_full_modal: Region {
                x1: 1512 / 2 - 320,
                y1: 982 / 2 - 240,
                x2: 1512 / 2 + 320,
                y2: 982 / 2 + 240,
            },

            // Code-mode right panel. Current Arena layout compresses chat
            // to the left ~490px and starts the generated preview/code
            // panel immediately to its right.
            code_panel: Region {
                x1: 490,
                y1: 130,
                x2: 1512,
                y2: 982,
            },
            preview_toggle: Region {
                x1: 500,
                y1: 130,
                x2: 545,
                y2: 165,
            },
            code_toggle: Region {
                x1: 545,
                y1: 130,
                x2: 595,
                y2: 165,
            },
            download_button: Region {
                x1: 900,
                y1: 130,
                x2: 1020,
                y2: 165,
            },

            // When the generated preview/code panel is open, Arena
            // compresses the chat input to the left pane. The send/stop
            // button is the small square at the input's right edge, around
            // x≈452,y≈934 on 1512×982.
            send_or_stop_code_mode: Region {
                x1: 425,
                y1: 905,
                x2: 485,
                y2: 965,
            },

            input_focus_point_code_mode: Region {
                x1: 100,
                y1: 910,
                x2: 420,
                y2: 940,
            },

            // Paperclip scan: same y-band as fresh send (1110–1170, 725–785),
            // shifted left. Typically 50–80px to the left of the send button.
            attach_scan_fresh: Region {
                x1: 1040,
                y1: 725,
                x2: 1110,
                y2: 785,
            },

            // In-chat send is at (1015–1065, 910–950); paperclip just left.
            attach_scan_inchat: Region {
                x1: 945,
                y1: 910,
                x2: 1015,
                y2: 950,
            },
        }
    }

    /// Scaffold for Arena's Text-filter layout. Keep this separate from
    /// Code so new text screenshots can retune only this branch without
    /// weakening the working code-view matrices.
    pub fn mac_1512x982_text_scaffold() -> Self {
        let mut regions = Self::mac_1512x982_code();
        regions.surface = ArenaSurface::Text;

        // Text-filter active chat from 3024×1964 Retina screenshots
        // (logical coords are physical / 2). Keep Code view untouched.
        regions.input_box = Region {
            x1: 380,
            y1: 830,
            x2: 1190,
            y2: 960,
        };
        regions.input_focus_point_inchat = Region {
            x1: 400,
            y1: 845,
            x2: 980,
            y2: 885,
        };
        regions.send_or_stop_inchat = Region {
            x1: 1135,
            y1: 910,
            x2: 1185,
            y2: 960,
        };
        regions.conversation = Region {
            x1: 360,
            y1: 170,
            x2: 1200,
            y2: 835,
        };
        regions.message_footer = Region {
            x1: 360,
            y1: 250,
            x2: 620,
            y2: 835,
        };
        regions.message_footer_inchat = regions.message_footer;

        // Text picker is wider than Code picker when filter tabs are visible.
        regions.model_menu_expanded = Region {
            x1: 185,
            y1: 160,
            x2: 690,
            y2: 570,
        };

        regions
    }

    /// Linear scaling of the Mac calibration to a Windows 1920×1080 logical
    /// reference. **Recapture this on real Windows hardware** before
    /// trusting the click coords for production runs.
    pub fn win_1920x1080() -> Self {
        Self::win_1920x1080_for(ArenaSurface::Code)
    }

    pub fn win_1920x1080_for(surface: ArenaSurface) -> Self {
        let base = match surface {
            ArenaSurface::Code => Self::mac_1512x982_code(),
            ArenaSurface::Text => Self::mac_1512x982_text_scaffold(),
        };
        scale(base, 1920, 1080)
    }

    /// Pick the closest preset for the current screen size. Falls back to
    /// the Mac preset when neither calibration is a great fit.
    pub fn for_screen(width: i32, height: i32) -> Self {
        Self::for_screen_and_surface(width, height, ArenaSurface::Code)
    }

    pub fn for_screen_and_surface(width: i32, height: i32, surface: ArenaSurface) -> Self {
        // Crude classifier — exact match wins, otherwise pick by aspect.
        if (width, height) == (1512, 982) {
            return match surface {
                ArenaSurface::Code => Self::mac_1512x982_code(),
                ArenaSurface::Text => Self::mac_1512x982_text_scaffold(),
            };
        }
        if (width, height) == (1920, 1080) {
            return Self::win_1920x1080_for(surface);
        }

        // Fall back to the closest by area.
        let mac_area = 1512 * 982;
        let win_area = 1920 * 1080;
        let host_area = width * height;
        if (host_area - mac_area).abs() <= (host_area - win_area).abs() {
            match surface {
                ArenaSurface::Code => Self::mac_1512x982_code(),
                ArenaSurface::Text => Self::mac_1512x982_text_scaffold(),
            }
        } else {
            Self::win_1920x1080_for(surface)
        }
    }
}

/// Per-rectangle linear scale.
fn scale(r: Regions, w: i32, h: i32) -> Regions {
    let sx = |v: i32| (v as f32 * w as f32 / r.screen_width as f32).round() as i32;
    let sy = |v: i32| (v as f32 * h as f32 / r.screen_height as f32).round() as i32;
    let s = |reg: Region| Region {
        x1: sx(reg.x1),
        y1: sy(reg.y1),
        x2: sx(reg.x2),
        y2: sy(reg.y2),
    };
    Regions {
        surface: r.surface,
        screen_width: w,
        screen_height: h,
        sidebar_expanded: s(r.sidebar_expanded),
        sidebar_collapsed: s(r.sidebar_collapsed),
        sidebar_toggle: s(r.sidebar_toggle),
        sidebar_toggle_when_expanded: s(r.sidebar_toggle_when_expanded),
        sidebar_toggle_when_collapsed: s(r.sidebar_toggle_when_collapsed),
        new_chat_button_when_collapsed: s(r.new_chat_button_when_collapsed),
        new_chat_button_when_expanded: s(r.new_chat_button_when_expanded),
        mode_dropdown: s(r.mode_dropdown),
        mode_dropdown_expanded: s(r.mode_dropdown_expanded),
        mode_menu_expanded: s(r.mode_menu_expanded),
        model_dropdown: s(r.model_dropdown),
        model_dropdown_expanded: s(r.model_dropdown_expanded),
        model_menu_expanded: s(r.model_menu_expanded),
        input_box: s(r.input_box),
        input_focus_point: s(r.input_focus_point),
        input_focus_point_inchat: s(r.input_focus_point_inchat),
        send_or_stop: s(r.send_or_stop),
        send_or_stop_inchat: s(r.send_or_stop_inchat),
        greeting: s(r.greeting),
        conversation: s(r.conversation),
        message_footer: s(r.message_footer),
        message_footer_inchat: s(r.message_footer_inchat),
        error_banner: s(r.error_banner),
        chat_full_modal: s(r.chat_full_modal),
        code_panel: s(r.code_panel),
        preview_toggle: s(r.preview_toggle),
        code_toggle: s(r.code_toggle),
        download_button: s(r.download_button),
        send_or_stop_code_mode: s(r.send_or_stop_code_mode),
        input_focus_point_code_mode: s(r.input_focus_point_code_mode),
        attach_scan_fresh: s(r.attach_scan_fresh),
        attach_scan_inchat: s(r.attach_scan_inchat),
    }
}

/// Convenience: rectangle centre.
pub fn center(r: Region) -> (i32, i32) {
    ((r.x1 + r.x2) / 2, (r.y1 + r.y2) / 2)
}
