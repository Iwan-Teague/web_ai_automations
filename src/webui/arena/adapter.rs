//! `WebUiAdapter` impl for Arena.ai.
//!
//! Detection layering — least → most expensive:
//!
//! 1. **URL** — `arena.ai/c/<uuid>` means we're inside a chat;
//!    `arena.ai/text/...` or `arena.ai/code/...` or bare `arena.ai`
//!    means home/empty. URL is by far the most stable signal across
//!    theme / font / DPI changes, so we look there first.
//! 2. **Visual templates with bounded regions** — used to disambiguate
//!    "in chat, generating" from "in chat, complete" by checking the
//!    send/stop button at the same screen position.
//! 3. **Fallback** — when neither yields a confident answer the adapter
//!    returns `Unknown` and the FSM hands off to the recovery dispatcher.
//!
//! Click coordinates are computed at the call site from the matched
//! `Region::center()` — no hard-coded pixels.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

use crate::image_matrix::compare::{find_all_matches_gray, match_any_gray, match_which_gray};
use crate::image_matrix::types::{GrayMatrix, Tolerance};
use crate::webui::adapter::{AdapterError, Region, WebUiAdapter, WebUiState};
use crate::webui::arena::regions::{ArenaSurface, Regions, center};
use crate::webui::arena::templates;
use crate::webui::browser_state;

// ── Logging helpers ─────────────────────────────────────────────────────────

fn ts() -> String {
    chrono::Local::now().format("%H:%M:%S%.3f").to_string()
}

fn log_click(ctx: &str, label: &str, x: i32, y: i32) {
    println!("[{}] [{}] {} — coords ({}, {})", ts(), ctx, label, x, y);
}

fn log_info(ctx: &str, msg: &str) {
    println!("[{}] [{}] {}", ts(), ctx, msg);
}

#[allow(dead_code)]
fn log_warn(ctx: &str, msg: &str) {
    eprintln!("[{}] [{}] WARN: {}", ts(), ctx, msg);
}

pub struct ArenaAdapter {
    regions: Regions,
    last_model: Mutex<Option<String>>,
    /// Variance snapshot of the conversation area from the previous
    /// `detect_state` call, used for change-detection fallback when
    /// template matching fails (e.g. code-mode layout).
    last_conv_var: Mutex<Option<f64>>,
    /// Consecutive polls where variance was stable (Δ < 20, var > 200).
    /// Requires ≥ 3 consecutive before declaring Complete — prevents
    /// false positives from pages that haven't started rendering yet.
    stable_polls: Mutex<u32>,
    /// Whether variance ever changed (Δ > 50) since the last
    /// `submit_prompt` call. Prevents false Complete when the send
    /// failed and the page never changed at all.
    saw_variance_change: Mutex<bool>,
}

impl ArenaAdapter {
    /// Build using the screen-size-appropriate region preset detected at
    /// startup. Falls back to the Mac calibration if the screen size
    /// can't be read.
    pub fn new() -> Self {
        Self::new_for_surface(ArenaSurface::Code)
    }

    pub fn new_for_surface(surface: ArenaSurface) -> Self {
        let regions = match crate::image_matrix::capture::primary_screen_size() {
            Ok((w, h)) => {
                let r = Regions::for_screen_and_surface(w, h, surface);
                eprintln!("[adapter] using {:?} regions for {w}×{h}", r.surface);
                r
            }
            Err(_) => match surface {
                ArenaSurface::Code => Regions::mac_1512x982_code(),
                ArenaSurface::Text => Regions::mac_1512x982_text_scaffold(),
            },
        };
        Self::with_regions(regions)
    }

    pub fn with_regions(regions: Regions) -> Self {
        // Load every template's binary matrix sidecar now so no PNG
        // decode happens mid-poll. After this returns, every accessor
        // is a pure memory read.
        super::templates::preload_all();
        Self {
            regions,
            last_model: Mutex::new(None),
            last_conv_var: Mutex::new(None),
            stable_polls: Mutex::new(0),
            saw_variance_change: Mutex::new(false),
        }
    }

    pub fn regions(&self) -> &Regions {
        &self.regions
    }

    pub fn surface(&self) -> ArenaSurface {
        self.regions.surface
    }
}

impl Default for ArenaAdapter {
    fn default() -> Self {
        Self::new()
    }
}

// ── Detection helpers ───────────────────────────────────────────────────────

/// Pixel-value variance of a captured screen region — theme-agnostic
/// "how much content is here?" feature. Empty/uniform regions ≈ 0;
/// regions with text/icons score in the hundreds to thousands.
fn region_variance(region: Region) -> f64 {
    let cap = match crate::image_matrix::capture::capture_gray_matrix(
        region.x1, region.y1, region.x2, region.y2,
    ) {
        Ok(m) => m,
        Err(_) => return 0.0,
    };
    let n = cap.data.len();
    if n == 0 {
        return 0.0;
    }
    let mean: f64 = cap.data.iter().map(|&v| v as f64).sum::<f64>() / n as f64;
    cap.data
        .iter()
        .map(|&v| (v as f64 - mean).powi(2))
        .sum::<f64>()
        / n as f64
}

/// Mean brightness for a bounded screen region.
fn region_mean(region: Region) -> f64 {
    let cap = match crate::image_matrix::capture::capture_gray_matrix(
        region.x1, region.y1, region.x2, region.y2,
    ) {
        Ok(m) => m,
        Err(_) => return 0.0,
    };
    let n = cap.data.len();
    if n == 0 {
        return 0.0;
    }
    cap.data.iter().map(|&v| v as f64).sum::<f64>() / n as f64
}

fn region_bright_ratio(region: Region, min_luma: u8) -> f64 {
    let cap = match crate::image_matrix::capture::capture_gray_matrix(
        region.x1, region.y1, region.x2, region.y2,
    ) {
        Ok(m) => m,
        Err(_) => return 0.0,
    };
    let n = cap.data.len();
    if n == 0 {
        return 0.0;
    }
    let bright = cap.data.iter().filter(|&&v| v >= min_luma).count();
    bright as f64 / n as f64
}

/// Capture `region` and check whether any template matches.
///
/// An empty `templates` slice (no PNGs captured yet) returns `Ok(false)`
/// rather than an error — keeps the adapter usable on a fresh checkout.
fn region_matches(
    region: Region,
    templates: &[GrayMatrix],
    tolerance: Tolerance,
    threshold: f32,
) -> Result<bool, AdapterError> {
    if templates.is_empty() {
        return Ok(false);
    }

    let m = crate::image_matrix::capture::capture_gray_matrix(
        region.x1, region.y1, region.x2, region.y2,
    )
    .map_err(|e| AdapterError::Capture(e.to_string()))?;

    Ok(match_any_gray(&m, templates, tolerance, threshold))
}

fn template_center_in_region(
    region: Region,
    templates: &[GrayMatrix],
    tolerance: Tolerance,
    threshold: f32,
) -> Result<Option<(i32, i32)>, AdapterError> {
    if templates.is_empty() {
        return Ok(None);
    }

    let cap = crate::image_matrix::capture::capture_gray_matrix(
        region.x1, region.y1, region.x2, region.y2,
    )
    .map_err(|e| AdapterError::Capture(e.to_string()))?;

    Ok(
        match_which_gray(&cap, templates, tolerance, threshold).map(|(idx, m)| {
            let needle = &templates[idx];
            capture_to_mouse_point((
                region.x1 + m.col as i32 + needle.width as i32 / 2,
                region.y1 + m.row as i32 + needle.height as i32 / 2,
            ))
        }),
    )
}

#[derive(Debug, Clone)]
struct ButtonCandidate {
    center: (i32, i32),
    bounds: Region,
    mean: f64,
    area: usize,
}

/// Matrix-scan a toolbar for the bright rectangular Download button.
///
/// This intentionally does not depend on the exact stale PNG template:
/// Arena changes button text/icon antialiasing often, but the active
/// Download button is still a bright rounded rectangle in the top-right
/// toolbar. We confirm by connected-component shape + brightness, then
/// click the component center and verify via the Downloads folder.
fn find_bright_toolbar_button(
    region: Region,
    screen_width: i32,
) -> Result<Option<ButtonCandidate>, AdapterError> {
    let cap = crate::image_matrix::capture::capture_gray_matrix(
        region.x1, region.y1, region.x2, region.y2,
    )
    .map_err(|e| AdapterError::Capture(e.to_string()))?;

    let width = cap.width;
    let height = cap.height;
    if width == 0 || height == 0 {
        return Ok(None);
    }

    let mut visited = vec![false; cap.data.len()];
    let mut best: Option<ButtonCandidate> = None;

    for y in 0..height {
        for x in 0..width {
            let idx = y * width + x;
            if visited[idx] || cap.data[idx] < 150 {
                continue;
            }

            let mut stack = vec![(x, y)];
            visited[idx] = true;
            let mut min_x = x;
            let mut max_x = x;
            let mut min_y = y;
            let mut max_y = y;
            let mut area = 0usize;
            let mut sum = 0usize;

            while let Some((cx, cy)) = stack.pop() {
                let cidx = cy * width + cx;
                area += 1;
                sum += cap.data[cidx] as usize;
                min_x = min_x.min(cx);
                max_x = max_x.max(cx);
                min_y = min_y.min(cy);
                max_y = max_y.max(cy);

                let x0 = cx.saturating_sub(1);
                let x1 = (cx + 1).min(width - 1);
                let y0 = cy.saturating_sub(1);
                let y1 = (cy + 1).min(height - 1);
                for ny in y0..=y1 {
                    for nx in x0..=x1 {
                        let nidx = ny * width + nx;
                        if !visited[nidx] && cap.data[nidx] >= 150 {
                            visited[nidx] = true;
                            stack.push((nx, ny));
                        }
                    }
                }
            }

            let bw = max_x - min_x + 1;
            let bh = max_y - min_y + 1;
            if !(70..=190).contains(&bw) || !(20..=55).contains(&bh) {
                continue;
            }
            if area < 900 {
                continue;
            }

            let abs_x1 = region.x1 + min_x as i32;
            let abs_y1 = region.y1 + min_y as i32;
            let abs_x2 = region.x1 + max_x as i32 + 1;
            let abs_y2 = region.y1 + max_y as i32 + 1;
            let center = ((abs_x1 + abs_x2) / 2, (abs_y1 + abs_y2) / 2);

            // Download is the rightmost large bright toolbar button. This
            // prevents matching the preview URL/address field or left icons.
            if center.0 < screen_width - 260 {
                continue;
            }

            let mean = sum as f64 / area as f64;
            let candidate = ButtonCandidate {
                center: capture_to_mouse_point(center),
                bounds: Region {
                    x1: abs_x1,
                    y1: abs_y1,
                    x2: abs_x2,
                    y2: abs_y2,
                },
                mean,
                area,
            };

            let replace = match &best {
                None => true,
                Some(prev) => {
                    candidate.center.0 > prev.center.0
                        || (candidate.center.0 == prev.center.0 && candidate.mean > prev.mean)
                }
            };
            if replace {
                best = Some(candidate);
            }
        }
    }

    Ok(best)
}

fn capture_to_mouse_point((x, y): (i32, i32)) -> (i32, i32) {
    let dx = std::env::var("WEB_AI_CAPTURE_TO_MOUSE_DX")
        .ok()
        .and_then(|v| v.parse::<i32>().ok())
        .unwrap_or(0);
    let dy = std::env::var("WEB_AI_CAPTURE_TO_MOUSE_DY")
        .ok()
        .and_then(|v| v.parse::<i32>().ok())
        .unwrap_or(0);
    (x + dx, y + dy)
}

fn region_diff_pct(pre: &GrayMatrix, region: Region, tolerance: u8) -> Result<f32, AdapterError> {
    let post = crate::image_matrix::capture::capture_gray_matrix(
        region.x1, region.y1, region.x2, region.y2,
    )
    .map_err(|e| AdapterError::Capture(e.to_string()))?;
    let total = pre.data.len().max(1);
    let diff = crate::calibration::diff_pixel_count(pre, &post, tolerance);
    Ok(diff as f32 / total as f32 * 100.0)
}

fn has_dark_frame_around_component(
    cap: &GrayMatrix,
    min_x: usize,
    max_x: usize,
    min_y: usize,
    max_y: usize,
) -> bool {
    if cap.width == 0 || cap.height == 0 {
        return false;
    }

    let pad = 7usize;
    if min_x < pad || min_y < pad || max_x + pad >= cap.width || max_y + pad >= cap.height {
        return false;
    }

    let x1 = min_x - pad;
    let x2 = max_x + pad;
    let y1 = min_y - pad;
    let y2 = max_y + pad;

    let mut border_pixels = 0usize;
    let mut dark_pixels = 0usize;
    for y in y1..=y2 {
        for x in x1..=x2 {
            let in_white_component_box = x >= min_x && x <= max_x && y >= min_y && y <= max_y;
            if in_white_component_box {
                continue;
            }

            border_pixels += 1;
            if cap.get(x, y) <= 90 {
                dark_pixels += 1;
            }
        }
    }

    border_pixels >= 300 && dark_pixels as f64 / border_pixels as f64 >= 0.55
}

/// What the URL tells us, if anything.
#[derive(Debug)]
enum UrlClass {
    Home,    // arena.ai / arena.ai/text/* / arena.ai/code/* — empty
    InChat,  // arena.ai/c/<uuid>
    OffSite, // somewhere else entirely
    Unknown, // couldn't read the URL
}

fn classify_url() -> UrlClass {
    match browser_state::active_chrome_url() {
        Ok(url) => {
            let l = url.to_lowercase();
            if !(l.contains("arena.ai") || l.contains("lmarena.ai")) {
                return UrlClass::OffSite;
            }
            if l.contains("/c/") {
                return UrlClass::InChat;
            }
            UrlClass::Home
        }
        Err(_) => UrlClass::Unknown,
    }
}

// ── Adapter impl ────────────────────────────────────────────────────────────

