//! Lazy-loaded template caches for Arena.ai.
//!
//! Multi-variant templates live under `assets/macos/arena.ai/templates/`. Naming
//! follows `{element}_{theme}_{zoom}.png`. Each cache holds a vector of
//! greyscale matrices — match against any variant succeeds.
//!
//! `load_or_skip` is fault-tolerant: missing files are skipped silently,
//! so a fresh checkout (no captures yet) still compiles and runs. The
//! adapter treats an empty cache as "never matches" rather than a fatal
//! error, which lets the FSM's `Unknown → recovery` path do something
//! useful even on a half-calibrated install.

use std::sync::OnceLock;

use crate::image_matrix::io::load_gray_from_image_cached;
use crate::image_matrix::types::GrayMatrix;

pub const ASSET_ROOT: &str = "assets/macos/arena.ai/templates";

// ── Asset paths ─────────────────────────────────────────────────────────────
//
// Theme suffix: `dark` (current Arena default) and `light` (when added).
// Zoom suffix: `100`, `125`. Drop new variants in here as they are
// captured.

// Send button — bottom-right of input box.
pub const SEND_BUTTON_DARK_100: &str = "assets/macos/arena.ai/templates/send_button_dark_100.png";
pub const SEND_BUTTON_DARK_125: &str = "assets/macos/arena.ai/templates/send_button_dark_125.png";
pub const SEND_BUTTON_LIGHT_100: &str = "assets/macos/arena.ai/templates/send_button_light_100.png";

// Stop button — same screen position, replaces send during inference.
pub const STOP_BUTTON_DARK_100: &str = "assets/macos/arena.ai/templates/stop_button_dark_100.png";

// Rerun / retry button — circular arrows in the prompt action-button slot.
pub const RERUN_BUTTON_DARK_100: &str = "assets/macos/arena.ai/templates/rerun_button_dark_100.png";

// Per-message copy button.
pub const COPY_BUTTON_DARK_100: &str = "assets/macos/arena.ai/templates/copy_button_dark_100.png";

// Arena A/B response chooser Skip button.
pub const CHOICE_SKIP_BUTTON_DARK_100: &str =
    "assets/macos/arena.ai/templates/choice_skip_button_dark_100.png";
pub const CHOICE_SKIP_LABEL_DARK_100: &str =
    "assets/macos/arena.ai/templates/choice_skip_label_dark_100.png";

// Error banner / "Try again".
pub const ERROR_BANNER_DARK: &str = "assets/macos/arena.ai/templates/error_banner_dark.png";

// Conversation-too-long modal.
pub const CHAT_FULL_DARK: &str = "assets/macos/arena.ai/templates/chat_full_dark.png";

// Mode-dropdown items (used by `session_setup`).
pub const MODE_DIRECT_DARK: &str = "assets/macos/arena.ai/templates/mode_direct_dark.png";
pub const MODE_BATTLE_DARK: &str = "assets/macos/arena.ai/templates/mode_battle_dark.png";

// "New Chat" button (sidebar collapsed and expanded variants).
pub const NEW_CHAT_DARK_COLLAPSED: &str =
    "assets/macos/arena.ai/templates/new_chat_dark_collapsed.png";
pub const NEW_CHAT_DARK_EXPANDED: &str =
    "assets/macos/arena.ai/templates/new_chat_dark_expanded.png";

// ── Code-mode right panel ──────────────────────────────────────────────────
//
// These appear when Arena is generating a code project. The eye/`</>`
// pair toggles between preview and source views; the download button
// saves the generated zip.
pub const PREVIEW_TOGGLE_DARK: &str = "assets/macos/arena.ai/templates/preview_toggle_dark.png";
pub const CODE_TOGGLE_DARK: &str = "assets/macos/arena.ai/templates/code_toggle_dark.png";
pub const DOWNLOAD_BUTTON_DARK: &str = "assets/macos/arena.ai/templates/download_button_dark.png";
pub const ATTACH_BUTTON_DARK: &str = "assets/macos/arena.ai/templates/attach_button_dark.png";
// Chip close (×) button — capture once a real attachment chip is visible on Arena.
pub const ATTACH_CLOSE_BUTTON_DARK: &str =
    "assets/macos/arena.ai/templates/attach_close_button_dark.png";

// ── Caches ──────────────────────────────────────────────────────────────────

static SEND_BUTTON: OnceLock<Vec<GrayMatrix>> = OnceLock::new();
static STOP_BUTTON: OnceLock<Vec<GrayMatrix>> = OnceLock::new();
static RERUN_BUTTON: OnceLock<Vec<GrayMatrix>> = OnceLock::new();
static COPY_BUTTON: OnceLock<Vec<GrayMatrix>> = OnceLock::new();
static CHOICE_SKIP_BUTTON: OnceLock<Vec<GrayMatrix>> = OnceLock::new();
static ERROR_BANNER: OnceLock<Vec<GrayMatrix>> = OnceLock::new();
static CHAT_FULL: OnceLock<Vec<GrayMatrix>> = OnceLock::new();
static MODE_DIRECT: OnceLock<Vec<GrayMatrix>> = OnceLock::new();
static NEW_CHAT: OnceLock<Vec<GrayMatrix>> = OnceLock::new();
static NEW_CHAT_COLLAPSED: OnceLock<Vec<GrayMatrix>> = OnceLock::new();
static NEW_CHAT_EXPANDED: OnceLock<Vec<GrayMatrix>> = OnceLock::new();
static PREVIEW_TOGGLE: OnceLock<Vec<GrayMatrix>> = OnceLock::new();
static CODE_TOGGLE: OnceLock<Vec<GrayMatrix>> = OnceLock::new();
static DOWNLOAD_BUTTON: OnceLock<Vec<GrayMatrix>> = OnceLock::new();
static ATTACH_BUTTON: OnceLock<Vec<GrayMatrix>> = OnceLock::new();
static ATTACH_CLOSE_BUTTON: OnceLock<Vec<GrayMatrix>> = OnceLock::new();

