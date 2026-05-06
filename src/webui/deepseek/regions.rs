//! Bounded screen regions for DeepSeek, calibrated from 1512x982 logical
//! macOS fullscreen screenshots.

use crate::webui::adapter::Region;

#[derive(Clone, Copy, Debug)]
pub struct Regions {
    pub screen_width: i32,
    pub screen_height: i32,
    pub new_chat_button: Region,
    pub model_segment: Region,
    pub model_scan: Region,
    pub instant_button: Region,
    pub expert_button: Region,
    pub input_box_fresh: Region,
    pub input_focus_fresh: Region,
    pub input_box_inchat: Region,
    pub input_focus_inchat: Region,
    pub deep_thinking: Region,
    pub deep_thinking_scan: Region,
    pub smart_search: Region,
    pub smart_search_scan: Region,
    pub send_or_stop_fresh: Region,
    pub send_or_stop_inchat: Region,
    pub send_scan_fresh: Region,
    pub send_scan_inchat: Region,
    pub conversation: Region,
    pub response_actions: Region,
    pub copy_scan: Region,
    /// Scan region for the file-upload (paperclip) button on the fresh-chat page.
    /// Covers the area just left of the send button where the icon lives.
    pub attach_scan_fresh: Region,
    /// Scan region for the paperclip in an active chat.
    pub attach_scan_inchat: Region,
    /// Top-right area where the doc-preview side-panel × close button lives.
    /// When DeepSeek's "view document" panel is open it shifts the entire
    /// composer layout — every poll dismisses it before classifying state.
    pub doc_panel_close_scan: Region,
    /// Top-left compact pill (panel-toggle + new-chat icons) that appears
    /// when the chat-history sidebar is collapsed. Used to detect the
    /// "sidebar closed" state and click the panel-toggle button to expand.
    pub panel_toggle_scan: Region,
    /// Strip above the fresh-chat input where uploaded file chips render.
    /// Used to find the *rightmost* chip — the most recently uploaded one
    /// — so we can verify or delete just-uploaded files without touching
    /// earlier successful chips.
    pub chip_strip_fresh: Region,
    /// Same strip, but in the in-chat composer layout (input box at the
    /// bottom of the page).
    pub chip_strip_inchat: Region,
}

impl Regions {
    pub fn for_primary_screen() -> Self {
        match crate::image_matrix::capture::primary_screen_size() {
            Ok((w, h)) => Self::scaled(w, h),
            Err(_) => Self::mac_1512x982(),
        }
    }

    pub fn mac_1512x982() -> Self {
        Self::scaled(1512, 982)
    }

    fn scaled(screen_width: i32, screen_height: i32) -> Self {
        let sx = screen_width as f32 / 1512.0;
        let sy = screen_height as f32 / 982.0;
        let r = |x1, y1, x2, y2| scale_region(x1, y1, x2, y2, sx, sy);
        Self {
            screen_width,
            screen_height,
            new_chat_button: r(5, 185, 255, 250),
            model_segment: r(745, 440, 1035, 505),
            model_scan: r(735, 220, 1050, 540),
            instant_button: r(750, 445, 890, 500),
            expert_button: r(890, 445, 1030, 500),
            input_box_fresh: r(495, 385, 1280, 790),
            input_focus_fresh: r(515, 405, 1220, 760),
            input_box_inchat: r(495, 830, 1280, 955),
            input_focus_inchat: r(515, 850, 1220, 885),
            deep_thinking: r(505, 590, 655, 655),
            // Tightened to the actual toggle row. Previous y=500..960 scan
            // spanned 460px and produced false-positive matches above the
            // button — clicks landed at y≈575 (above the button) and the
            // toggle never flipped.
            deep_thinking_scan: r(500, 580, 660, 670),
            smart_search: r(645, 590, 805, 655),
            smart_search_scan: r(640, 580, 815, 670),
            send_or_stop_fresh: r(1220, 585, 1275, 800),
            send_or_stop_inchat: r(1220, 900, 1275, 950),
            send_scan_fresh: r(1225, 500, 1275, 820),
            send_scan_inchat: r(1225, 875, 1275, 965),
            conversation: r(500, 135, 1280, 820),
            response_actions: r(490, 700, 760, 800),
            copy_scan: r(485, 450, 590, 850),
            // Fresh-chat: input box centered on screen; attach button at bottom of centered
            // box, near y=590-650 for an empty input (same row as send_or_stop_fresh).
            // Wide x to account for layout variance; y starts at 530 to cover the full
            // send_or_stop_fresh range (585-800) plus headroom for empty-input position.
            // Paperclip lives on the right rail of the composer's bottom
            // row. Empirically observed at (1195, 615) on one layout and
            // (1210, 758) on another — DeepSeek's empty-state composer
            // moves the row, but it's never above y≈600. Tightened y
            // upper bound from 530 → 600 because the previous wider band
            // matched a phantom UI element at (1210, 564) on every scan.
            // X stays at x≥1180 to keep chip icons (x≈1120) out of range.
            attach_scan_fresh: r(1180, 600, 1230, 820),
            // In-chat attachment row shifts left when DeepSeek shows the
            // wide "New chat" submit pill. Observed paperclip centers:
            // normal (1200,925), New Chat layout (≈1143,923). Keep x2
            // near the old rail but widen left enough to include both.
            attach_scan_inchat: r(1100, 875, 1230, 965),
            // Doc-preview side panel close × — always near the top-right of
            // the viewport when the panel is open. Generous box for layout
            // variance; template threshold rejects empty regions.
            doc_panel_close_scan: r(1420, 115, 1510, 200),
            // Compact pill at the very top-left of the viewport when the
            // chat-history sidebar is collapsed. Below Chrome's address bar
            // (which lives ~y=50-130) and above the expanded sidebar
            // header at y=185. Generous box for layout variance.
            panel_toggle_scan: r(15, 130, 250, 220),
            // Chip cards sit BELOW the "Start chatting with …" header.
            // Debug capture of the prior y=385-470 region showed only the
            // header text and the top edges of the chips — the icons live
            // ~80 px lower. Empirically the chip body spans roughly
            // y=470-560 in fresh-chat layout.
            chip_strip_fresh: r(495, 470, 1280, 565),
            // In-chat composer sits at y≈810–955. When a chip attaches
            // mid-conversation, DeepSeek may either expand the composer
            // upward (chip lands at y≈735–795) OR render the chip inline
            // at the top of the unexpanded composer (chip lands at
            // y≈810–880). Widen the scan to cover both behaviours so the
            // post-prompt deferred-upload retry can detect a chip
            // regardless of which way the layout grows.
            chip_strip_inchat: r(495, 735, 1280, 900),
        }
    }
}

pub fn center(region: Region) -> (i32, i32) {
    ((region.x1 + region.x2) / 2, (region.y1 + region.y2) / 2)
}

fn scale_region(x1: i32, y1: i32, x2: i32, y2: i32, sx: f32, sy: f32) -> Region {
    Region {
        x1: (x1 as f32 * sx).round() as i32,
        y1: (y1 as f32 * sy).round() as i32,
        x2: (x2 as f32 * sx).round() as i32,
        y2: (y2 as f32 * sy).round() as i32,
    }
}