impl WebUiAdapter for ArenaAdapter {
    fn name(&self) -> &'static str {
        "arena"
    }

    /// Raise Chrome to the foreground before each detection poll —
    /// otherwise Terminal or another window can drift over the Arena tab
    /// and region captures read the wrong pixels.
    fn prepare_for_detection(&self) -> Result<(), AdapterError> {
        let _ = crate::webui::browser_state::focus_app_by_name("Chrome");
        std::thread::sleep(std::time::Duration::from_millis(150));
        Ok(())
    }

    fn detect_state(&self) -> Result<WebUiState, AdapterError> {
        // Step 1 — URL classification (cheap, no screen capture).
        let url = classify_url();

        // Step 2 — modal/error checks first, regardless of URL, because
        // a modal can be drawn on top of any state.
        if region_matches(
            self.regions.chat_full_modal,
            templates::chat_full_modal(),
            Tolerance::NORMAL,
            0.88,
        )? {
            return Ok(WebUiState::ChatFull);
        }
        if region_matches(
            self.regions.error_banner,
            templates::error_banner(),
            Tolerance::NORMAL,
            0.88,
        )? {
            return Ok(WebUiState::Error);
        }

        // Stage-agnostic Arena watchdog: when the white stop square appears,
        // click it before any Ready/Generating/Complete classification.
        if !matches!(url, UrlClass::OffSite) && self.click_stop_square_watchdog_if_visible()? {
            return Ok(WebUiState::Unknown);
        }

        // Step 3 — URL-driven branches.
        match url {
            UrlClass::OffSite => Ok(WebUiState::Unknown),
            UrlClass::Home => {
                // Home page can be Ready (input box visible) or Unknown.
                // The input box / send button anchor confirms Ready.
                if region_matches(
                    self.regions.send_or_stop,
                    templates::send_button(),
                    Tolerance::NORMAL,
                    0.85,
                )? {
                    Ok(WebUiState::Ready)
                } else {
                    // Even without templates, the URL alone is enough to
                    // call this Ready. Be honest about confidence —
                    // return Ready; recovery is cheap if it's wrong.
                    Ok(WebUiState::Ready)
                }
            }
            UrlClass::InChat => {
                // Being InChat (URL = /c/<id>) proves submission
                // succeeded, but it does NOT prove the model has produced
                // output. Do not mark `saw_variance_change` here; doing so
                // let a quiet, unchanged conversation region promote itself
                // to Complete and caused follow-up prompts to be pasted
                // while the model was still working.
                {
                    let saw = self.saw_variance_change.lock().unwrap();
                    if !*saw {
                        log_info(
                            "detect_state",
                            "InChat URL detected; waiting for visual completion signal",
                        );
                    }
                }

                // After sending, the input/send button moves to the bottom
                // of the screen. Check calibrated positions including code-
                // mode layout (narrower left pane when right panel is open).
                let stop_regions: Vec<Region> = if self.regions.surface == ArenaSurface::Text {
                    vec![self.regions.send_or_stop_inchat]
                } else {
                    vec![
                        self.regions.send_or_stop,
                        self.regions.send_or_stop_inchat,
                        self.regions.send_or_stop_code_mode,
                    ]
                };
                let send_regions = stop_regions.clone();
                let footer_regions = [
                    self.regions.message_footer,
                    self.regions.message_footer_inchat,
                ];

                let code_panel_visible = if self.regions.surface == ArenaSurface::Code {
                    // Code completion has a stronger signal than the bottom
                    // input button: the right panel's Download button becomes
                    // bright/active. Text view has no code panel; do not scan
                    // browser toolbar as a fake download signal there.
                    if let Some(download_active) = self.download_button_active_state()? {
                        if !download_active {
                            log_info(
                                "detect_state",
                                "download button visible but inactive/dim → Generating",
                            );
                            return Ok(WebUiState::Generating);
                        }
                        log_info("detect_state", "download button active/bright → Complete");
                        return Ok(WebUiState::Complete);
                    }
                    self.has_code_panel()?
                } else {
                    false
                };

                if self.regions.surface == ArenaSurface::Text {
                    if self.text_stop_square_visible()? {
                        return Ok(WebUiState::Generating);
                    }
                } else {
                    for &r in &stop_regions {
                        if region_matches(r, templates::stop_button(), Tolerance::NORMAL, 0.85)? {
                            return Ok(WebUiState::Generating);
                        }
                    }
                    if let Some((sx, sy)) = self.find_stop_button_on_screen()? {
                        log_info(
                            "detect_state",
                            &format!("stop button visible at ({sx},{sy}) → Generating"),
                        );
                        return Ok(WebUiState::Generating);
                    }
                }
                if self.find_response_choice_skip_button()?.is_some() {
                    log_info(
                        "detect_state",
                        "response chooser visible → Complete (will Skip before copy)",
                    );
                    return Ok(WebUiState::Complete);
                }
                if self.find_rerun_button_in_prompt_slot()?.is_some() {
                    log_info(
                        "detect_state",
                        "rerun icon visible in prompt slot → Complete",
                    );
                    return Ok(WebUiState::Complete);
                }
                if code_panel_visible {
                    log_info(
                        "detect_state",
                        "code panel visible and no stop button → Complete",
                    );
                    return Ok(WebUiState::Complete);
                }
                if self.regions.surface == ArenaSurface::Text {
                    if self.find_latest_response_copy_button()?.is_some() {
                        log_info(
                            "detect_state",
                            "text response copy button visible → Complete",
                        );
                        return Ok(WebUiState::Complete);
                    }
                }
                for &r in &footer_regions {
                    if region_matches(r, templates::copy_button(), Tolerance::NORMAL, 0.85)? {
                        return Ok(WebUiState::Complete);
                    }
                }
                for &r in &send_regions {
                    if region_matches(r, templates::send_button(), Tolerance::NORMAL, 0.85)? {
                        log_info("detect_state", "send button template visible → Complete");
                        return Ok(WebUiState::Complete);
                    }
                }
                if let Some((sx, sy)) = self.find_send_button_on_screen()? {
                    log_info(
                        "detect_state",
                        &format!("send button visible at ({sx},{sy}) → Complete"),
                    );
                    return Ok(WebUiState::Complete);
                }

                // ── Variance-based fallback (code-mode layout) ──────────
                //
                // In code mode, Arena splits the screen: chat on left,
                // preview on right. The send/stop/copy buttons move to
                // positions outside the calibrated regions. Template
                // matching fails → Unknown.
                //
                // Fallback: compare conversation-area variance across
                // consecutive polls. Large delta → content is streaming
                // (Generating). Stable variance alone is NOT completion;
                // it can mean "the model is thinking without visible
                // movement". Completion must be proven above by a ready UI
                // signal (code panel/download active, copy button, send
                // button).
                let conv = self.regions.conversation;
                let cur_var = region_variance(conv);
                let mut last_var = self.last_conv_var.lock().unwrap();
                let prev = *last_var;
                *last_var = Some(cur_var);
                drop(last_var);

                let mut sp = self.stable_polls.lock().unwrap();
                let mut saw = self.saw_variance_change.lock().unwrap();

                if let Some(pv) = prev {
                    let delta = (cur_var - pv).abs();
                    if delta > 50.0 {
                        *sp = 0;
                        *saw = true;
                        log_info(
                            "detect_state",
                            &format!(
                                "variance fallback: Δ={delta:.1} (prev={pv:.1} cur={cur_var:.1}) → Generating"
                            ),
                        );
                        return Ok(WebUiState::Generating);
                    }
                    if cur_var > 200.0 && delta < 20.0 {
                        *sp += 1;
                        let count = *sp;
                        if *saw {
                            // Variance DID change at some point → generation
                            // happened. Now stable → probably settling, but
                            // no authoritative ready UI signal has matched.
                            if count >= 3 {
                                log_info(
                                    "detect_state",
                                    &format!(
                                        "variance fallback: stable ×{count} (Δ={delta:.1}, var={cur_var:.1}) but no ready UI signal → Generating"
                                    ),
                                );
                                return Ok(WebUiState::Generating);
                            }
                            log_info(
                                "detect_state",
                                &format!(
                                    "variance fallback: stable ×{count}/3 (Δ={delta:.1}, var={cur_var:.1}) → Generating"
                                ),
                            );
                            return Ok(WebUiState::Generating);
                        }
                        // Never saw a change → page is unchanged since
                        // submission. Don't falsely report Generating.
                        log_info(
                            "detect_state",
                            &format!(
                                "variance fallback: stable ×{count} but no prior change — Unknown"
                            ),
                        );
                        return Ok(WebUiState::Unknown);
                    }
                    *sp = 0;
                } else {
                    *sp = 0;
                }

                Ok(WebUiState::Unknown)
            }
            UrlClass::Unknown => {
                // No URL — fall back to pure visual matching.
                if region_matches(
                    self.regions.send_or_stop,
                    templates::stop_button(),
                    Tolerance::NORMAL,
                    0.85,
                )? {
                    return Ok(WebUiState::Generating);
                }
                if region_matches(
                    self.regions.send_or_stop,
                    templates::send_button(),
                    Tolerance::NORMAL,
                    0.85,
                )? {
                    return Ok(WebUiState::Ready);
                }
                Ok(WebUiState::Unknown)
            }
        }
    }

    fn detect_model(&self) -> Result<Option<String>, AdapterError> {
        // Try the page title first — Arena puts the model name in the
        // tab title for Direct mode (e.g. "gemini-3-flash · Arena").
        if let Ok(title) = browser_state::active_chrome_title() {
            if let Some(stripped) = title.strip_suffix(" · Arena") {
                let m = stripped.trim().to_string();
                if !m.is_empty() {
                    if let Ok(mut g) = self.last_model.lock() {
                        *g = Some(m.clone());
                    }
                    return Ok(Some(m));
                }
            }
        }
        Ok(self.last_model.lock().ok().and_then(|g| g.clone()))
    }

    fn submit_prompt(&self, text: &str) -> Result<(), AdapterError> {
        // Arena has THREE input positions:
        //   - Fresh chat (/text/direct, /code/direct): input CENTERED
        //     vertically in the page (input_focus_point).
        //   - Active chat (/c/<id>), full width: input at BOTTOM of page
        //     (input_focus_point_inchat).
        //   - Active chat with code panel: input at bottom of the NARROWER
        //     left pane (input_focus_point_code_mode, x≈260).
        let in_chat = matches!(classify_url(), UrlClass::InChat);
        let code_panel_open = in_chat
            && self.regions.surface == ArenaSurface::Code
            && self.has_code_panel().unwrap_or(false);
        let input_region = self.regions.input_box;
        let conv_region = self.regions.conversation;
        let (cx, cy) = if code_panel_open {
            center(self.regions.input_focus_point_code_mode)
        } else if in_chat {
            center(self.regions.input_focus_point_inchat)
        } else {
            center(self.regions.input_focus_point)
        };

        // 1. Save current clipboard.
        let mut clip = arboard::Clipboard::new()
            .map_err(|e| AdapterError::Clipboard(format!("arboard new: {e}")))?;
        let saved_clip = clip.get_text().ok();

        // 2. Set clipboard to prompt text.
        log_info(
            "submit_prompt",
            &format!(
                "setting clipboard: {} chars, first 60: {:?}",
                text.len(),
                text.chars().take(60).collect::<String>()
            ),
        );
        clip.set_text(text.to_string())
            .map_err(|e| AdapterError::Clipboard(format!("clipboard set: {e}")))?;
        std::thread::sleep(std::time::Duration::from_millis(80));

        let verify = clip.get_text().unwrap_or_default();
        if verify.len() != text.len() {
            log_warn(
                "submit_prompt",
                &format!(
                    "clipboard verify MISMATCH: set {} chars but read back {} chars",
                    text.len(),
                    verify.len()
                ),
            );
        } else {
            log_info(
                "submit_prompt",
                &format!("clipboard verified: {} chars", verify.len()),
            );
        }

        // 3. Paste: try calibrated position, then y-offsets, then code-mode.
        let mut pasted_at_code_mode = code_panel_open;
        log_info("submit_prompt", &format!("attempt 1: click ({cx},{cy})"));
        let mut pasted = self.try_click_and_paste(cx, cy, input_region, &mut clip, &saved_clip)?;

        if !pasted {
            save_debug_png("submit_input_miss", input_region);
            save_debug_png("submit_conv_region", conv_region);

            let offsets = [-8, 8, -16, 16];
            for &dy in &offsets {
                let retry_y = cy + dy;
                let _ = crate::human_simulations::press_key("cmd+z");
                std::thread::sleep(std::time::Duration::from_millis(80));
                clip.set_text(text.to_string())
                    .map_err(|e| AdapterError::Clipboard(format!("clipboard re-set: {e}")))?;
                std::thread::sleep(std::time::Duration::from_millis(60));

                log_info("submit_prompt", &format!("retry: click ({cx},{retry_y})"));
                pasted =
                    self.try_click_and_paste(cx, retry_y, input_region, &mut clip, &saved_clip)?;
                if pasted {
                    break;
                }
            }

            // Try code-mode input position (narrower left pane).
            if !pasted && in_chat {
                let (code_cx, code_cy) = center(self.regions.input_focus_point_code_mode);
                log_info(
                    "submit_prompt",
                    &format!("trying code-mode input ({code_cx},{code_cy})"),
                );
                let _ = crate::human_simulations::press_key("cmd+z");
                std::thread::sleep(std::time::Duration::from_millis(80));
                clip.set_text(text.to_string())
                    .map_err(|e| AdapterError::Clipboard(format!("clipboard re-set: {e}")))?;
                std::thread::sleep(std::time::Duration::from_millis(60));
                pasted = self.try_click_and_paste(
                    code_cx,
                    code_cy,
                    input_region,
                    &mut clip,
                    &saved_clip,
                )?;

                if !pasted {
                    for &dy in &[-8, 8, -16, 16] {
                        let retry_y = code_cy + dy;
                        let _ = crate::human_simulations::press_key("cmd+z");
                        std::thread::sleep(std::time::Duration::from_millis(80));
                        clip.set_text(text.to_string()).map_err(|e| {
                            AdapterError::Clipboard(format!("clipboard re-set: {e}"))
                        })?;
                        std::thread::sleep(std::time::Duration::from_millis(60));

                        log_info(
                            "submit_prompt",
                            &format!("code-mode retry: click ({code_cx},{retry_y})"),
                        );
                        pasted = self.try_click_and_paste(
                            code_cx,
                            retry_y,
                            input_region,
                            &mut clip,
                            &saved_clip,
                        )?;
                        if pasted {
                            break;
                        }
                    }
                }
                if pasted {
                    pasted_at_code_mode = true;
                }
            }

            if !pasted {
                if let Some(s) = &saved_clip {
                    let _ = clip.set_text(s.clone());
                }
                return Err(AdapterError::Input(format!(
                    "prompt paste failed at ({cx},{cy}) and code-mode ({},{}) with ±offsets. \
                     Check the run screenshots folder to see actual textarea position.",
                    center(self.regions.input_focus_point_code_mode).0,
                    center(self.regions.input_focus_point_code_mode).1,
                )));
            }
        }

        // 4. Click send button.
        //
        // Send-verification:
        //   - Fresh chat → URL changes from /text/... to /c/<uuid>
        //   - In-chat → URL stays at /c/<uuid>, so we measure
        //     INPUT AREA variance: if it drops, the text was consumed.
        //     (Using detect_state here was unreliable — stale variance
        //     counters falsely returned Generating/Complete.)
        let in_chat_send = matches!(classify_url(), UrlClass::InChat);
        let pre_send_conv = crate::image_matrix::capture::capture_gray_matrix(
            self.regions.conversation.x1,
            self.regions.conversation.y1,
            self.regions.conversation.x2,
            self.regions.conversation.y2,
        )
        .ok();
        let send_region = if in_chat_send {
            self.regions.send_or_stop_inchat
        } else {
            self.regions.send_or_stop
        };
        let sx = center(send_region).0;
        let base_y = center(send_region).1;
        let inchat_send_scan = if in_chat_send {
            self.find_send_button_on_screen()?
        } else {
            None
        };
        let fresh_send_by_template = if !in_chat_send {
            self.find_send_button_on_screen()?.or_else(|| {
                template_center_in_region(
                    self.regions.send_or_stop,
                    templates::send_button(),
                    Tolerance::NORMAL,
                    0.82,
                )
                .ok()
                .flatten()
            })
        } else {
            None
        };

        // Order candidates: code-mode first when paste was at code-mode.
        let send_candidates: Vec<(i32, i32)> = if pasted_at_code_mode {
            let mut v = Vec::new();
            if let Some(p) = inchat_send_scan {
                log_info(
                    "submit_prompt",
                    &format!("in-chat send icon matched at ({},{})", p.0, p.1),
                );
                v.push(p);
            }
            v
        } else if in_chat_send {
            let mut v = Vec::new();
            if let Some(p) = inchat_send_scan {
                log_info(
                    "submit_prompt",
                    &format!("in-chat send icon matched at ({},{})", p.0, p.1),
                );
                v.push(p);
            }
            v
        } else {
            // Fresh-chat pages can show non-send buttons below the input
            // (for example data-use/training notices). Do not probe random
            // y positions here; use only the calibrated send region, then
            // fall back to Return if needed.
            let mut v = Vec::new();
            if let Some(p) = fresh_send_by_template {
                log_info(
                    "submit_prompt",
                    &format!("fresh send icon matched at ({},{})", p.0, p.1),
                );
                v.push(p);
            }
            v.push((sx, base_y));
            v.sort();
            v.dedup();
            v
        };

        // Measure the input region where paste landed for send verification.
        let input_measure = if pasted_at_code_mode {
            self.regions.input_focus_point_code_mode
        } else if in_chat {
            self.regions.input_focus_point_inchat
        } else {
            self.regions.input_focus_point
        };

        let mut sent = false;
        for &(send_x, send_y) in &send_candidates {
            let pre_input_var = region_variance(input_measure);
            log_info(
                "submit_prompt",
                &format!(
                    "click send ({send_x},{send_y}) in_chat={in_chat_send} pre_input_var={pre_input_var:.1}"
                ),
            );
            crate::human_simulations::move_mouse_single_click(send_x, send_y).map_err(|e| {
                if let Some(s) = &saved_clip {
                    let _ = clip.set_text(s.clone());
                }
                AdapterError::Input(e)
            })?;
            std::thread::sleep(std::time::Duration::from_millis(700));
            self.move_mouse_away_from_prompt_button()?;
            std::thread::sleep(std::time::Duration::from_millis(800));

            if in_chat_send {
                // Check if the input area cleared (text consumed by send).
                let post_input_var = region_variance(input_measure);
                let dropped = pre_input_var > 30.0 && post_input_var < pre_input_var * 0.5;
                log_info(
                    "submit_prompt",
                    &format!(
                        "send check: input var {pre_input_var:.1} → {post_input_var:.1} cleared={dropped}"
                    ),
                );
                if dropped && self.confirm_submission_started(pre_send_conv.as_ref())? {
                    log_info(
                        "submit_prompt",
                        "sent (in-chat) — input cleared and generation/change confirmed",
                    );
                    sent = true;
                    break;
                }
            } else {
                let post_url = crate::webui::browser_state::active_chrome_url().unwrap_or_default();
                if post_url.contains("/c/") {
                    log_info("submit_prompt", &format!("sent — URL now {post_url}"));
                    sent = true;
                    break;
                }
                log_info(
                    "submit_prompt",
                    &format!("URL still '{post_url}' — trying next"),
                );
            }
        }

        if !sent {
            log_warn(
                "submit_prompt",
                "send button missed — trying osascript Return",
            );
            let pre_input_var = region_variance(input_measure);
            let _ = std::process::Command::new("osascript")
                .args(["-e", "tell application \"System Events\" to key code 36"])
                .output();
            std::thread::sleep(std::time::Duration::from_millis(2000));
            if in_chat_send {
                let post_input_var = region_variance(input_measure);
                if pre_input_var > 30.0
                    && post_input_var < pre_input_var * 0.5
                    && self.confirm_submission_started(pre_send_conv.as_ref())?
                {
                    log_info(
                        "submit_prompt",
                        "sent via Return (in-chat) — input cleared and generation/change confirmed",
                    );
                    sent = true;
                }
            } else {
                let post_url = crate::webui::browser_state::active_chrome_url().unwrap_or_default();
                if post_url.contains("/c/") {
                    log_info(
                        "submit_prompt",
                        &format!("sent via Return — URL now {post_url}"),
                    );
                    sent = true;
                }
            }
        }

        if !sent {
            save_debug_png(
                "submit_send_failed",
                Region {
                    x1: 0,
                    y1: 600,
                    x2: self.regions.screen_width,
                    y2: self.regions.screen_height,
                },
            );
            if let Some(s) = saved_clip {
                let _ = clip.set_text(s);
            }
            log_warn(
                "submit_prompt",
                "could not confirm send — check the run screenshots folder",
            );
            return Err(AdapterError::Input(
                "prompt pasted but send could not be confirmed; refusing to continue with stale input"
                    .to_string()
            ));
        }

        log_info(
            "submit_prompt",
            "submit complete — leaving scroll position unchanged",
        );

        // Restore clipboard.
        if let Some(s) = saved_clip {
            let _ = clip.set_text(s);
        }

        // Reset variance tracking so post-submit detection starts fresh.
        // Done at the END (not start) because internal detect_state calls
        // during send verification pollute the counters.
        *self.last_conv_var.lock().unwrap() = None;
        *self.stable_polls.lock().unwrap() = 0;
        *self.saw_variance_change.lock().unwrap() = false;

        Ok(())
    }

    fn copy_response(&self) -> Result<String, AdapterError> {
        let in_chat = matches!(classify_url(), UrlClass::InChat);
        let footer_regions = if in_chat {
            [
                self.regions.message_footer_inchat,
                self.regions.message_footer,
            ]
        } else {
            [
                self.regions.message_footer,
                self.regions.message_footer_inchat,
            ]
        };

        // Snapshot clipboard before clicking — if it doesn't change after
        // the click, the copy button wasn't hit (error state, wrong
        // position, etc.).
        let pre_clip = crate::human_simulations::read_clipboard().unwrap_or_default();

        self.resolve_response_choice_if_present()?;

        if self.regions.surface == ArenaSurface::Text {
            let candidates = self.find_text_response_copy_candidates()?;
            if candidates.is_empty() {
                save_debug_png("text_copy_button_not_found", self.text_copy_scan_region());
                return Err(AdapterError::NotFound {
                    what: "text response copy button above input".to_string(),
                });
            }

            for (cx, cy) in candidates {
                eprintln!(
                    "[adapter] copy_response: text verified copy candidate click ({cx},{cy})"
                );
                crate::human_simulations::move_mouse_single_click(cx, cy)
                    .map_err(AdapterError::Input)?;
                std::thread::sleep(std::time::Duration::from_millis(800));

                let post_clip =
                    crate::human_simulations::read_clipboard().map_err(AdapterError::Clipboard)?;
                if post_clip != pre_clip && !post_clip.trim().is_empty() {
                    return Ok(post_clip);
                }
            }

            save_debug_png(
                "text_copy_button_clicked_no_clipboard",
                self.text_copy_scan_region(),
            );
            return Err(AdapterError::Clipboard(
                "text response copy button clicked but clipboard did not change".to_string(),
            ));
        }

        let mut copy_pos = None;
        for region in footer_regions {
            if copy_pos.is_some() {
                break;
            }
            if let Some(p) = template_center_in_region(
                region,
                templates::copy_button(),
                Tolerance::NORMAL,
                0.72,
            )? {
                copy_pos = Some(p);
                break;
            }
        }

        let Some((cx, cy)) = copy_pos else {
            eprintln!(
                "[adapter] copy_response: no verified copy button — returning empty response without clicking page"
            );
            return Ok(String::new());
        };

        eprintln!("[adapter] copy_response: in_chat={in_chat} verified copy click ({cx},{cy})");
        crate::human_simulations::move_mouse_single_click(cx, cy).map_err(AdapterError::Input)?;
        std::thread::sleep(std::time::Duration::from_millis(300));

        let post_clip =
            crate::human_simulations::read_clipboard().map_err(AdapterError::Clipboard)?;

        if post_clip != pre_clip && !post_clip.is_empty() {
            return Ok(post_clip);
        }

        eprintln!("[adapter] copy_response: clipboard unchanged after verified copy click");
        Ok(String::new())
    }

    fn start_new_chat(&self) -> Result<(), AdapterError> {
        // URL pattern is the cleanest "are we on an active chat?" signal:
        //   - /c/<id>             → an active chat with prior messages.
        //                           Need to click "+ New Chat" to reset.
        //   - /text/direct, /code/direct, /, /text, /code → already on
        //                           Arena's fresh-chat entry page. NO-OP.
        //
        // This avoids false-fail on the case where the user is already
        // sitting on a fresh chat (conv region has greeting text + input UI
        // that put variance above our 200 threshold).
        let pre_url = crate::webui::browser_state::active_chrome_url().unwrap_or_default();
        if !pre_url.contains("/c/") {
            eprintln!(
                "[adapter] start_new_chat: URL '{pre_url}' is not an active /c/ chat — already fresh, no click needed"
            );
            return Ok(());
        }

        // We ARE on an active chat — click "+ New Chat" and verify reset.
        let btn = self.regions.new_chat_button_when_collapsed;
        let conv = self.regions.conversation;
        let (cx, cy) = center(btn);

        let pre_var = region_variance(conv);
        eprintln!(
            "[adapter] start_new_chat: ON ACTIVE CHAT '{pre_url}' (conv var={pre_var:.1})  clicking ({cx},{cy})"
        );

        crate::human_simulations::move_mouse_single_click(cx, cy).map_err(AdapterError::Input)?;
        std::thread::sleep(std::time::Duration::from_millis(900));

        let post_var = region_variance(conv);
        let post_url = crate::webui::browser_state::active_chrome_url().unwrap_or_default();
        let drop_pct = if pre_var > 0.1 {
            (pre_var - post_var) / pre_var * 100.0
        } else {
            0.0
        };
        eprintln!(
            "[adapter] start_new_chat: post-conv-variance={post_var:.1} (drop {drop_pct:.1}%)  post-URL='{post_url}'"
        );

        // Acceptance for an active-chat → fresh-chat transition:
        //   - URL left /c/<id> for /text/direct or similar, OR
        //   - URL changed to a different /c/<id>, OR
        //   - Conv variance dropped ≥ 40% (messages cleared even if URL stayed).
        let url_left_active = !post_url.contains("/c/");
        let url_changed = post_url != pre_url;
        let cleared_content = pre_var > 200.0 && drop_pct >= 40.0;
        if url_left_active || url_changed || cleared_content {
            let why = if url_left_active {
                "URL left /c/ pattern"
            } else if url_changed {
                "URL changed to different /c/"
            } else {
                "conversation variance dropped ≥40%"
            };
            eprintln!("[adapter] start_new_chat: verified — {why}");
            return Ok(());
        }
        Err(AdapterError::Input(format!(
            "start_new_chat: clicked ({cx},{cy}) but no signal of new chat. \
             URL stayed '{post_url}' AND conv variance {pre_var:.1} → {post_var:.1} (drop {drop_pct:.1}%, need ≥40%). \
             New-chat button may be miscalibrated."
        )))
    }

    fn dismiss_modal(&self) -> Result<bool, AdapterError> {
        // Don't auto-dismiss the chat-full modal — the FSM routes that to
        // StartingNewChat which is the correct response.
        if region_matches(
            self.regions.chat_full_modal,
            templates::chat_full_modal(),
            Tolerance::NORMAL,
            0.88,
        )? {
            return Ok(false);
        }
        crate::human_simulations::press_key("escape").map_err(AdapterError::Input)?;
        Ok(true)
    }

    fn download_artifacts(&self) -> Result<bool, AdapterError> {
        self.download_artifacts_to(&crate::output::writer::results_root())
    }

    fn download_artifacts_to(&self, dest_dir: &Path) -> Result<bool, AdapterError> {
        if self.regions.surface == ArenaSurface::Text {
            log_info(
                "download_artifacts",
                "text surface — response saved from copy_response; no zip download",
            );
            return Ok(false);
        }

        let code_panel_visible = match self.has_code_panel() {
            Ok(v) => v,
            Err(e) => {
                log_warn(
                    "download_artifacts",
                    &format!("code panel check failed: {e}"),
                );
                false
            }
        };
        let download_ready = match self.download_button_active_state() {
            Ok(Some(true)) => {
                log_info(
                    "download_artifacts",
                    "download button active/bright — downloading zip",
                );
                true
            }
            Ok(Some(false)) => {
                log_warn(
                    "download_artifacts",
                    "download button visible but inactive/dim — skipping download",
                );
                false
            }
            Ok(None) => false,
            Err(e) => {
                log_warn(
                    "download_artifacts",
                    &format!("download button scan failed: {e}"),
                );
                false
            }
        };

        if code_panel_visible || download_ready {
            log_info(
                "download_artifacts",
                "code panel detected — downloading zip",
            );
            fs::create_dir_all(dest_dir)
                .map_err(|e| AdapterError::Other(format!("create artifact dir: {e}")))?;
            let preview_path = dest_dir.join("design_preview.png");
            if let Err(e) = self.save_preview_screenshot_path(&preview_path) {
                log_warn(
                    "download_artifacts",
                    &format!("preview screenshot failed: {e}"),
                );
            }
            let before = newest_download_file();
            if !self.download_code_artifacts()? {
                log_warn(
                    "download_artifacts",
                    "download button not found by image scan — skipping download",
                );
                return Ok(false);
            }
            match wait_and_move_download(before.as_ref(), dest_dir) {
                Ok(moved) => {
                    log_info(
                        "download_artifacts",
                        &format!("download moved → {}", moved.display()),
                    );
                    Ok(true)
                }
                Err(e) => {
                    log_warn("download_artifacts", &format!("download did not land: {e}"));
                    Ok(false)
                }
            }
        } else {
            log_info(
                "download_artifacts",
                "no code panel or active download button — skipping download",
            );
            Ok(false)
        }
    }

    fn upload_file(&self, path: &Path) -> Result<bool, AdapterError> {
        use std::time::Duration;

        let abs = path
            .canonicalize()
            .map_err(|e| AdapterError::Other(format!("upload_file: cannot resolve path: {e}")))?;
        let path_str = abs.to_string_lossy().into_owned();
        log_info("upload_file", &format!("attaching {path_str}"));

        // Pick regions based on chat state.
        let state = self.detect_state().unwrap_or(WebUiState::Unknown);
        let (attach_scan, send_region, input_box) = if matches!(state, WebUiState::Ready) {
            (
                self.regions.attach_scan_fresh,
                self.regions.send_or_stop,
                self.regions.input_box,
            )
        } else {
            (
                self.regions.attach_scan_inchat,
                self.regions.send_or_stop_inchat,
                self.regions.input_box,
            )
        };

        // Remove any pre-existing attachment before uploading.
        match arena_clear_existing_attachment(input_box) {
            Ok(true) => log_info("upload_file", "cleared pre-existing attachment"),
            Ok(false) => {}
            Err(e) => log_info(
                "upload_file",
                &format!("clear_existing_attachment warning: {e}"),
            ),
        }

        // Scan for the paperclip icon.
        let (ax, ay) = find_attach_button_in_region(attach_scan)
            .map_err(|e| AdapterError::Input(format!("attach scan failed: {e}")))?
            .ok_or_else(|| AdapterError::NotFound {
                what: "paperclip/attach button not found in scan region".into(),
            })?;
        log_info("upload_file", &format!("attach button at ({ax}, {ay})"));

        // Snapshot screen before click to verify dialog opens.
        let dialog_check = self.regions.conversation;
        let pre_var = capture_region_variance(dialog_check);

        crate::human_simulations::move_mouse_single_click(ax, ay)
            .map_err(|e| AdapterError::Input(format!("attach click failed: {e}")))?;
        std::thread::sleep(Duration::from_millis(1200));

        let post_var = capture_region_variance(dialog_check);
        if (post_var - pre_var).abs() < 100.0 {
            return Err(AdapterError::NotFound {
                what: "file dialog did not appear after clicking attach button".into(),
            });
        }

        osascript_open_file_dialog(&path_str)
            .map_err(|e| AdapterError::Input(format!("file dialog navigation failed: {e}")))?;

        // Poll the send button for up to 2 minutes until the upload finishes.
        arena_wait_for_upload_complete(send_region, Duration::from_secs(120)).map_err(|e| {
            AdapterError::Timeout {
                what: format!("upload polling: {e}"),
                after_ms: 120_000,
            }
        })?;

        log_info("upload_file", "file attached and ready");
        Ok(true)
    }
}

