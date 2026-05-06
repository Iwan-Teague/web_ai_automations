//! Lazy-loaded DeepSeek templates.

use std::sync::OnceLock;

use crate::image_matrix::io::load_gray_from_image_cached;
use crate::image_matrix::types::GrayMatrix;

pub const ASSET_ROOT: &str = "assets/macos/deepseek/templates";

pub const SEND_ARROW_DARK: &str = "assets/macos/deepseek/templates/send_arrow_dark.png";
pub const STOP_SQUARE_DARK: &str = "assets/macos/deepseek/templates/stop_square_dark.png";
pub const NEW_CHAT_DARK: &str = "assets/macos/deepseek/templates/new_chat_dark.png";
pub const RESPONSE_ACTIONS_DARK: &str = "assets/macos/deepseek/templates/response_actions_dark.png";
pub const COPY_BUTTON_DARK: &str = "assets/macos/deepseek/templates/copy_button_dark.png";
pub const MODEL_INSTANT_SELECTED_DARK: &str =
    "assets/macos/deepseek/templates/model_instant_selected_dark.png";
pub const MODEL_EXPERT_SELECTED_DARK: &str =
    "assets/macos/deepseek/templates/model_expert_selected_dark.png";
pub const DEEP_THINKING_ON_DARK: &str = "assets/macos/deepseek/templates/deep_thinking_on_dark.png";
pub const DEEP_THINKING_OFF_DARK: &str =
    "assets/macos/deepseek/templates/deep_thinking_off_dark.png";
pub const SMART_SEARCH_ON_DARK: &str = "assets/macos/deepseek/templates/smart_search_on_dark.png";
pub const SMART_SEARCH_OFF_DARK: &str = "assets/macos/deepseek/templates/smart_search_off_dark.png";
pub const ATTACH_BUTTON_DARK_INCHAT: &str =
    "assets/macos/deepseek/templates/attach_button_dark_inchat.png";
pub const ATTACH_BUTTON_DARK_FRESH: &str =
    "assets/macos/deepseek/templates/attach_button_dark_fresh.png";
// File-chip document icon — the blue icon on the left of the attached-file card.
// Used to locate the chip anywhere in the input box (Y varies with prompt length).
pub const ATTACH_CHIP_ICON_DARK: &str = "assets/macos/deepseek/templates/attach_chip_icon_dark.png";
// × button that closes the doc-preview side panel — always at top-right of
// the viewport when an attached document has been clicked open.
pub const DOC_PANEL_CLOSE_DARK: &str = "assets/macos/deepseek/templates/doc_panel_close_dark.png";
// Square panel-toggle icon (left button of the compact pill at top-left)
// that appears when the chat-history sidebar is collapsed. Clicking it
// expands the sidebar.
pub const PANEL_TOGGLE_DARK: &str = "assets/macos/deepseek/templates/panel_toggle_dark.png";
// White × inside a dark/translucent circle — the close button that appears
// at the top-right of an uploaded chip on hover. Distinct from the
// existing `attach_chip_delete_dark.png` which captured an empty area
// (chip × is `display:none` until :hover and the original capture missed
// the hover state).
pub const CHIP_CLOSE_X_DARK: &str = "assets/macos/deepseek/templates/chip_close_x_dark.png";
// Circular reload-arrow icon — replaces the blue-doc icon on a chip
// when DeepSeek's backend is overloaded ("Server busy" subtitle, red
// chip border). Detected during upload polling so we can reload the
// page and restart the upload batch.
pub const SERVER_BUSY_DARK: &str = "assets/macos/deepseek/templates/server_busy_dark.png";
// Wide blue "New chat" button that REPLACES the send-arrow when
// DeepSeek detects a file attached to an existing chat. Clicking it
// starts a fresh chat with only this file carried over — all prior
// uploads are lost. Detection lets us recognise the state and trigger
// a full re-upload of the project map into the new chat.
pub const SUBMIT_NEW_CHAT_DARK: &str = "assets/macos/deepseek/templates/submit_new_chat_dark.png";

static SEND_ARROW: OnceLock<Vec<GrayMatrix>> = OnceLock::new();
static STOP_SQUARE: OnceLock<Vec<GrayMatrix>> = OnceLock::new();
static NEW_CHAT: OnceLock<Vec<GrayMatrix>> = OnceLock::new();
static RESPONSE_ACTIONS: OnceLock<Vec<GrayMatrix>> = OnceLock::new();
static COPY_BUTTON: OnceLock<Vec<GrayMatrix>> = OnceLock::new();
static MODEL_INSTANT_SELECTED: OnceLock<Vec<GrayMatrix>> = OnceLock::new();
static MODEL_EXPERT_SELECTED: OnceLock<Vec<GrayMatrix>> = OnceLock::new();
static DEEP_THINKING_ON: OnceLock<Vec<GrayMatrix>> = OnceLock::new();
static DEEP_THINKING_OFF: OnceLock<Vec<GrayMatrix>> = OnceLock::new();
static SMART_SEARCH_ON: OnceLock<Vec<GrayMatrix>> = OnceLock::new();
static SMART_SEARCH_OFF: OnceLock<Vec<GrayMatrix>> = OnceLock::new();
static ATTACH_BUTTON: OnceLock<Vec<GrayMatrix>> = OnceLock::new();
static ATTACH_CHIP_ICON: OnceLock<Vec<GrayMatrix>> = OnceLock::new();
static DOC_PANEL_CLOSE: OnceLock<Vec<GrayMatrix>> = OnceLock::new();
static PANEL_TOGGLE: OnceLock<Vec<GrayMatrix>> = OnceLock::new();
static CHIP_CLOSE_X: OnceLock<Vec<GrayMatrix>> = OnceLock::new();
static SERVER_BUSY: OnceLock<Vec<GrayMatrix>> = OnceLock::new();
static SUBMIT_NEW_CHAT: OnceLock<Vec<GrayMatrix>> = OnceLock::new();