fn load_or_skip(paths: &[&'static str]) -> Vec<GrayMatrix> {
    paths
        .iter()
        .filter_map(|p| load_gray_from_image_cached(p).ok())
        .collect()
}

pub fn send_button() -> &'static [GrayMatrix] {
    SEND_BUTTON.get_or_init(|| {
        load_or_skip(&[
            SEND_BUTTON_DARK_100,
            SEND_BUTTON_DARK_125,
            SEND_BUTTON_LIGHT_100,
        ])
    })
}

pub fn stop_button() -> &'static [GrayMatrix] {
    STOP_BUTTON.get_or_init(|| load_or_skip(&[STOP_BUTTON_DARK_100]))
}

pub fn rerun_button() -> &'static [GrayMatrix] {
    RERUN_BUTTON.get_or_init(|| load_or_skip(&[RERUN_BUTTON_DARK_100]))
}

pub fn copy_button() -> &'static [GrayMatrix] {
    COPY_BUTTON.get_or_init(|| load_or_skip(&[COPY_BUTTON_DARK_100]))
}

pub fn choice_skip_button() -> &'static [GrayMatrix] {
    CHOICE_SKIP_BUTTON.get_or_init(|| load_or_skip(&[CHOICE_SKIP_BUTTON_DARK_100]))
}

pub fn error_banner() -> &'static [GrayMatrix] {
    ERROR_BANNER.get_or_init(|| load_or_skip(&[ERROR_BANNER_DARK]))
}

pub fn chat_full_modal() -> &'static [GrayMatrix] {
    CHAT_FULL.get_or_init(|| load_or_skip(&[CHAT_FULL_DARK]))
}

pub fn mode_direct() -> &'static [GrayMatrix] {
    MODE_DIRECT.get_or_init(|| load_or_skip(&[MODE_DIRECT_DARK]))
}

pub fn new_chat_button() -> &'static [GrayMatrix] {
    NEW_CHAT.get_or_init(|| load_or_skip(&[NEW_CHAT_DARK_COLLAPSED, NEW_CHAT_DARK_EXPANDED]))
}

/// Only the collapsed-sidebar variant — used for unambiguous detection
/// of whether the sidebar is currently collapsed.
pub fn new_chat_collapsed_only() -> &'static [GrayMatrix] {
    NEW_CHAT_COLLAPSED.get_or_init(|| load_or_skip(&[NEW_CHAT_DARK_COLLAPSED]))
}

/// Only the expanded-sidebar variant — symmetric to the above.
pub fn new_chat_expanded_only() -> &'static [GrayMatrix] {
    NEW_CHAT_EXPANDED.get_or_init(|| load_or_skip(&[NEW_CHAT_DARK_EXPANDED]))
}

pub fn preview_toggle() -> &'static [GrayMatrix] {
    PREVIEW_TOGGLE.get_or_init(|| load_or_skip(&[PREVIEW_TOGGLE_DARK]))
}

pub fn code_toggle() -> &'static [GrayMatrix] {
    CODE_TOGGLE.get_or_init(|| load_or_skip(&[CODE_TOGGLE_DARK]))
}

pub fn download_button() -> &'static [GrayMatrix] {
    DOWNLOAD_BUTTON.get_or_init(|| load_or_skip(&[DOWNLOAD_BUTTON_DARK]))
}

pub fn attach_button() -> &'static [GrayMatrix] {
    ATTACH_BUTTON.get_or_init(|| load_or_skip(&[ATTACH_BUTTON_DARK]))
}

/// The × close button on an attached-file chip. Empty until the template is
/// captured — `load_or_skip` returns [] gracefully, triggering the pixel heuristic.
pub fn attach_close_button() -> &'static [GrayMatrix] {
    ATTACH_CLOSE_BUTTON.get_or_init(|| load_or_skip(&[ATTACH_CLOSE_BUTTON_DARK]))
}

/// Force every template's `OnceLock` to initialise now. Subsequent
/// matching calls become pure in-memory reads — no PNG decode, no
/// `.rgbm` open, no `rgb_to_gray` pass. Called from `ArenaAdapter::new`
/// so the cost is paid during adapter construction, not mid-FSM poll.
pub fn preload_all() {
    let _ = send_button();
    let _ = stop_button();
    let _ = rerun_button();
    let _ = copy_button();
    let _ = choice_skip_button();
    let _ = error_banner();
    let _ = chat_full_modal();
    let _ = mode_direct();
    let _ = new_chat_button();
    let _ = new_chat_collapsed_only();
    let _ = new_chat_expanded_only();
    let _ = preview_toggle();
    let _ = code_toggle();
    let _ = download_button();
    let _ = attach_button();
    let _ = attach_close_button();
}