// ── Prompt-submission helpers ─────────────────────────────────────────────

impl ArenaAdapter {
    fn move_mouse_away_from_prompt_button(&self) -> Result<(), AdapterError> {
        let x = (self.regions.conversation.x1 + 120).min(self.regions.screen_width - 20);
        let y = (self.regions.conversation.y1 + 90).min(self.regions.screen_height - 20);
        log_info(
            "submit_prompt",
            &format!("move cursor away from prompt button ({x},{y})"),
        );
        crate::human_simulations::move_mouse(x, y).map_err(AdapterError::Input)
    }

    fn find_send_button_on_screen(&self) -> Result<Option<(i32, i32)>, AdapterError> {
        // Search only where the textarea's action button can legitimately
        // live. Broad bottom-screen scans matched unrelated square-ish UI
        // (Code button / scroll affordances) and caused false sends.
        let fresh_input_right = Region {
            x1: 1060.min(self.regions.screen_width),
            y1: 650.min(self.regions.screen_height),
            x2: 1190.min(self.regions.screen_width),
            y2: 815.min(self.regions.screen_height),
        };
        let code_input_right = Region {
            x1: 390.min(self.regions.screen_width),
            y1: (self.regions.screen_height - 125).max(0),
            x2: 500.min(self.regions.screen_width),
            y2: (self.regions.screen_height - 20).max(0),
        };
        let inchat_input_right = Region {
            x1: 900.min(self.regions.screen_width),
            y1: (self.regions.screen_height - 125).max(0),
            x2: 1180.min(self.regions.screen_width),
            y2: (self.regions.screen_height - 20).max(0),
        };
        let search_regions: Vec<(&str, Region, f32)> = if self.regions.surface == ArenaSurface::Text
        {
            vec![
                ("fresh_send", self.regions.send_or_stop, 0.78),
                ("text_inchat_send", self.regions.send_or_stop_inchat, 0.78),
                ("fresh_input_right", fresh_input_right, 0.74),
                ("text_input_right", inchat_input_right, 0.76),
            ]
        } else {
            vec![
                ("fresh_send", self.regions.send_or_stop, 0.78),
                ("code_send", self.regions.send_or_stop_code_mode, 0.78),
                ("inchat_send", self.regions.send_or_stop_inchat, 0.78),
                ("fresh_input_right", fresh_input_right, 0.74),
                ("code_input_right", code_input_right, 0.76),
                ("inchat_input_right", inchat_input_right, 0.76),
            ]
        };
        for (label, region, threshold) in search_regions {
            match template_center_in_region(
                region,
                templates::send_button(),
                Tolerance::NORMAL,
                threshold,
            )? {
                Some(p) => {
                    log_info(
                        "submit_prompt",
                        &format!(
                            "send button matched in {label} region at ({},{}) threshold={threshold}",
                            p.0, p.1
                        ),
                    );
                    return Ok(Some(p));
                }
                None => log_info(
                    "submit_prompt",
                    &format!(
                        "send button not found in {label} region ({},{})→({},{})",
                        region.x1, region.y1, region.x2, region.y2
                    ),
                ),
            }
        }
        Ok(None)
    }