fn load_or_skip(paths: &[&'static str]) -> Vec<GrayMatrix> {
    paths
        .iter()
        .filter_map(|p| load_gray_from_image_cached(p).ok())
        .collect()
}

pub fn send_arrow() -> &'static [GrayMatrix] {
    SEND_ARROW.get_or_init(|| load_or_skip(&[SEND_ARROW_DARK]))
}

pub fn stop_square() -> &'static [GrayMatrix] {
    STOP_SQUARE.get_or_init(|| load_or_skip(&[STOP_SQUARE_DARK]))
}

pub fn new_chat() -> &'static [GrayMatrix] {
    NEW_CHAT.get_or_init(|| load_or_skip(&[NEW_CHAT_DARK]))
}

pub fn response_actions() -> &'static [GrayMatrix] {
    RESPONSE_ACTIONS.get_or_init(|| load_or_skip(&[RESPONSE_ACTIONS_DARK]))
}

pub fn copy_button() -> &'static [GrayMatrix] {
    COPY_BUTTON.get_or_init(|| load_or_skip(&[COPY_BUTTON_DARK]))
}

pub fn model_instant_selected() -> &'static [GrayMatrix] {
    MODEL_INSTANT_SELECTED.get_or_init(|| load_or_skip(&[MODEL_INSTANT_SELECTED_DARK]))
}

pub fn model_expert_selected() -> &'static [GrayMatrix] {
    MODEL_EXPERT_SELECTED.get_or_init(|| load_or_skip(&[MODEL_EXPERT_SELECTED_DARK]))
}

pub fn deep_thinking_on() -> &'static [GrayMatrix] {
    DEEP_THINKING_ON.get_or_init(|| load_or_skip(&[DEEP_THINKING_ON_DARK]))
}

pub fn deep_thinking_off() -> &'static [GrayMatrix] {
    DEEP_THINKING_OFF.get_or_init(|| load_or_skip(&[DEEP_THINKING_OFF_DARK]))
}

pub fn smart_search_on() -> &'static [GrayMatrix] {
    SMART_SEARCH_ON.get_or_init(|| load_or_skip(&[SMART_SEARCH_ON_DARK]))
}

pub fn smart_search_off() -> &'static [GrayMatrix] {
    SMART_SEARCH_OFF.get_or_init(|| load_or_skip(&[SMART_SEARCH_OFF_DARK]))
}

pub fn attach_button() -> &'static [GrayMatrix] {
    ATTACH_BUTTON
        .get_or_init(|| load_or_skip(&[ATTACH_BUTTON_DARK_INCHAT, ATTACH_BUTTON_DARK_FRESH]))
}

/// Blue document icon on the left of the attached-file chip card.
/// Used to locate the chip anywhere in the input box via full-height scan.
pub fn attach_chip_icon() -> &'static [GrayMatrix] {
    ATTACH_CHIP_ICON.get_or_init(|| load_or_skip(&[ATTACH_CHIP_ICON_DARK]))
}

/// × close button for the document-preview side panel that opens when a chip
/// is clicked. Detection-only — when found, the panel is dismissed.
pub fn doc_panel_close() -> &'static [GrayMatrix] {
    DOC_PANEL_CLOSE.get_or_init(|| load_or_skip(&[DOC_PANEL_CLOSE_DARK]))
}

/// Square panel-toggle button — left icon of the compact pill at top-left
/// of the viewport when the chat-history sidebar is collapsed.
pub fn panel_toggle() -> &'static [GrayMatrix] {
    PANEL_TOGGLE.get_or_init(|| load_or_skip(&[PANEL_TOGGLE_DARK]))
}

/// White × on dark circle — the chip close button revealed by mouse hover.
pub fn chip_close_x() -> &'static [GrayMatrix] {
    CHIP_CLOSE_X.get_or_init(|| load_or_skip(&[CHIP_CLOSE_X_DARK]))
}

/// Circular reload arrow shown when DeepSeek's backend is overloaded.
pub fn server_busy() -> &'static [GrayMatrix] {
    SERVER_BUSY.get_or_init(|| load_or_skip(&[SERVER_BUSY_DARK]))
}

/// Wide blue "New chat" button shown in place of the send-arrow when
/// DeepSeek wants to spin up a fresh chat for the just-attached file.
pub fn submit_new_chat() -> &'static [GrayMatrix] {
    SUBMIT_NEW_CHAT.get_or_init(|| load_or_skip(&[SUBMIT_NEW_CHAT_DARK]))
}

/// Force every template's `OnceLock` to initialise now. Subsequent
/// matching calls become pure in-memory reads — no PNG decode, no
/// `.rgbm` open, no `rgb_to_gray` pass. Called by `DeepSeekAdapter::new`
/// so the cost is paid up-front during adapter construction rather than
/// in the middle of an FSM poll.
pub fn preload_all() {
    let _ = send_arrow();
    let _ = stop_square();
    let _ = new_chat();
    let _ = response_actions();
    let _ = copy_button();
    let _ = model_instant_selected();
    let _ = model_expert_selected();
    let _ = deep_thinking_on();
    let _ = deep_thinking_off();
    let _ = smart_search_on();
    let _ = smart_search_off();
    let _ = attach_button();
    let _ = attach_chip_icon();
    let _ = doc_panel_close();
    let _ = panel_toggle();
    let _ = chip_close_x();
    let _ = server_busy();
    let _ = submit_new_chat();
}