    fn find_latest_response_copy_button(&self) -> Result<Option<(i32, i32)>, AdapterError> {
        Ok(self
            .find_text_response_copy_candidates()?
            .into_iter()
            .next())
    }

    fn response_choice_scan_region(&self) -> Region {
        let input_top = if matches!(classify_url(), UrlClass::InChat) {
            self.regions.input_focus_point_inchat.y1
        } else {
            self.regions.input_box.y1
        };
        let y2 = (input_top + 24).min(self.regions.screen_height);
        let y1 = (input_top - 120).max(self.regions.conversation.y1);
        let x1 = ((self.regions.screen_width as f32) * 0.32).round() as i32;
        let x2 = ((self.regions.screen_width as f32) * 0.70).round() as i32;
        Region {
            x1: x1.max(0),
            y1,
            x2: x2.min(self.regions.screen_width),
            y2,
        }
    }

    fn response_choice_skip_expected_point(&self) -> (i32, i32) {
        let input_top = if matches!(classify_url(), UrlClass::InChat) {
            self.regions.input_focus_point_inchat.y1
        } else {
            self.regions.input_box.y1
        };
        (
            (self.regions.input_box.x1 + self.regions.input_box.x2) / 2,
            (input_top - 56).max(self.regions.conversation.y1),
        )
    }

    fn find_response_choice_skip_button(&self) -> Result<Option<(i32, i32)>, AdapterError> {
        if self.regions.surface == ArenaSurface::Text {
            return Ok(None);
        }

        let scan = self.response_choice_scan_region();
        let template_hit = template_center_in_region(
            scan,
            templates::choice_skip_button(),
            Tolerance::NORMAL,
            0.84,
        )?;
        if let Some((cx, cy)) = template_hit {
            if !self.response_choice_button_row_visible()? {
                log_info(
                    "choice",
                    &format!(
                        "response chooser Skip template matched at ({cx},{cy}) but row probe rejected it"
                    ),
                );
                return Ok(None);
            }
            log_info(
                "choice",
                &format!(
                    "response chooser Skip matched at ({cx},{cy}) in ({},{})→({},{})",
                    scan.x1, scan.y1, scan.x2, scan.y2
                ),
            );
            return Ok(Some((cx, cy)));
        }

        if self.response_choice_button_row_visible()? {
            let (cx, cy) = self.response_choice_skip_expected_point();
            log_info(
                "choice",
                &format!("response chooser row visible; fallback Skip click at ({cx},{cy})"),
            );
            return Ok(Some((cx, cy)));
        }

        log_info(
            "choice",
            &format!(
                "response chooser Skip not found in ({},{})→({},{})",
                scan.x1, scan.y1, scan.x2, scan.y2
            ),
        );
        Ok(None)
    }

    fn response_choice_button_row_visible(&self) -> Result<bool, AdapterError> {
        let (cx, cy) = self.response_choice_skip_expected_point();
        let band = Region {
            x1: (cx - 120).max(0),
            y1: (cy - 35).max(self.regions.conversation.y1),
            x2: (cx + 120).min(self.regions.screen_width),
            y2: (cy + 35).min(self.regions.screen_height),
        };
        let cap =
            crate::image_matrix::capture::capture_gray_matrix(band.x1, band.y1, band.x2, band.y2)
                .map_err(|e| AdapterError::Capture(e.to_string()))?;

        let w = cap.width;
        let h = cap.height;
        if w == 0 || h == 0 {
            return Ok(false);
        }

        let mut rows_with_long_edge = 0;
        let mut max_bright_in_row = 0usize;
        for y in 0..h {
            let row = &cap.data[y * w..(y + 1) * w];
            let bright = row.iter().filter(|&&v| v >= 74).count();
            max_bright_in_row = max_bright_in_row.max(bright);
            if bright >= 90 {
                rows_with_long_edge += 1;
            }
        }

        let mean = cap.data.iter().map(|&v| v as f64).sum::<f64>() / cap.data.len() as f64;
        let variance = cap
            .data
            .iter()
            .map(|&v| (v as f64 - mean).powi(2))
            .sum::<f64>()
            / cap.data.len() as f64;
        let bright_ratio =
            cap.data.iter().filter(|&&v| v >= 120).count() as f64 / cap.data.len() as f64;

        let visible = rows_with_long_edge >= 2 && variance >= 45.0 && bright_ratio >= 0.006;
        log_info(
            "choice",
            &format!(
                "response chooser row probe ({},{})→({},{}) max_row_bright={} edge_rows={} var={variance:.1} bright_ratio={bright_ratio:.3} visible={visible}",
                band.x1, band.y1, band.x2, band.y2, max_bright_in_row, rows_with_long_edge
            ),
        );
        Ok(visible)
    }

    fn resolve_response_choice_if_present(&self) -> Result<(), AdapterError> {
        let Some((cx, cy)) = self.find_response_choice_skip_button()? else {
            return Ok(());
        };

        log_info(
            "choice",
            &format!("click Skip to force single response ({cx},{cy})"),
        );
        crate::human_simulations::move_mouse_single_click(cx, cy).map_err(AdapterError::Input)?;
        std::thread::sleep(Duration::from_millis(900));
        let _ = self.move_mouse_away_from_prompt_button();

        self.wait_for_choice_regeneration()
    }

    fn wait_for_choice_regeneration(&self) -> Result<(), AdapterError> {
        let deadline = Instant::now() + Duration::from_secs(180);
        let mut extra_skip_clicks = 0;
        loop {
            if Instant::now() >= deadline {
                save_debug_png("choice_skip_timeout", self.response_choice_scan_region());
                return Err(AdapterError::Other(
                    "timed out waiting for Arena Skip to regenerate a single response".to_string(),
                ));
            }

            if let Some((cx, cy)) = self.find_response_choice_skip_button()? {
                if extra_skip_clicks < 3 {
                    extra_skip_clicks += 1;
                    log_info(
                        "choice",
                        &format!("Skip still visible; retry click {extra_skip_clicks} ({cx},{cy})"),
                    );
                    crate::human_simulations::move_mouse_single_click(cx, cy)
                        .map_err(AdapterError::Input)?;
                    let _ = self.move_mouse_away_from_prompt_button();
                }
                std::thread::sleep(Duration::from_millis(1200));
                continue;
            }

            if self.regions.surface == ArenaSurface::Text {
                if self.text_stop_square_visible()? {
                    std::thread::sleep(Duration::from_millis(1500));
                    continue;
                }
                if self.find_latest_response_copy_button()?.is_some() {
                    log_info("choice", "single response ready after Skip");
                    return Ok(());
                }
            } else {
                if self.find_stop_button_on_screen()?.is_some() {
                    std::thread::sleep(Duration::from_millis(1500));
                    continue;
                }
                if let Some(active) = self.download_button_active_state()? {
                    if active {
                        log_info("choice", "code response ready after Skip");
                        return Ok(());
                    }
                }
                if self.has_code_panel()? {
                    log_info("choice", "code panel ready after Skip");
                    return Ok(());
                }
            }

            std::thread::sleep(Duration::from_millis(1500));
        }
    }

    fn text_copy_scan_region(&self) -> Region {
        let y2 = self.regions.input_box.y1.min(self.regions.conversation.y2);
        let y1 = (y2 - 95).max(self.regions.conversation.y1);
        let x1 = self.regions.conversation.x1;
        Region {
            x1,
            y1,
            x2: (x1 + 190).min(self.regions.screen_width),
            y2,
        }
    }

    fn find_text_response_copy_candidates(&self) -> Result<Vec<(i32, i32)>, AdapterError> {
        let templates = templates::copy_button();
        if templates.is_empty() {
            return Ok(Vec::new());
        }

        // Text mode footer copy button sits just above the input, after
        // like/dislike/retry. Keep the scan in that footer strip only:
        // scanning the whole answer false-matches tables and prose.
        let scan = self.text_copy_scan_region();
        let cap =
            crate::image_matrix::capture::capture_gray_matrix(scan.x1, scan.y1, scan.x2, scan.y2)
                .map_err(|e| AdapterError::Capture(e.to_string()))?;

        let mut candidates: Vec<(i32, i32, f32)> = Vec::new();
        for template in templates {
            for hit in find_all_matches_gray(&cap, template, Tolerance::NORMAL, 0.88) {
                let cx = scan.x1 + hit.col as i32 + template.width as i32 / 2;
                let cy = scan.y1 + hit.row as i32 + template.height as i32 / 2;
                candidates.push((cx, cy, hit.score));
            }
        }

        let Some(max_y) = candidates.iter().map(|(_, y, _)| *y).max() else {
            log_info(
                "copy_response",
                &format!(
                    "text latest response copy not found in ({},{})→({},{})",
                    scan.x1, scan.y1, scan.x2, scan.y2
                ),
            );
            return Ok(Vec::new());
        };

        let expected_copy_x = scan.x1 + ((scan.x2 - scan.x1) as f32 * 0.63).round() as i32;
        candidates.retain(|(_, y, _)| *y >= max_y - 28);
        candidates.sort_by(|a, b| {
            (a.0 - expected_copy_x)
                .abs()
                .cmp(&(b.0 - expected_copy_x).abs())
                .then_with(|| b.1.cmp(&a.1))
                .then_with(|| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal))
        });
        candidates.dedup_by(|a, b| (a.0 - b.0).abs() < 16 && (a.1 - b.1).abs() < 12);

        let converted: Vec<(i32, i32)> = candidates
            .into_iter()
            .take(6)
            .map(|(x, y, score)| {
                let (mx, my) = capture_to_mouse_point((x, y));
                log_info(
                    "copy_response",
                    &format!("text response copy candidate at ({mx},{my}) score={score:.2}"),
                );
                (mx, my)
            })
            .collect();

        if !converted.is_empty() {
            return Ok(converted);
        }

        log_info(
            "copy_response",
            &format!(
                "text latest response copy not found in ({},{})→({},{})",
                scan.x1, scan.y1, scan.x2, scan.y2
            ),
        );
        Ok(Vec::new())
    }

    fn text_stop_square_visible(&self) -> Result<bool, AdapterError> {
        let (cx, cy) = center(self.regions.send_or_stop_inchat);
        let icon_core = Region {
            x1: (cx - 14).max(0),
            y1: (cy - 14).max(0),
            x2: (cx + 14).min(self.regions.screen_width),
            y2: (cy + 14).min(self.regions.screen_height),
        };
        let bright_ratio = region_bright_ratio(icon_core, 190);
        let mean = region_mean(icon_core);

        // Text mode stop icon is a filled square; send icon is a thin arrow.
        // Template matching confused the arrow for the square at low threshold,
        // so use fill density inside the icon core.
        let visible = bright_ratio >= 0.18;
        log_info(
            "detect_state",
            &format!(
                "text stop-square core ({},{})→({},{}) bright_ratio={bright_ratio:.3} mean={mean:.1} visible={visible}",
                icon_core.x1, icon_core.y1, icon_core.x2, icon_core.y2
            ),
        );
        Ok(visible)
    }

    fn prompt_button_scan_regions(&self) -> Vec<(&'static str, Region)> {
        let fresh_input_right = Region {
            x1: 1060.min(self.regions.screen_width),
            y1: 650.min(self.regions.screen_height),
            x2: 1190.min(self.regions.screen_width),
            y2: 815.min(self.regions.screen_height),
        };
        let code_input_right = Region {
            x1: 390.min(self.regions.screen_width),
            y1: (self.regions.screen_height - 125).max(0),
            x2: 500.min(self.regions.screen_width),
            y2: (self.regions.screen_height - 20).max(0),
        };
        let inchat_input_right = Region {
            x1: 900.min(self.regions.screen_width),
            y1: (self.regions.screen_height - 125).max(0),
            x2: 1180.min(self.regions.screen_width),
            y2: (self.regions.screen_height - 20).max(0),
        };
        let text_prompt_column = Region {
            x1: 1010.min(self.regions.screen_width),
            y1: 640.min(self.regions.screen_height),
            x2: 1210.min(self.regions.screen_width),
            y2: self.regions.screen_height,
        };

        let mut regions = vec![
            ("fresh_prompt_button", self.regions.send_or_stop),
            ("inchat_prompt_button", self.regions.send_or_stop_inchat),
            ("fresh_input_right", fresh_input_right),
            ("inchat_input_right", inchat_input_right),
            ("text_prompt_column", text_prompt_column),
        ];
        if self.regions.surface == ArenaSurface::Code {
            regions.push(("code_prompt_button", self.regions.send_or_stop_code_mode));
            regions.push(("code_input_right", code_input_right));
        }
        regions
    }

    fn central_white_square_region(&self) -> Region {
        Region {
            x1: (self.regions.screen_width / 4).max(0),
            y1: (self.regions.screen_height / 5).max(0),
            x2: (self.regions.screen_width * 3 / 4).min(self.regions.screen_width),
            y2: (self.regions.screen_height * 2 / 3).min(self.regions.screen_height),
        }
    }

    fn prompt_action_button_regions(&self) -> Vec<(&'static str, Region)> {
        self.prompt_button_scan_regions()
    }

    fn find_rerun_button_in_prompt_slot(&self) -> Result<Option<(i32, i32)>, AdapterError> {
        for (label, region) in self.prompt_action_button_regions() {
            if let Some((cx, cy)) = template_center_in_region(
                region,
                templates::rerun_button(),
                Tolerance::NORMAL,
                0.70,
            )? {
                log_info(
                    "detect_state",
                    &format!("rerun icon matched in {label} at ({cx},{cy})"),
                );
                return Ok(Some((cx, cy)));
            }
        }
        Ok(None)
    }

    fn find_bright_stop_square_in_region(
        &self,
        label: &str,
        region: Region,
    ) -> Result<Option<(i32, i32)>, AdapterError> {
        let cap = crate::image_matrix::capture::capture_gray_matrix(
            region.x1, region.y1, region.x2, region.y2,
        )
        .map_err(|e| AdapterError::Capture(e.to_string()))?;

        let width = cap.width;
        let height = cap.height;
        if width == 0 || height == 0 {
            return Ok(None);
        }

        let mut visited = vec![false; cap.data.len()];
        let mut best: Option<(i32, i32, usize, f64)> = None;
        let mut best_rejected: Option<(usize, usize, usize, f64, f64)> = None;

        for y in 0..height {
            for x in 0..width {
                let idx = y * width + x;
                if visited[idx] || cap.data[idx] < 200 {
                    continue;
                }

                let mut stack = vec![(x, y)];
                visited[idx] = true;
                let mut min_x = x;
                let mut max_x = x;
                let mut min_y = y;
                let mut max_y = y;
                let mut area = 0usize;
                let mut sum = 0usize;

                while let Some((cx, cy)) = stack.pop() {
                    let cidx = cy * width + cx;
                    area += 1;
                    sum += cap.data[cidx] as usize;
                    min_x = min_x.min(cx);
                    max_x = max_x.max(cx);
                    min_y = min_y.min(cy);
                    max_y = max_y.max(cy);

                    let x0 = cx.saturating_sub(1);
                    let x1 = (cx + 1).min(width - 1);
                    let y0 = cy.saturating_sub(1);
                    let y1 = (cy + 1).min(height - 1);
                    for ny in y0..=y1 {
                        for nx in x0..=x1 {
                            let nidx = ny * width + nx;
                            if !visited[nidx] && cap.data[nidx] >= 200 {
                                visited[nidx] = true;
                                stack.push((nx, ny));
                            }
                        }
                    }
                }

                let bw = max_x - min_x + 1;
                let bh = max_y - min_y + 1;
                let fill = area as f64 / (bw * bh) as f64;
                let mean = sum as f64 / area as f64;

                let rejected_score = area as f64 * mean;
                let replace_rejected =
                    best_rejected
                        .as_ref()
                        .is_none_or(|(_, _, prev_area, prev_mean, _)| {
                            rejected_score > *prev_area as f64 * *prev_mean
                        });
                if replace_rejected {
                    best_rejected = Some((bw, bh, area, mean, fill));
                }

                if !(32..=75).contains(&bw) || !(32..=75).contains(&bh) {
                    continue;
                }
                if bw.abs_diff(bh) > 18 {
                    continue;
                }
                if area < 900 {
                    continue;
                }
                if fill < 0.55 || mean < 230.0 {
                    continue;
                }
                if !has_dark_frame_around_component(&cap, min_x, max_x, min_y, max_y) {
                    continue;
                }

                let abs_x = region.x1 + ((min_x + max_x + 1) / 2) as i32;
                let abs_y = region.y1 + ((min_y + max_y + 1) / 2) as i32;
                let score = area as f64 * mean;
                let replace = best.as_ref().is_none_or(|(_, _, prev_area, prev_mean)| {
                    score > *prev_area as f64 * *prev_mean
                });
                if replace {
                    best = Some((abs_x, abs_y, area, mean));
                }
            }
        }

        if let Some((x, y, area, mean)) = best {
            let (mx, my) = capture_to_mouse_point((x, y));
            log_info(
                "watchdog",
                &format!(
                    "white stop-square matrix match in {label} ({},{})→({},{}) at ({mx},{my}) area={area} mean={mean:.1} (large + dark frame)",
                    region.x1, region.y1, region.x2, region.y2
                ),
            );
            Ok(Some((mx, my)))
        } else {
            if let Some((bw, bh, area, mean, fill)) = best_rejected {
                log_info(
                    "watchdog",
                    &format!(
                        "no white stop-square in {label} ({},{})→({},{}); best bright blob {}×{} area={area} mean={mean:.1} fill={fill:.2}",
                        region.x1, region.y1, region.x2, region.y2, bw, bh
                    ),
                );
            }
            Ok(None)
        }
    }

    fn find_white_stop_square(&self) -> Result<Option<(i32, i32)>, AdapterError> {
        let region = self.central_white_square_region();
        if let Some(p) = self.find_bright_stop_square_in_region("center_screen", region)? {
            return Ok(Some(p));
        }
        Ok(None)
    }

    fn click_stop_square_watchdog_if_visible(&self) -> Result<bool, AdapterError> {
        let stop_pos = self.find_white_stop_square()?;
        let Some((cx, cy)) = stop_pos else {
            return Ok(false);
        };

        log_info(
            "watchdog",
            &format!("stop square visible — click ({cx},{cy}), wait 15s"),
        );
        crate::human_simulations::move_mouse_single_click(cx, cy).map_err(AdapterError::Input)?;
        std::thread::sleep(Duration::from_secs(15));
        let _ = self.move_mouse_away_from_prompt_button();
        Ok(true)
    }

    fn find_stop_button_on_screen(&self) -> Result<Option<(i32, i32)>, AdapterError> {
        // Same guard as send scanning: the stop square is only meaningful
        // at the right edge of the active input. The previous broad scan
        // matched the bottom-left Code/toolbar area at x≈451 forever,
        // incorrectly reporting Generating after completion.
        let code_input_right = Region {
            x1: 390.min(self.regions.screen_width),
            y1: (self.regions.screen_height - 125).max(0),
            x2: 500.min(self.regions.screen_width),
            y2: (self.regions.screen_height - 20).max(0),
        };
        let inchat_input_right = Region {
            x1: 900.min(self.regions.screen_width),
            y1: (self.regions.screen_height - 125).max(0),
            x2: 1180.min(self.regions.screen_width),
            y2: (self.regions.screen_height - 20).max(0),
        };
        if self.regions.surface == ArenaSurface::Text {
            return if self.text_stop_square_visible()? {
                Ok(Some(center(self.regions.send_or_stop_inchat)))
            } else {
                Ok(None)
            };
        }

        let search_regions: Vec<(&str, Region, f32)> = vec![
            ("code_stop", self.regions.send_or_stop_code_mode, 0.78),
            ("inchat_stop", self.regions.send_or_stop_inchat, 0.78),
            ("code_input_right", code_input_right, 0.76),
            ("inchat_input_right", inchat_input_right, 0.76),
        ];
        for (label, region, threshold) in search_regions {
            match template_center_in_region(
                region,
                templates::stop_button(),
                Tolerance::NORMAL,
                threshold,
            )? {
                Some(p) => {
                    log_info(
                        "detect_state",
                        &format!(
                            "stop button matched in {label} region at ({},{}) threshold={threshold}",
                            p.0, p.1
                        ),
                    );
                    return Ok(Some(p));
                }
                None => log_info(
                    "detect_state",
                    &format!(
                        "stop button not found in {label} region ({},{})→({},{})",
                        region.x1, region.y1, region.x2, region.y2
                    ),
                ),
            }
        }
        Ok(None)
    }

    fn download_button_active_state(&self) -> Result<Option<bool>, AdapterError> {
        let toolbar = Region {
            x1: self.regions.code_panel.x1,
            y1: self.regions.code_panel.y1,
            x2: self.regions.screen_width,
            y2: self.regions.code_panel.y1 + 95,
        };
        let search_regions = [
            ("expected", self.regions.download_button, 0.70),
            ("toolbar", toolbar, 0.66),
        ];
        for (label, region, threshold) in search_regions {
            if let Some((cx, cy)) = template_center_in_region(
                region,
                templates::download_button(),
                Tolerance::NORMAL,
                threshold,
            )? {
                let button_region = Region {
                    x1: (cx - 60).max(0),
                    y1: (cy - 18).max(0),
                    x2: (cx + 60).min(self.regions.screen_width),
                    y2: (cy + 18).min(self.regions.screen_height),
                };
                let mean = region_mean(button_region);
                let active = mean >= 185.0;
                log_info(
                    "detect_state",
                    &format!(
                        "download button matched in {label} at ({cx},{cy}); mean={mean:.1} active={active}"
                    ),
                );
                return Ok(Some(active));
            }
        }

        let wide_toolbar = Region {
            x1: self.regions.code_panel.x1,
            y1: self.regions.code_panel.y1,
            x2: self.regions.screen_width,
            y2: self.regions.code_panel.y1 + 90,
        };
        if let Some(candidate) =
            find_bright_toolbar_button(wide_toolbar, self.regions.screen_width)?
        {
            let active = candidate.mean >= 185.0;
            log_info(
                "detect_state",
                &format!(
                    "download button matrix-toolbar candidate at ({},{}) bounds=({},{})→({},{}) mean={:.1} area={} active={}",
                    candidate.center.0,
                    candidate.center.1,
                    candidate.bounds.x1,
                    candidate.bounds.y1,
                    candidate.bounds.x2,
                    candidate.bounds.y2,
                    candidate.mean,
                    candidate.area,
                    active
                ),
            );
            return Ok(Some(active));
        }

        Ok(None)
    }

    fn confirm_submission_started(
        &self,
        pre_conv: Option<&GrayMatrix>,
    ) -> Result<bool, AdapterError> {
        let stop_regions: Vec<Region> = if self.regions.surface == ArenaSurface::Text {
            vec![self.regions.send_or_stop_inchat]
        } else {
            vec![
                self.regions.send_or_stop,
                self.regions.send_or_stop_inchat,
                self.regions.send_or_stop_code_mode,
            ]
        };
        for attempt in 1..=12 {
            for &r in &stop_regions {
                if region_matches(r, templates::stop_button(), Tolerance::NORMAL, 0.78)? {
                    log_info(
                        "submit_prompt",
                        &format!("send confirm: stop button visible on attempt {attempt}"),
                    );
                    return Ok(true);
                }
            }
            if let Some(pre) = pre_conv {
                let diff = region_diff_pct(pre, self.regions.conversation, 8)?;
                if diff >= 0.8 {
                    log_info(
                        "submit_prompt",
                        &format!(
                            "send confirm: conversation changed {diff:.2}% on attempt {attempt}"
                        ),
                    );
                    return Ok(true);
                }
            }
            std::thread::sleep(Duration::from_millis(1000));
        }
        log_warn(
            "submit_prompt",
            "input cleared but no stop button or conversation image change confirmed",
        );
        Ok(false)
    }

    /// Click at `(x, y)`, paste from clipboard, verify via a **narrow
    /// horizontal strip** at the click point. Returns `true` if paste
    /// registered.
    ///
    /// Previous approach measured a large region (770×193px) — a short
    /// prompt in a textarea only fills ~400×15px, giving a tiny Δ that
    /// fell below threshold. Now we measure a 500×30px strip centred on
    /// the click point, where even a few words produce a large Δ.
    fn try_click_and_paste(
        &self,
        x: i32,
        y: i32,
        _measure_region: Region, // kept for API compat, not used
        clip: &mut arboard::Clipboard,
        saved_clip: &Option<String>,
    ) -> Result<bool, AdapterError> {
        // Narrow measurement strip: ±250px horizontal, ±15px vertical
        // around the click point. Text in a textarea fills most of this
        // strip → variance spikes reliably.
        let strip = Region {
            x1: (x - 250).max(0),
            y1: (y - 15).max(0),
            x2: x + 250,
            y2: y + 15,
        };

        let pre_cap = crate::image_matrix::capture::capture_gray_matrix(
            strip.x1, strip.y1, strip.x2, strip.y2,
        )
        .ok();
        let pre_var = region_variance(strip);

        // Verify clipboard actually has content before clicking.
        let clip_check = clip.get_text().unwrap_or_default();
        let clip_len = clip_check.len();
        log_info(
            "try_click_and_paste",
            &format!(
                "clipboard has {clip_len} chars (first 40: {:?})",
                clip_check.chars().take(40).collect::<String>()
            ),
        );
        if clip_len == 0 {
            log_warn(
                "try_click_and_paste",
                "clipboard is EMPTY — re-setting from arboard",
            );
        }

        // Single click to focus the textarea.
        crate::human_simulations::move_mouse_single_click(x, y).map_err(|e| {
            if let Some(s) = saved_clip {
                let _ = clip.set_text(s.clone());
            }
            AdapterError::Input(e)
        })?;
        std::thread::sleep(std::time::Duration::from_millis(600));

        // Paste via osascript (System Events) — more reliable than enigo
        // for reaching React contenteditable/textarea in Chrome.
        let paste_out = std::process::Command::new("osascript")
            .args([
                "-e",
                "tell application \"System Events\" to keystroke \"v\" using command down",
            ])
            .output();
        match &paste_out {
            Ok(o) if o.status.success() => log_info("try_click_and_paste", "osascript cmd+v sent"),
            Ok(o) => log_warn(
                "try_click_and_paste",
                &format!(
                    "osascript paste non-zero exit: {}",
                    String::from_utf8_lossy(&o.stderr)
                ),
            ),
            Err(e) => log_warn(
                "try_click_and_paste",
                &format!("osascript paste failed: {e}"),
            ),
        }
        std::thread::sleep(std::time::Duration::from_millis(1200));

        let post_var = region_variance(strip);
        let delta = post_var - pre_var;

        // Also do pixel-diff if we have a pre-capture.
        let diff_pct = match (
            &pre_cap,
            crate::image_matrix::capture::capture_gray_matrix(
                strip.x1, strip.y1, strip.x2, strip.y2,
            )
            .ok(),
        ) {
            (Some(pre), Some(post)) => {
                let total = pre.data.len().max(1);
                let diff = crate::calibration::diff_pixel_count(pre, &post, 5);
                diff as f32 / total as f32 * 100.0
            }
            _ => 0.0,
        };

        log_info(
            "try_click_and_paste",
            &format!(
                "click ({x},{y}): strip var {pre_var:.1} → {post_var:.1} (Δ {delta:+.1}), diff {diff_pct:.1}%"
            ),
        );

        // Debug PNG of the strip after paste — lets us SEE whether text landed.
        save_debug_png("paste_strip_post", strip);

        // Accept if strong signal shows change (Δ≥8 was false-positive
        // from cursor blink; 1466 chars of text produces Δ>>50):
        let ok = delta >= 30.0 || diff_pct >= 10.0 || (post_var >= 200.0 && pre_var < 100.0);
        Ok(ok)
    }
}

// ── Code-mode helpers ──────────────────────────────────────────────────────
//
// Methods specific to the right-panel that appears when Arena generates a
// code project. They live as inherent methods on `ArenaAdapter` (rather
// than on `WebUiAdapter`) because the right panel is Arena-specific —
// other web UIs won't have an equivalent. The runner calls these only
// when `cfg.target_webui == Arena` AND the prompt was a code request.

impl ArenaAdapter {
    /// Returns `true` if the right-hand code panel is currently visible.
    ///
    /// Detection: any of (preview-toggle, code-toggle, download-button)
    /// templates match in their bounded regions. Three independent
    /// signals reduce the chance of a single template miss producing a
    /// false negative.
    pub fn has_code_panel(&self) -> Result<bool, AdapterError> {
        if region_matches(
            self.regions.preview_toggle,
            templates::preview_toggle(),
            Tolerance::NORMAL,
            0.80,
        )? || region_matches(
            self.regions.code_toggle,
            templates::code_toggle(),
            Tolerance::NORMAL,
            0.80,
        )? || region_matches(
            self.regions.download_button,
            templates::download_button(),
            Tolerance::NORMAL,
            0.80,
        )? {
            return Ok(true);
        }

        // Template coordinates drift when Arena changes its split width.
        // Fallback catches visible generated previews, e.g. bright/colored
        // website canvas on the right side, without treating dark full-chat
        // text as a code panel.
        let preview_body = Region {
            x1: self.regions.code_panel.x1,
            y1: self.regions.code_panel.y1 + 80,
            x2: self.regions.code_panel.x2,
            y2: self.regions.conversation.y2,
        };
        let mean = region_mean(preview_body);
        let var = region_variance(preview_body);
        let visual_panel = mean > 60.0 && var > 100.0;
        if visual_panel {
            log_info(
                "has_code_panel",
                &format!("visual right-panel signal: mean={mean:.1} var={var:.1}"),
            );
        }
        Ok(visual_panel)
    }

    /// Save the rendered website/design panel so the run leaves a visual
    /// artifact even when the code download button drifts.
    pub fn save_preview_screenshot(&self, path: &str) -> Result<(), AdapterError> {
        self.save_preview_screenshot_path(Path::new(path))
    }

    pub fn save_preview_screenshot_path(&self, path: &Path) -> Result<(), AdapterError> {
        let panel = self.regions.code_panel;
        let rgb = crate::image_matrix::capture::capture_rgb_matrix(
            panel.x1, panel.y1, panel.x2, panel.y2,
        )
        .map_err(|e| AdapterError::Capture(e.to_string()))?;
        let path_str = path.to_string_lossy();
        crate::image_matrix::io::save_rgb_matrix_as_image(&rgb, &path_str)
            .map_err(|e| AdapterError::Other(format!("save preview screenshot: {e}")))?;
        log_info(
            "download_artifacts",
            &format!("design preview saved → {}", path.display()),
        );
        Ok(())
    }

    /// Click the source-view toggle (`</>`). Useful before reading code,
    /// to make sure we're not looking at the rendered preview.
    pub fn switch_to_code_view(&self) -> Result<(), AdapterError> {
        let (cx, cy) = center(self.regions.code_toggle);
        crate::human_simulations::move_mouse_single_click(cx, cy).map_err(AdapterError::Input)?;
        std::thread::sleep(std::time::Duration::from_millis(200));
        Ok(())
    }

    /// Click the preview-view toggle (eye icon).
    pub fn switch_to_preview_view(&self) -> Result<(), AdapterError> {
        let (cx, cy) = center(self.regions.preview_toggle);
        crate::human_simulations::move_mouse_single_click(cx, cy).map_err(AdapterError::Input)?;
        std::thread::sleep(std::time::Duration::from_millis(200));
        Ok(())
    }

    /// Click the download button to save the generated project as a zip.
    ///
    /// Returns `Ok(())` once the click has fired. Confirming the file
    /// actually landed in `~/Downloads` is a separate concern — the
    /// runner watches that directory for new arrivals if it cares.
    pub fn download_code_artifacts(&self) -> Result<bool, AdapterError> {
        let toolbar = Region {
            x1: self.regions.code_panel.x1,
            y1: self.regions.code_panel.y1,
            x2: self.regions.code_panel.x2,
            y2: self.regions.code_panel.y1 + 55,
        };
        let top_band = Region {
            x1: self.regions.code_panel.x1,
            y1: self.regions.code_panel.y1,
            x2: self.regions.screen_width,
            y2: self.regions.code_panel.y1 + 80,
        };
        let search_regions = [
            ("expected", self.regions.download_button, 0.72),
            ("toolbar", toolbar, 0.70),
            ("top_band", top_band, 0.68),
        ];

        let mut found = None;
        for (label, region, threshold) in search_regions {
            match template_center_in_region(
                region,
                templates::download_button(),
                Tolerance::NORMAL,
                threshold,
            )? {
                Some((cx, cy)) => {
                    log_info(
                        "download_artifacts",
                        &format!(
                            "download button matched in {label} region at ({cx},{cy}) threshold={threshold}"
                        ),
                    );
                    found = Some((cx, cy));
                    break;
                }
                None => log_info(
                    "download_artifacts",
                    &format!(
                        "download button not found in {label} region ({},{})→({},{})",
                        region.x1, region.y1, region.x2, region.y2
                    ),
                ),
            }
        }

        if found.is_none() {
            match find_bright_toolbar_button(top_band, self.regions.screen_width)? {
                Some(candidate) if candidate.mean >= 185.0 => {
                    log_info(
                        "download_artifacts",
                        &format!(
                            "download button matrix-toolbar candidate at ({},{}) bounds=({},{})→({},{}) mean={:.1} area={}",
                            candidate.center.0,
                            candidate.center.1,
                            candidate.bounds.x1,
                            candidate.bounds.y1,
                            candidate.bounds.x2,
                            candidate.bounds.y2,
                            candidate.mean,
                            candidate.area
                        ),
                    );
                    found = Some(candidate.center);
                }
                Some(candidate) => {
                    log_warn(
                        "download_artifacts",
                        &format!(
                            "download button candidate is dim/inactive at ({},{}) mean={:.1}; skipping click",
                            candidate.center.0, candidate.center.1, candidate.mean
                        ),
                    );
                    return Ok(false);
                }
                None => {
                    log_info(
                        "download_artifacts",
                        "no matrix-toolbar download candidate found",
                    );
                }
            }
        }

        let Some((cx, cy)) = found else {
            return Ok(false);
        };
        log_info(
            "download_artifacts",
            &format!("click verified download ({cx},{cy})"),
        );
        crate::human_simulations::move_mouse_single_click(cx, cy).map_err(AdapterError::Input)?;
        std::thread::sleep(std::time::Duration::from_millis(300));
        Ok(true)
    }
}

// ── Layout-control helpers used by session_setup ───────────────────────────
//
// These methods read the screen, decide what to click, and fire the click
// at the matched template's position. No hard-coded relative offsets —
// every click is anchored to a real visual landmark.

impl ArenaAdapter {
    /// Returns `true` if the sidebar is currently in collapsed (icons-only)
    /// state. Detection: the `new_chat_dark_collapsed` template matches
    /// inside the **narrow** new-chat button region (~50 px tall).
    ///
    /// Searching the whole sidebar height was too lenient — uniform
    /// dark background let the template false-match against the Terms-
    /// of-Use footer area. Tight region + tighter threshold (0.92) +
    /// tighter tolerance (TIGHT) make this reliable.
    ///
    /// Returns `false` when the template isn't present (treated as
    /// "expanded or unknown") OR when capture fails.
    pub fn is_sidebar_collapsed(&self) -> bool {
        let templates = templates::new_chat_collapsed_only();
        if templates.is_empty() {
            return false;
        }
        match region_matches(
            self.regions.new_chat_button_when_collapsed,
            templates,
            Tolerance::TIGHT,
            0.92,
        ) {
            Ok(b) => b,
            Err(_) => false,
        }
    }

    /// Force the sidebar into the **collapsed** state. Idempotent —
    /// already-collapsed is a no-op. Returns `Ok(true)` if a click was
    /// fired (state changed), `Ok(false)` if no action was needed.
    pub fn collapse_sidebar(&self) -> Result<bool, AdapterError> {
        if self.is_sidebar_collapsed() {
            log_info("collapse_sidebar", "already collapsed — no action");
            return Ok(false);
        }
        let (cx, cy) = center(self.regions.sidebar_toggle_when_expanded);
        log_click("collapse_sidebar", "click sidebar toggle", cx, cy);
        crate::human_simulations::move_mouse_single_click(cx, cy).map_err(AdapterError::Input)?;
        std::thread::sleep(std::time::Duration::from_millis(450)); // collapse animation
        log_info("collapse_sidebar", "sidebar collapsed");
        Ok(true)
    }

    /// Click the "New Chat" entry, locating it by template-match in the
    /// **narrow** new-chat region for whichever sidebar state we're in.
    ///
    /// Tight region + tight tolerance (TIGHT) + raised threshold (0.92)
    /// reduces false positives. False positives sent the cursor into
    /// the Terms-of-Use footer in earlier diagnostics.
    pub fn click_new_chat(&self) -> Result<bool, AdapterError> {
        let collapsed = self.is_sidebar_collapsed();
        let (region, templates_) = if collapsed {
            log_info(
                "click_new_chat",
                "sidebar collapsed — using collapsed region",
            );
            (
                self.regions.new_chat_button_when_collapsed,
                templates::new_chat_collapsed_only(),
            )
        } else {
            log_info("click_new_chat", "sidebar expanded — using expanded region");
            (
                self.regions.new_chat_button_when_expanded,
                templates::new_chat_expanded_only(),
            )
        };
        click_template_in_region(region, templates_, Tolerance::TIGHT, 0.92)
    }

    /// Force Direct mode.
    ///
    /// The dropdown is a 3-item list: Battle Mode → Side by Side → Direct.
    /// Direct is always the bottom row.
    ///
    /// Key insight (diagnosed via debug PNGs):
    /// The dropdown is sometimes **already open** when this runs (left open
    /// by a prior step or previous run). Clicking the header when it is
    /// already open just closes it, which is the "open-then-close" bug.
    ///
    /// Strategy:
    ///   1. Snapshot the menu region and measure brightness to detect
    ///      whether the dropdown is currently open.
    ///   2. Only click the header if the dropdown is closed.
    ///   3. Compute the Direct row y from the menu region geometry
    ///      (center of the 3rd of 3 equal rows), then hover + click there.
    ///
    /// Debug PNGs written to the run screenshots folder:
    ///   `debug_dropdown_before.png`, `debug_dropdown_open.png`,
    ///   `debug_dropdown_after.png`
    pub fn force_direct_mode(&self) -> Result<bool, AdapterError> {
        let (hx, hy) = center(self.regions.mode_dropdown);
        let menu = self.regions.mode_menu_expanded;

        // Direct row = centre of 3rd of 3 equal rows in the menu region.
        // The menu content only fills ~90% of the captured region height
        // (the bottom 10% is empty padding). Using the full height overshoots
        // by ~22px and lands below the Direct row.
        let content_h = (menu.y2 - menu.y1) * 5 / 6; // ≈212px of actual rows
        let row_h = content_h / 3; // ≈76px each
        let direct_x = (menu.x1 + menu.x2) / 2;
        let direct_y = menu.y1 + row_h * 2 + row_h / 2; // centre of row 3
        log_info(
            "force_direct_mode",
            &format!(
                "menu region ({},{})→({},{})  row_h={row_h}  \
                      Direct target=({direct_x},{direct_y})",
                menu.x1, menu.y1, menu.x2, menu.y2
            ),
        );

        // ── Step 1: snapshot + decide whether to open the dropdown ──────────
        save_debug_png("dropdown_before", menu);
        let already_open = menu_region_is_open(menu);
        log_info(
            "force_direct_mode",
            &format!("dropdown already open: {already_open}"),
        );

        if !already_open {
            log_click("force_direct_mode", "open dropdown", hx, hy);
            crate::human_simulations::move_mouse_single_click(hx, hy)
                .map_err(AdapterError::Input)?;
            std::thread::sleep(std::time::Duration::from_millis(600));
            save_debug_png("dropdown_open", menu);
        } else {
            save_debug_png("dropdown_open", menu); // same state, for consistency
        }

        // ── Step 2: hover over Direct row, then click ───────────────────────
        log_info(
            "force_direct_mode",
            &format!("hovering over Direct row ({direct_x}, {direct_y})"),
        );
        crate::human_simulations::move_mouse(direct_x, direct_y).map_err(AdapterError::Input)?;
        std::thread::sleep(std::time::Duration::from_millis(150));

        log_click("force_direct_mode", "click Direct row", direct_x, direct_y);
        crate::human_simulations::left_click().map_err(AdapterError::Input)?;
        std::thread::sleep(std::time::Duration::from_millis(400));

        save_debug_png("dropdown_after", menu);
        Ok(true)
    }

    /// Select a specific model in the Direct-mode model picker.
    ///
    /// Arena's model picker is a searchable dropdown: clicking the header
    /// opens a panel with a text input filter.  We type the target model
    /// name and press Enter to select the first match.
    ///
    /// Returns `Ok(true)` once the click sequence fires, `Ok(false)` if
    /// already on the right model (no action needed).
    ///
    /// Debug PNGs written to the run screenshots folder:
    ///   `debug_model_before.png`, `debug_model_open.png`,
    ///   `debug_model_after.png`
    /// Select a specific model in the Direct-mode model picker.
    ///
    /// The model picker is a searchable modal:
    ///   - A text search input sits at the very top of the modal.
    ///   - Typing filters the list live.
    ///   - The first matching result row appears below a tab bar and
    ///     optional group header.
    ///
    /// Sequence:
    ///   1. Open the modal (click header, only if not already open).
    ///   2. Click the search input to guarantee keyboard focus.
    ///   3. Cmd+A to wipe any pre-existing text, then type the target name.
    ///   4. Wait for the list to filter.
    ///   5. Click the first result row.
    ///
    /// Coordinate offsets relative to `model_menu_expanded.y1` (≈165):
    ///   search input centre   ≈ y1 + 17  = 182
    ///   first result centre   ≈ y1 + 140 = 305
    ///   (tab bar + label take up ~120 px between search and first row)
    ///
    /// Returns `Ok(false)` if already on the requested model.
    /// Debug PNGs: `debug_model_before/open/filtered/after.png`.
    pub fn select_model(&self, target: &str) -> Result<bool, AdapterError> {
        let (hx, hy) = center(self.regions.model_dropdown);
        let menu = self.regions.model_menu_expanded;

        // Derive key y positions from the menu region.
        let search_x = (menu.x1 + menu.x2) / 2;
        let search_y = menu.y1 + 17; // search input centre
        let result_x = search_x;
        let result_y = menu.y1 + 140; // first result row centre

        // Skip if already on the right model.
        if let Ok(Some(current)) = self.detect_model() {
            if current.to_lowercase().contains(&target.to_lowercase()) {
                log_info(
                    "select_model",
                    &format!("already on model '{current}' — skipping"),
                );
                return Ok(false);
            }
            log_info(
                "select_model",
                &format!("current model '{current}' ≠ target '{target}'"),
            );
        }

        save_debug_png("model_before", menu);

        // ── Step 1: open the modal if not already open ──────────────────────
        let already_open = menu_region_is_open(menu);
        log_info(
            "select_model",
            &format!("picker already open: {already_open}"),
        );
        if !already_open {
            log_click("select_model", "open model picker", hx, hy);
            crate::human_simulations::move_mouse_single_click(hx, hy)
                .map_err(AdapterError::Input)?;
            std::thread::sleep(std::time::Duration::from_millis(600));
        }
        save_debug_png("model_open", menu);

        // ── Step 2: click the search input to guarantee focus ───────────────
        // Without this, type_text fires into whatever element has focus
        // (often the page body), the typed text goes nowhere, and Enter
        // confirms the default-highlighted model instead.
        log_click("select_model", "focus search input", search_x, search_y);
        crate::human_simulations::move_mouse_single_click(search_x, search_y)
            .map_err(AdapterError::Input)?;
        std::thread::sleep(std::time::Duration::from_millis(150));

        // ── Step 3: clear any pre-existing text, type the target name ───────
        crate::human_simulations::press_key("cmd+a").map_err(AdapterError::Input)?;
        std::thread::sleep(std::time::Duration::from_millis(50));
        log_info("select_model", &format!("typing '{target}'"));
        crate::human_simulations::type_text(target).map_err(AdapterError::Input)?;
        std::thread::sleep(std::time::Duration::from_millis(500)); // wait for live filter
        save_debug_png("model_filtered", menu);

        // ── Step 4: click the first result row ──────────────────────────────
        log_click("select_model", "click first result", result_x, result_y);
        crate::human_simulations::move_mouse_single_click(result_x, result_y)
            .map_err(AdapterError::Input)?;
        std::thread::sleep(std::time::Duration::from_millis(400));

        save_debug_png("model_after", menu);

        if let Ok(mut g) = self.last_model.lock() {
            *g = Some(target.to_string());
        }
        Ok(true)
    }
}

// ── Title-based state helpers ──────────────────────────────────────────────

impl ArenaAdapter {
    /// Returns `true` when Chrome's tab title indicates Direct mode.
    ///
    /// Heuristic: title does NOT contain Battle/Side-by-Side markers.
    /// Direct is Arena's default mode, so:
    ///   - `"Model · Arena"` → Direct (active chat)
    ///   - `"Arena"` (home page) → Direct (no chat yet, default mode)
    ///   - `"Battle: X vs Y · Arena"` → NOT Direct
    ///   - `"Side by Side: ..."` → NOT Direct
    ///
    /// This is intentionally permissive on the home page where the title
    /// is just "Arena" — verifying via the dropdown header pixels would be
    /// stricter, but the title is the only cross-OS truth source we have.
    pub fn verify_direct_mode(&self) -> bool {
        match crate::webui::browser_state::active_chrome_title() {
            Ok(title) => {
                let t = title.trim();
                let lower = t.to_lowercase();
                if lower.contains("battle") {
                    return false;
                }
                if lower.contains("side by side") {
                    return false;
                }
                if lower.contains("side-by-side") {
                    return false;
                }
                if lower.contains("compare") {
                    return false;
                }
                // Anything else (including bare "Arena" home title) — Direct.
                true
            }
            Err(_) => false,
        }
    }

    /// Returns the model name from the Chrome title when in Direct mode.
    ///
    /// Parses `"<model> · Arena"` → `Some("<model>")`. Returns `None`
    /// if the title is missing, malformed, or contains "Battle"/"Side by Side".
    pub fn title_model(&self) -> Option<String> {
        let title = crate::webui::browser_state::active_chrome_title().ok()?;
        let t = title.trim();
        let prefix = t.strip_suffix(" · Arena")?;
        let prefix = prefix.trim();
        if prefix.is_empty() || prefix.contains("Battle") || prefix.contains("Side by Side") {
            return None;
        }
        Some(prefix.to_string())
    }
}

// ── Free helper: capture region, template-match, click match centre ────────

/// Capture `region` as greyscale, find the first matching template via
/// `match_which_gray`, click the centre of the match in absolute screen
/// coordinates.
///
/// Returns `Ok(true)` if a match was found and clicked, `Ok(false)` if
/// no template matched (no click fired). `Err` only on capture or input
/// failure.
fn click_template_in_region(
    region: Region,
    templates: &[GrayMatrix],
    tolerance: Tolerance,
    threshold: f32,
) -> Result<bool, AdapterError> {
    if templates.is_empty() {
        eprintln!(
            "[{}] [click] no templates for region ({},{})→({},{}) — skipping",
            ts(),
            region.x1,
            region.y1,
            region.x2,
            region.y2
        );
        return Ok(false);
    }

    let cap_w = region.x2 - region.x1;
    let cap_h = region.y2 - region.y1;
    let cap = crate::image_matrix::capture::capture_gray_matrix(
        region.x1, region.y1, region.x2, region.y2,
    )
    .map_err(|e| AdapterError::Capture(e.to_string()))?;

    let result = match_which_gray(&cap, templates, tolerance, threshold);
    match result {
        Some((idx, m)) => {
            let needle = &templates[idx];
            let capture_x = region.x1 + m.col as i32 + (needle.width as i32 / 2);
            let capture_y = region.y1 + m.row as i32 + (needle.height as i32 / 2);
            let (click_x, click_y) = capture_to_mouse_point((capture_x, capture_y));
            println!(
                "[{}] [click] region ({},{})→({},{}) [{}×{}] · tmpl={} ({}×{}) · score={:.2} · clicking ({},{})",
                ts(),
                region.x1,
                region.y1,
                region.x2,
                region.y2,
                cap_w,
                cap_h,
                idx,
                needle.width,
                needle.height,
                m.score,
                click_x,
                click_y
            );
            crate::human_simulations::move_mouse_single_click(click_x, click_y)
                .map_err(AdapterError::Input)?;
            Ok(true)
        }
        None => {
            eprintln!(
                "[{}] [click] region ({},{})→({},{}) [{}×{}] · {} templates · NO MATCH (threshold={threshold})",
                ts(),
                region.x1,
                region.y1,
                region.x2,
                region.y2,
                cap_w,
                cap_h,
                templates.len()
            );
            Ok(false)
        }
    }
}

// ── Dropdown open-state detection ───────────────────────────────────────────

/// Returns `true` when the mode dropdown menu appears to be visible.
///
/// Detection: average brightness of the menu region. Arena's background is
/// very dark (~25–35 gray). An open dropdown adds white text and row
/// highlights that raise the mean well above 65.
///
/// Threshold raised from 45 → 65 because real-world measurements showed the
/// chat page background gave mean brightness ≈ 56–57 even when the dropdown
/// was NOT open (false positive). Real open dropdowns measure ≈ 80+, so 65
/// sits safely between the closed-page noise floor and the open dropdown.
fn menu_region_is_open(region: Region) -> bool {
    match crate::image_matrix::capture::capture_gray_matrix(
        region.x1, region.y1, region.x2, region.y2,
    ) {
        Ok(cap) => {
            let sum: u64 = cap.data.iter().map(|&v| v as u64).sum();
            let mean = sum / cap.data.len().max(1) as u64;
            println!("[{}] [menu_region_is_open] mean brightness = {mean}", ts());
            mean > 65
        }
        Err(_) => false,
    }
}

// ── Debug screenshot helper ──────────────────────────────────────────────────

/// Capture `region` as an RGB PNG in the run screenshots folder.
/// Logs the saved path (or the error) — never panics.
fn save_debug_png(label: &str, region: Region) {
    let path = crate::output::writer::screenshot_path(format!("debug_{label}.png"));
    let path_str = path.to_string_lossy();
    match crate::image_matrix::capture::capture_rgb_matrix(
        region.x1, region.y1, region.x2, region.y2,
    ) {
        Ok(cap) => match crate::image_matrix::io::save_rgb_matrix_as_image(&cap, &path_str) {
            Ok(_) => println!(
                "[{}] [debug] screenshot saved → {}  region=({},{})→({},{})",
                ts(),
                path.display(),
                region.x1,
                region.y1,
                region.x2,
                region.y2
            ),
            Err(e) => eprintln!("[{}] [debug] save failed for {}: {e}", ts(), path.display()),
        },
        Err(e) => eprintln!("[{}] [debug] capture failed for {label}: {e}", ts()),
    }
}

fn downloads_dir() -> Result<PathBuf, AdapterError> {
    let home =
        std::env::var("HOME").map_err(|e| AdapterError::Other(format!("HOME not set: {e}")))?;
    Ok(PathBuf::from(home).join("Downloads"))
}

fn newest_download_file() -> Option<PathBuf> {
    let dir = downloads_dir().ok()?;
    newest_file_in_dir(&dir)
}

fn newest_file_in_dir(dir: &Path) -> Option<PathBuf> {
    let mut newest: Option<(SystemTime, PathBuf)> = None;
    for entry in fs::read_dir(dir).ok()? {
        let entry = entry.ok()?;
        let path = entry.path();
        if !path.is_file() || is_partial_download(&path) {
            continue;
        }
        let modified = entry.metadata().ok()?.modified().ok()?;
        match &newest {
            Some((prev, _)) if *prev >= modified => {}
            _ => newest = Some((modified, path)),
        }
    }
    newest.map(|(_, path)| path)
}

fn wait_and_move_download(
    before: Option<&PathBuf>,
    dest_dir: &Path,
) -> Result<PathBuf, AdapterError> {
    let downloads = downloads_dir()?;
    fs::create_dir_all(dest_dir)
        .map_err(|e| AdapterError::Other(format!("create artifact dir: {e}")))?;

    for _ in 0..30 {
        if let Some(path) = newest_file_in_dir(&downloads) {
            let is_new = before.map(|b| b != &path).unwrap_or(true);
            if is_new && !is_partial_download(&path) {
                let dest = unique_dest_path(dest_dir, &path)?;
                fs::rename(&path, &dest).map_err(|e| {
                    AdapterError::Other(format!(
                        "move download {} → {}: {e}",
                        path.display(),
                        dest.display()
                    ))
                })?;
                return Ok(dest);
            }
        }
        std::thread::sleep(Duration::from_millis(1000));
    }

    Err(AdapterError::Timeout {
        what: "download file in ~/Downloads".to_string(),
        after_ms: 30_000,
    })
}

fn unique_dest_path(dest_dir: &Path, src: &Path) -> Result<PathBuf, AdapterError> {
    let name = src.file_name().ok_or_else(|| {
        AdapterError::Other(format!("download has no file name: {}", src.display()))
    })?;
    let candidate = dest_dir.join(name);
    if !candidate.exists() {
        return Ok(candidate);
    }
    let stem = src
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("artifact");
    let ext = src.extension().and_then(|s| s.to_str()).unwrap_or("");
    for i in 1..1000 {
        let file_name = if ext.is_empty() {
            format!("{stem}_{i}")
        } else {
            format!("{stem}_{i}.{ext}")
        };
        let candidate = dest_dir.join(file_name);
        if !candidate.exists() {
            return Ok(candidate);
        }
    }
    Err(AdapterError::Other(
        "could not allocate unique artifact path".to_string(),
    ))
}

fn is_partial_download(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|s| s.to_str()),
        Some("crdownload") | Some("download") | Some("tmp")
    )
}

/// Capture a region and return its pixel variance (used to detect screen changes).
fn capture_region_variance(region: Region) -> f64 {
    let cap = match crate::image_matrix::capture::capture_gray_matrix(
        region.x1, region.y1, region.x2, region.y2,
    ) {
        Ok(m) => m,
        Err(_) => return 0.0,
    };
    let n = cap.data.len();
    if n == 0 {
        return 0.0;
    }
    let mean = cap.data.iter().map(|&v| v as f64).sum::<f64>() / n as f64;
    cap.data
        .iter()
        .map(|&v| (v as f64 - mean).powi(2))
        .sum::<f64>()
        / n as f64
}

/// After the file dialog closes, poll the Arena send button for up to `timeout`
/// until the upload finishes (send button becomes active again).
fn arena_wait_for_upload_complete(
    send_region: Region,
    timeout: std::time::Duration,
) -> Result<(), AdapterError> {
    // Allow the dialog to dismiss and the upload to initiate.
    std::thread::sleep(std::time::Duration::from_millis(1000));

    let deadline = std::time::Instant::now() + timeout;
    log_info(
        "upload",
        &format!(
            "polling send button for upload completion (timeout={}s)…",
            timeout.as_secs()
        ),
    );

    while std::time::Instant::now() < deadline {
        // Arena's send button is detected via the send_button template.
        use crate::webui::arena::templates;
        let tpls = templates::send_button();
        if !tpls.is_empty() {
            if let Ok(cap) = crate::image_matrix::capture::capture_gray_matrix(
                send_region.x1,
                send_region.y1,
                send_region.x2,
                send_region.y2,
            ) {
                let found = tpls.iter().any(|t| {
                    crate::image_matrix::compare::match_any_gray(
                        &cap,
                        std::slice::from_ref(t),
                        crate::image_matrix::types::Tolerance::NORMAL,
                        0.60,
                    )
                });
                if found {
                    log_info("upload", "send button active — upload complete");
                    return Ok(());
                }
            }
        } else {
            // No template: fall back to variance change (send button area becomes
            // non-uniform when the upload-indicator clears).
            let var = capture_region_variance(send_region);
            if var > 200.0 {
                log_info(
                    "upload",
                    "send region variance high — assuming upload complete",
                );
                return Ok(());
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }

    Err(AdapterError::Timeout {
        what: "file upload did not complete within timeout".into(),
        after_ms: timeout.as_millis() as u64,
    })
}

/// Scan `input_box` for an existing file-attachment chip and click its close
/// button if found. Returns `true` when an attachment was removed.
fn arena_clear_existing_attachment(input_box: Region) -> Result<bool, AdapterError> {
    let box_w = input_box.x2 - input_box.x1;
    let box_h = input_box.y2 - input_box.y1;
    let chip_region = Region {
        x1: input_box.x1,
        y1: input_box.y1 + box_h * 55 / 100,
        x2: input_box.x1 + box_w * 45 / 100,
        y2: input_box.y2 - box_h * 12 / 100,
    };

    let var = capture_region_variance(chip_region);
    if var < 150.0 {
        return Ok(false);
    }
    log_info(
        "upload",
        &format!("possible attachment chip (variance={var:.1})"),
    );

    // Template match if available.
    use crate::webui::arena::templates;
    let close_tpls = templates::attach_close_button();
    if !close_tpls.is_empty() {
        if let Ok(Some((cx, cy))) = find_attach_button_in_region(chip_region) {
            log_info("upload", &format!("chip close-button at ({cx}, {cy})"));
            crate::human_simulations::move_mouse_single_click(cx, cy)
                .map_err(|e| AdapterError::Input(format!("chip close click: {e}")))?;
            std::thread::sleep(std::time::Duration::from_millis(600));
            return Ok(true);
        }
    }

    // Pixel heuristic fallback (same logic as DeepSeek adapter).
    let cap = crate::image_matrix::capture::capture_gray_matrix(
        chip_region.x1,
        chip_region.y1,
        chip_region.x2,
        chip_region.y2,
    )
    .map_err(|e| AdapterError::Capture(e.to_string()))?;

    if let Some((lx, ly)) = arena_find_close_button_pixel(&cap) {
        let cx = chip_region.x1 + lx;
        let cy = chip_region.y1 + ly;
        log_info(
            "upload",
            &format!("chip close-button pixel heuristic at ({cx}, {cy})"),
        );
        crate::human_simulations::move_mouse_single_click(cx, cy)
            .map_err(|e| AdapterError::Input(format!("chip close click: {e}")))?;
        std::thread::sleep(std::time::Duration::from_millis(600));
        return Ok(true);
    }

    log_info(
        "upload",
        "chip region has content but close button not located — skipping clear",
    );
    Ok(false)
}

fn arena_find_close_button_pixel(
    cap: &crate::image_matrix::types::GrayMatrix,
) -> Option<(i32, i32)> {
    const BRIGHT: u8 = 185;
    const WIN: i32 = 28;
    const MIN_PX: usize = 6;
    const MAX_PX: usize = 150;
    const MAX_SPAN: i32 = 24;
    let w = cap.width as i32;
    let h = cap.height as i32;
    let mut wy = 0i32;
    while wy <= h - WIN {
        let mut wx = 0i32;
        while wx <= w - WIN {
            let mut bright = 0usize;
            let mut min_x = WIN;
            let mut min_y = WIN;
            let mut max_x = 0i32;
            let mut max_y = 0i32;
            for dy in 0..WIN {
                for dx in 0..WIN {
                    let px = (wx + dx) as usize;
                    let py = (wy + dy) as usize;
                    if cap.get(px, py) > BRIGHT {
                        bright += 1;
                        min_x = min_x.min(dx);
                        min_y = min_y.min(dy);
                        max_x = max_x.max(dx);
                        max_y = max_y.max(dy);
                    }
                }
            }
            if (MIN_PX..=MAX_PX).contains(&bright) {
                let sx = max_x - min_x + 1;
                let sy = max_y - min_y + 1;
                if sx <= MAX_SPAN && sy <= MAX_SPAN && sx > 0 && sy > 0 {
                    let aspect = sx * 10 / sy;
                    if (4..=25).contains(&aspect) {
                        return Some((wx + (min_x + max_x) / 2, wy + (min_y + max_y) / 2));
                    }
                }
            }
            wx += 4;
        }
        wy += 4;
    }
    None
}

/// Matrix-scan `region` for the paperclip/attach icon using the Arena template.
/// Returns the center of the best match, or `None` if not found.
///
/// Arena doesn't have a dedicated attach template yet — falls back to a
/// centre-of-region click. Capture `assets/macos/arena.ai/templates/attach_button_dark.png`
/// with `cargo run --bin capture_template` to enable proper matching.
fn find_attach_button_in_region(region: Region) -> Result<Option<(i32, i32)>, String> {
    use crate::webui::arena::templates;
    let tpls = templates::attach_button();
    if tpls.is_empty() {
        // No template yet — fall back to region centre.
        let cx = (region.x1 + region.x2) / 2;
        let cy = (region.y1 + region.y2) / 2;
        return Ok(Some((cx, cy)));
    }
    let cap = crate::image_matrix::capture::capture_gray_matrix(
        region.x1, region.y1, region.x2, region.y2,
    )
    .map_err(|e| format!("capture: {e}"))?;
    for tpl in tpls {
        let hits = crate::image_matrix::compare::find_all_matches_gray(
            &cap,
            tpl,
            crate::image_matrix::types::Tolerance::NORMAL,
            0.70,
        );
        if let Some(hit) = hits.into_iter().next() {
            let cx = region.x1 + hit.col as i32 + tpl.width as i32 / 2;
            let cy = region.y1 + hit.row as i32 + tpl.height as i32 / 2;
            return Ok(Some((cx, cy)));
        }
    }
    Ok(None)
}

/// Navigate the macOS file-open dialog to `path` using Cmd+Shift+G.
///
/// The dialog must already be open before calling this.
/// Puts the path in the clipboard, opens "Go to Folder", pastes, and confirms.
fn osascript_open_file_dialog(path: &str) -> Result<(), String> {
    let mut clip = arboard::Clipboard::new().map_err(|e| format!("clipboard init: {e}"))?;
    clip.set_text(path)
        .map_err(|e| format!("clipboard set: {e}"))?;

    let script = r#"
tell application "System Events"
    delay 1.0
    keystroke "g" using {shift down, command down}
    delay 0.8
    keystroke "a" using command down
    delay 0.2
    keystroke "v" using command down
    delay 0.5
    key code 36
    delay 0.6
    key code 36
end tell
"#;

    let out = std::process::Command::new("osascript")
        .args(["-e", script])
        .output()
        .map_err(|e| format!("osascript spawn: {e}"))?;

    if !out.status.success() {
        return Err(format!(
            "osascript non-zero: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(())
}
