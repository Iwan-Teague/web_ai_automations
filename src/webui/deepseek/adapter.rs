//! `WebUiAdapter` impl for DeepSeek.

use std::sync::Mutex;

use crate::image_matrix::compare::{find_all_matches_gray, match_any_gray, match_which_gray};
use crate::image_matrix::types::{GrayMatrix, Tolerance};
use crate::session::DeepSeekModelMode;
use crate::webui::adapter::{AdapterError, Region, WebUiAdapter, WebUiState};
use crate::webui::browser_state;
use crate::webui::deepseek::regions::{Regions, center};
use crate::webui::deepseek::templates;

fn ts() -> String {
    chrono::Local::now().format("%H:%M:%S%.3f").to_string()
}

fn log_info(ctx: &str, msg: &str) {
    println!("[{}] [deepseek:{ctx}] {msg}", ts());
}

#[derive(Debug)]
enum UrlClass {
    Home,
    InChat,
    OffSite,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ToggleState {
    On,
    Off,
    Unknown,
}

fn classify_url() -> UrlClass {
    match browser_state::active_chrome_url() {
        Ok(url) => {
            let lower = url.to_lowercase();
            if !lower.contains("chat.deepseek.com") {
                return UrlClass::OffSite;
            }
            if lower.contains("/chat/") || lower.contains("/a/chat/") {
                UrlClass::InChat
            } else {
                UrlClass::Home
            }
        }
        Err(_) => UrlClass::Unknown,
    }
}

pub struct DeepSeekAdapter {
    regions: Regions,
    last_conv_var: Mutex<Option<f64>>,
    saw_variance_change: Mutex<bool>,
}

#[derive(Debug, Clone, Copy)]
struct TemplateHit {
    bounds: Region,
    score: f32,
}

impl DeepSeekAdapter {
    pub fn new() -> Self {
        // Load every template's binary matrix sidecar now so no PNG
        // decode happens mid-poll. After this returns, every template
        // accessor is a pure memory read.
        templates::preload_all();
        Self {
            regions: Regions::for_primary_screen(),
            last_conv_var: Mutex::new(None),
            saw_variance_change: Mutex::new(false),
        }
    }

    pub fn regions(&self) -> &Regions {
        &self.regions
    }

    /// Remove all existing file-attachment chips from the fresh-chat input box.
    /// Must be called before `select_model` — DeepSeek disables mode switching
    /// while a file is attached. Loops until no chip is detected (max 8 passes).
    /// Detect whether a file-attachment chip is currently visible above the
    /// fresh-chat input box. Detection-only (no clicks). Used by setup as a
    /// post-reload sanity check — if a chip survives the reload, setup
    /// errors out rather than proceeding with a polluted composer.
    pub fn attachment_present(&self) -> bool {
        let icon_tpls = templates::attach_chip_icon();
        if icon_tpls.is_empty() {
            return false;
        }
        let input_box = self.regions.input_box_fresh;
        let scan = Region {
            x1: (input_box.x1 - 60).max(0),
            y1: (input_box.y1 - 120).max(0),
            x2: input_box.x2,
            y2: input_box.y1 + 5,
        };
        matches!(find_chip_icon(scan, icon_tpls), Ok(Some(_)))
    }

    pub fn select_model(&self, model: DeepSeekModelMode) -> Result<(), AdapterError> {
        let (current, hit) = self.detect_model_hit()?;
        log_info(
            "setup",
            &format!("model matrix: current={current:?} hit={:?}", hit.bounds),
        );
        if current == Some(model) {
            return Ok(());
        }
        if current.is_none() {
            return Err(AdapterError::NotFound {
                what: "DeepSeek model selector selected state".to_string(),
            });
        }

        let (x, y) = match model {
            DeepSeekModelMode::Instant => (
                hit.bounds.x1 + hit.bounds.width() / 4,
                (hit.bounds.y1 + hit.bounds.y2) / 2,
            ),
            DeepSeekModelMode::Expert => (
                hit.bounds.x1 + hit.bounds.width() * 3 / 4,
                (hit.bounds.y1 + hit.bounds.y2) / 2,
            ),
        };
        log_info("setup", &format!("select model {model}: click ({x},{y})"));
        crate::human_simulations::move_mouse_single_click(x, y).map_err(AdapterError::Input)?;
        self.move_mouse_off_controls()?;
        std::thread::sleep(std::time::Duration::from_millis(1000));
        let (verified, _) = self.detect_model_hit()?;
        if verified == Some(model) {
            Ok(())
        } else {
            Err(AdapterError::Input(format!(
                "DeepSeek model click did not verify selected {model}"
            )))
        }
    }

    fn detect_model_hit(&self) -> Result<(Option<DeepSeekModelMode>, TemplateHit), AdapterError> {
        let expert = template_hit_in_region(
            self.regions.model_scan,
            templates::model_expert_selected(),
            Tolerance::NORMAL,
            0.55,
        )?;
        let instant = template_hit_in_region(
            self.regions.model_scan,
            templates::model_instant_selected(),
            Tolerance::NORMAL,
            0.55,
        )?;
        match (instant, expert) {
            (Some(i), Some(e)) if e.score >= i.score => Ok((Some(DeepSeekModelMode::Expert), e)),
            (Some(i), Some(_)) => Ok((Some(DeepSeekModelMode::Instant), i)),
            (None, Some(e)) => Ok((Some(DeepSeekModelMode::Expert), e)),
            (Some(i), None) => Ok((Some(DeepSeekModelMode::Instant), i)),
            (None, None) => Err(AdapterError::NotFound {
                what: "DeepSeek model selector selected state by matrix".to_string(),
            }),
        }
    }

    pub fn set_deep_thinking(&self, enabled: bool) -> Result<(), AdapterError> {
        self.set_toggle(
            "deep thinking",
            self.regions.deep_thinking_scan,
            templates::deep_thinking_on(),
            templates::deep_thinking_off(),
            enabled,
        )
    }

    pub fn set_smart_search(&self, enabled: bool) -> Result<(), AdapterError> {
        self.set_toggle(
            "smart search",
            self.regions.smart_search_scan,
            templates::smart_search_on(),
            templates::smart_search_off(),
            enabled,
        )
    }

    fn set_toggle(
        &self,
        label: &str,
        region: Region,
        on_templates: &[GrayMatrix],
        off_templates: &[GrayMatrix],
        enabled: bool,
    ) -> Result<(), AdapterError> {
        // Smart-search on/off templates score within ~0.01 of each other —
        // below the noise floor — so the on-vs-off classification flips
        // randomly. We skip the toggle when the call is ambiguous and
        // also tolerate ambiguous post-click verification: a click that
        // landed correctly will not always shift the dominant template
        // by enough to flip the comparator.
        const AMBIGUOUS_DELTA: f32 = 0.025;

        let (state, pre_delta) =
            self.detect_toggle_state_with_delta(label, region, on_templates, off_templates)?;
        if (enabled && state == ToggleState::On) || (!enabled && state == ToggleState::Off) {
            return Ok(());
        }
        if state == ToggleState::Unknown {
            return Err(AdapterError::NotFound {
                what: format!("DeepSeek {label} toggle state"),
            });
        }
        if pre_delta < AMBIGUOUS_DELTA {
            log_info(
                "setup",
                &format!(
                    "{label} delta {pre_delta:.3} < {AMBIGUOUS_DELTA} — assuming target state {enabled}, no click"
                ),
            );
            return Ok(());
        }
        let hit = self.find_toggle_hit(region, on_templates, off_templates)?;
        let (x, y) = center(hit.bounds);
        log_info(
            "setup",
            &format!("toggle {label} → {enabled}: click ({x},{y})"),
        );
        crate::human_simulations::move_mouse_single_click(x, y).map_err(AdapterError::Input)?;
        self.move_mouse_off_controls()?;
        std::thread::sleep(std::time::Duration::from_millis(350));
        let (verified, post_delta) =
            self.detect_toggle_state_with_delta(label, region, on_templates, off_templates)?;
        if (enabled && verified == ToggleState::On) || (!enabled && verified == ToggleState::Off) {
            Ok(())
        } else if post_delta < AMBIGUOUS_DELTA {
            log_info(
                "setup",
                &format!(
                    "{label} post-click delta {post_delta:.3} < {AMBIGUOUS_DELTA} — accepting target state {enabled}"
                ),
            );
            Ok(())
        } else {
            Err(AdapterError::Input(format!(
                "DeepSeek {label} toggle click did not verify target state {enabled}"
            )))
        }
    }

    fn detect_toggle_state_with_delta(
        &self,
        label: &str,
        region: Region,
        on_templates: &[GrayMatrix],
        off_templates: &[GrayMatrix],
    ) -> Result<(ToggleState, f32), AdapterError> {
        let on = template_hit_in_region(region, on_templates, Tolerance::NORMAL, 0.55)?;
        let off = template_hit_in_region(region, off_templates, Tolerance::NORMAL, 0.55)?;
        let on_score = on.as_ref().map(|h| h.score).unwrap_or(0.0);
        let off_score = off.as_ref().map(|h| h.score).unwrap_or(0.0);
        let delta = (on_score - off_score).abs();
        // Persist both halves for drift analysis. Labels match the toggle
        // name plus the variant being tested so we can isolate which side
        // is degrading.
        let on_name = format!("toggle_{}_on", label.replace(' ', "_"));
        let off_name = format!("toggle_{}_off", label.replace(' ', "_"));
        crate::image_matrix::telemetry::record(
            &on_name,
            on.as_ref().map(|h| h.score),
            (region.x1, region.y1, region.x2, region.y2),
        );
        crate::image_matrix::telemetry::record(
            &off_name,
            off.as_ref().map(|h| h.score),
            (region.x1, region.y1, region.x2, region.y2),
        );
        log_info(
            "setup",
            &format!("{label} matrix scores: on={on_score:.3} off={off_score:.3} Δ={delta:.3}"),
        );
        let state = match (on, off) {
            (Some(_), Some(_)) if on_score >= off_score => ToggleState::On,
            (Some(_), Some(_)) => ToggleState::Off,
            (Some(_), None) => ToggleState::On,
            (None, Some(_)) => ToggleState::Off,
            (None, None) => ToggleState::Unknown,
        };
        Ok((state, delta))
    }

    fn find_toggle_hit(
        &self,
        region: Region,
        on_templates: &[GrayMatrix],
        off_templates: &[GrayMatrix],
    ) -> Result<TemplateHit, AdapterError> {
        let on = template_hit_in_region(region, on_templates, Tolerance::NORMAL, 0.55)?;
        let off = template_hit_in_region(region, off_templates, Tolerance::NORMAL, 0.55)?;
        match (on, off) {
            (Some(a), Some(b)) if a.score >= b.score => Ok(a),
            (Some(_), Some(b)) => Ok(b),
            (Some(a), None) => Ok(a),
            (None, Some(b)) => Ok(b),
            (None, None) => Err(AdapterError::NotFound {
                what: "DeepSeek toggle by matrix".to_string(),
            }),
        }
    }

    fn send_scan_regions_for_current_input(&self) -> [Region; 2] {
        match classify_url() {
            UrlClass::InChat => [self.regions.send_scan_inchat, self.regions.send_scan_fresh],
            _ => [self.regions.send_scan_fresh, self.regions.send_scan_inchat],
        }
    }

    fn current_input_regions(&self) -> (Region, Region) {
        match classify_url() {
            UrlClass::InChat => (
                self.regions.input_box_inchat,
                self.regions.input_focus_inchat,
            ),
            _ => (self.regions.input_box_fresh, self.regions.input_focus_fresh),
        }
    }

    fn move_mouse_off_controls(&self) -> Result<(), AdapterError> {
        let x = (self.regions.screen_width - 80).max(20);
        let y = 190.min(self.regions.screen_height.saturating_sub(20));
        crate::human_simulations::move_mouse(x, y).map_err(AdapterError::Input)
    }

    /// Ensure DeepSeek's left chat-history sidebar is OPEN.
    ///
    /// Two states matter:
    /// - **Open** — the "+ New chat" pill (with text) renders inside the
    ///   sidebar header at y≈217. The existing `new_chat` template matches.
    /// - **Closed** — only a compact pill `[panel_toggle][+]` shows at the
    ///   very top-left of the viewport (y≈70-100). The new_chat template
    ///   does not match here. Clicking the LEFT icon (square panel toggle)
    ///   expands the sidebar.
    ///
    /// Strategy: try the new_chat template in its expanded position first;
    /// if found, the sidebar is already open. Otherwise scan the compact-
    /// pill region for the panel-toggle template and click its centre.
    /// Returns `true` when the sidebar is in (or has been moved to) the
    /// open state.
    pub fn ensure_left_sidebar_open(&self) -> bool {
        // Already open? new_chat template only renders with "+ New chat"
        // text inside the open sidebar.
        match template_center_in_region(
            self.regions.new_chat_button,
            templates::new_chat(),
            Tolerance::NORMAL,
            0.60,
        ) {
            Ok(Some(_)) => {
                eprintln!("[deepseek] left sidebar already open");
                return true;
            }
            Ok(None) => {}
            Err(e) => eprintln!("[deepseek] sidebar open-check scan failed: {e:?}"),
        }

        let scan = self.regions.panel_toggle_scan;
        debug_capture_region(
            scan.x1,
            scan.y1,
            scan.x2 - scan.x1,
            scan.y2 - scan.y1,
            "panel_toggle_scan",
        );

        let tpls = templates::panel_toggle();
        if tpls.is_empty() {
            eprintln!(
                "[deepseek] panel_toggle template not loaded — save the cropped square \
                 panel icon at assets/macos/deepseek/templates/panel_toggle_dark.png"
            );
            return false;
        }
        match match_with_telemetry("panel_toggle", scan, tpls, Tolerance::NORMAL, 0.65) {
            Ok(Some(hit)) => {
                let (x, y) = (
                    (hit.bounds.x1 + hit.bounds.x2) / 2,
                    (hit.bounds.y1 + hit.bounds.y2) / 2,
                );
                eprintln!(
                    "[deepseek] sidebar closed (score={:.3}) — clicking panel toggle at ({x},{y})",
                    hit.score
                );
                if let Err(e) = crate::human_simulations::move_mouse_single_click(x, y) {
                    eprintln!("[deepseek] panel-toggle click failed: {e}");
                    return false;
                }
                let _ = self.move_mouse_off_controls();
                std::thread::sleep(std::time::Duration::from_millis(800));
                debug_capture_region(
                    scan.x1,
                    scan.y1,
                    scan.x2 - scan.x1,
                    scan.y2 - scan.y1,
                    "panel_toggle_after",
                );
                true
            }
            Ok(None) => {
                eprintln!(
                    "[deepseek] panel toggle not matched in compact-pill region (threshold 0.65)"
                );
                false
            }
            Err(e) => {
                eprintln!("[deepseek] panel toggle scan failed: {e:?}");
                false
            }
        }
    }

    /// Find the rightmost uploaded chip in the composer. Used to verify or
    /// repair the most recently added file without disturbing earlier
    /// successful uploads. Returns the screen-space centre of the blue
    /// document icon for that chip, or `None` if no chips are visible.
    /// Public wrapper for the one-off `capture_chip_x` helper bin.
    pub fn find_rightmost_chip_pub(&self) -> Option<(i32, i32)> {
        self.find_rightmost_chip()
    }

    /// True when any chip in the strip shows the "Server busy" reload-arrow
    /// icon. Threshold a touch lower than chip-icon detection because the
    /// arrow's white outline against the chip background sometimes scores
    /// 0.78–0.82 even on a clean match.
    /// True when the wide blue "New chat" button is rendered in the
    /// send-button area. Returns the click coordinates if found.
    /// DeepSeek shows this in place of the send-arrow when a fresh
    /// file is attached to an existing chat — clicking it spins up a
    /// new chat with this file as the only attachment, dropping the
    /// rest of the conversation.
    fn detect_submit_new_chat_button(&self) -> Option<(i32, i32)> {
        let hit = self.detect_submit_new_chat_hit()?;
        let cx = (hit.x1 + hit.x2) / 2;
        let cy = (hit.y1 + hit.y2) / 2;
        Some((cx, cy))
    }

    fn detect_submit_new_chat_hit(&self) -> Option<Region> {
        let widened = self.submit_new_chat_scan_region();
        let tpls = templates::submit_new_chat();
        if tpls.is_empty() {
            return None;
        }
        match template_hit_in_region(widened, tpls, Tolerance::NORMAL, 0.75) {
            Ok(Some(hit)) => {
                let cx = (hit.bounds.x1 + hit.bounds.x2) / 2;
                let cy = (hit.bounds.y1 + hit.bounds.y2) / 2;
                eprintln!(
                    "[upload] 'New chat' submit button detected (score={:.3}) at ({cx},{cy}) — DeepSeek wants to start a fresh chat",
                    hit.score
                );
                Some(hit.bounds)
            }
            _ => None,
        }
    }

    fn submit_new_chat_scan_region(&self) -> Region {
        let region = match classify_url() {
            UrlClass::InChat => self.regions.send_scan_inchat,
            _ => self.regions.send_scan_fresh,
        };
        // The submit-time "New chat" pill is much wider than the send
        // arrow and pushes the paperclip left. Scan the whole right side
        // of the composer, not just the arrow slot.
        Region {
            x1: (region.x2 - 280).max(0),
            y1: (region.y1 - 30).max(0),
            x2: (region.x2 + 20).min(self.regions.screen_width),
            y2: (region.y2 + 15).min(self.regions.screen_height),
        }
    }

    fn attach_scan_for_followup_layout(
        &self,
        base: Region,
        new_chat_hit: Option<Region>,
    ) -> Region {
        let Some(new_chat) = new_chat_hit else {
            return base;
        };
        Region {
            // New Chat sits where send normally lives and shifts the
            // paperclip left. Scan left of the button only so we don't
            // match the blue button/text instead of the paperclip.
            x1: (new_chat.x1 - 100).max(0),
            y1: base.y1,
            x2: (new_chat.x1 - 8).min(base.x2),
            y2: base.y2,
        }
    }

    fn detect_server_busy_chip(&self) -> bool {
        let region = match classify_url() {
            UrlClass::InChat => self.regions.chip_strip_inchat,
            _ => self.regions.chip_strip_fresh,
        };
        let tpls = templates::server_busy();
        if tpls.is_empty() {
            eprintln!("[busy-scan] server_busy template not loaded");
            return false;
        }
        let cap = match crate::image_matrix::capture::capture_gray_matrix(
            region.x1, region.y1, region.x2, region.y2,
        ) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("[busy-scan] capture failed: {e}");
                return false;
            }
        };
        for tpl in tpls {
            let hits = crate::image_matrix::compare::find_all_matches_gray(
                &cap,
                tpl,
                Tolerance::NORMAL,
                0.78,
            );
            // Same noise cap as the chip-icon scan — uniform-dark space
            // pseudo-matches dark-heavy templates at modest thresholds.
            if !hits.is_empty() && hits.len() <= 12 {
                let h = &hits[0];
                crate::image_matrix::telemetry::record(
                    "server_busy",
                    Some(h.score),
                    (region.x1, region.y1, region.x2, region.y2),
                );
                eprintln!(
                    "[busy-scan] {} hit(s); first at ({},{}) score={:.3}",
                    hits.len(),
                    region.x1 + h.col as i32,
                    region.y1 + h.row as i32,
                    h.score
                );
                return true;
            }
        }
        false
    }

    fn find_rightmost_chip(&self) -> Option<(i32, i32)> {
        let region = match classify_url() {
            UrlClass::InChat => self.regions.chip_strip_inchat,
            _ => self.regions.chip_strip_fresh,
        };
        // Always save what we scanned — when polls report "no chip yet"
        // the user can inspect this to confirm the y range and threshold.
        debug_capture_region(
            region.x1,
            region.y1,
            region.x2 - region.x1,
            region.y2 - region.y1,
            "chip_strip_scan",
        );
        let icon_tpls = templates::attach_chip_icon();
        if icon_tpls.is_empty() {
            eprintln!("[chip-scan] attach_chip_icon template not loaded");
            return None;
        }
        let cap = match crate::image_matrix::capture::capture_gray_matrix(
            region.x1, region.y1, region.x2, region.y2,
        ) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("[chip-scan] capture failed: {e}");
                return None;
            }
        };
        // Threshold raised back to 0.85 — the chip icon template has a lot
        // of dark background pixels, and a too-loose threshold pseudo-
        // matches uniform dark space (a recent run logged 144 hits at
        // 0.706 against an empty region).
        const CHIP_MATCH_THRESHOLD: f32 = 0.85;
        // Sanity cap: if matrix matching returns absurdly many hits, we're
        // matching noise (the template is mostly dark; uniform-dark areas
        // satisfy a generous threshold). Reject and return None.
        const MAX_PLAUSIBLE_HITS: usize = 12;

        let mut best: Option<(i32, i32, f32)> = None;
        let mut total_hits = 0usize;
        let mut best_score = 0.0f32;
        for tpl in icon_tpls {
            let hits = crate::image_matrix::compare::find_all_matches_gray(
                &cap,
                tpl,
                Tolerance::NORMAL,
                CHIP_MATCH_THRESHOLD,
            );
            total_hits += hits.len();
            for h in hits {
                if h.score > best_score {
                    best_score = h.score;
                }
                let cx = region.x1 + h.col as i32 + tpl.width as i32 / 2;
                let cy = region.y1 + h.row as i32 + tpl.height as i32 / 2;
                match best {
                    Some((bx, _, _)) if cx <= bx => {}
                    _ => best = Some((cx, cy, h.score)),
                }
            }
        }
        if total_hits > MAX_PLAUSIBLE_HITS {
            eprintln!(
                "[chip-scan] {} hits — exceeds plausible cap ({MAX_PLAUSIBLE_HITS}); treating as noise",
                total_hits
            );
            return None;
        }
        let region_tuple = (region.x1, region.y1, region.x2, region.y2);
        if let Some((cx, cy, sc)) = best {
            crate::image_matrix::telemetry::record("chip_icon_rightmost", Some(sc), region_tuple);
            eprintln!(
                "[chip-scan] {} hit(s); rightmost at ({cx},{cy}) score={sc:.3}",
                total_hits
            );
            return Some((cx, cy));
        }
        crate::image_matrix::telemetry::record("chip_icon_rightmost", None, region_tuple);
        eprintln!(
            "[chip-scan] 0 hits in region ({},{})-({},{}) (best partial score: {:.3})",
            region.x1, region.y1, region.x2, region.y2, best_score
        );
        None
    }

    /// Move mouse onto the rightmost chip and try matrix-finding the white
    /// × close button. The × is `display:none` until `:hover`, and a
    /// tooltip can pop over the button depending on cursor x. Tries a few
    /// hover positions on the icon-side (where no tooltip renders) before
    /// giving up.
    ///
    /// Returns `true` when the × was clicked.
    fn delete_rightmost_chip_via_hover(&self) -> bool {
        let (icon_cx, icon_cy) = match self.find_rightmost_chip() {
            Some(p) => p,
            None => {
                eprintln!("[upload] repair: no rightmost chip detected");
                return false;
            }
        };
        let close_tpls = templates::chip_close_x();
        if close_tpls.is_empty() {
            eprintln!(
                "[upload] repair: chip_close_x template not loaded — save the cropped \
                 white × on dark circle at \
                 assets/macos/deepseek/templates/chip_close_x_dark.png"
            );
            return false;
        }

        // Hover positions: centred-on-icon first (no tooltip there), then a
        // couple of fallbacks. The chip is roughly 240 wide × 60 tall in the
        // observed layout; × sits in the top-right corner.
        let hover_targets: [(i32, i32); 3] = [
            (icon_cx, icon_cy),           // on the icon
            (icon_cx - 5, icon_cy - 10),  // top-left-ish
            (icon_cx + 60, icon_cy - 18), // mid-chip but high
        ];

        // Search box around the chip's top-right corner where the × renders.
        let search_region = Region {
            x1: icon_cx + 100,
            y1: icon_cy - 35,
            x2: icon_cx + 260,
            y2: icon_cy + 5,
        };

        for (hx, hy) in hover_targets {
            if let Err(e) = crate::human_simulations::move_mouse(hx, hy) {
                eprintln!("[upload] repair: hover move failed: {e}");
                continue;
            }
            std::thread::sleep(std::time::Duration::from_millis(450));
            debug_capture_region(
                search_region.x1,
                search_region.y1,
                search_region.x2 - search_region.x1,
                search_region.y2 - search_region.y1,
                "chip_close_search",
            );
            // 0.70 is forgiving — a tooltip overlap can clip the × glyph
            // and drop the score, but the white-on-dark contrast is still
            // distinctive enough for a partial match to score above 0.70.
            match match_with_telemetry(
                "chip_close_x",
                search_region,
                close_tpls,
                Tolerance::NORMAL,
                0.70,
            ) {
                Ok(Some(hit)) => {
                    let (cx, cy) = (
                        (hit.bounds.x1 + hit.bounds.x2) / 2,
                        (hit.bounds.y1 + hit.bounds.y2) / 2,
                    );
                    eprintln!(
                        "[upload] repair: found chip × (score={:.3}) — clicking ({cx},{cy})",
                        hit.score
                    );
                    if let Err(e) = crate::human_simulations::move_mouse_single_click(cx, cy) {
                        eprintln!("[upload] repair: × click failed: {e}");
                        return false;
                    }
                    let _ = self.move_mouse_off_controls();
                    std::thread::sleep(std::time::Duration::from_millis(700));
                    return true;
                }
                Ok(None) => {
                    eprintln!("[upload] repair: × not found at hover ({hx},{hy})");
                }
                Err(e) => eprintln!("[upload] repair: × scan error: {e:?}"),
            }
        }
        eprintln!("[upload] repair: exhausted hover positions without finding ×");
        false
    }

    /// Detect and close DeepSeek's document-preview side panel.
    ///
    /// Clicking an uploaded chip opens a right-side panel that re-flows the
    /// composer and shifts every button location. Until the panel is closed
    /// every coordinate-based scan is wrong. Called from `detect_state` so
    /// it runs at every poll, plus once during setup.
    ///
    /// Returns `true` when a panel was detected and a click was issued.
    pub fn dismiss_doc_panel_if_open(&self) -> bool {
        let scan = self.regions.doc_panel_close_scan;
        // Always save what we scanned — lets the user crop the × out of a
        // real run capture if the template asset is missing.
        debug_capture_region(
            scan.x1,
            scan.y1,
            scan.x2 - scan.x1,
            scan.y2 - scan.y1,
            "doc_panel_close_scan",
        );

        let tpls = templates::doc_panel_close();
        if tpls.is_empty() {
            eprintln!(
                "[deepseek] doc_panel_close template not loaded — save the cropped × at \
                 assets/macos/deepseek/templates/doc_panel_close_dark.png"
            );
            return false;
        }
        // Use template_hit_in_region so we can log the score even on miss.
        // Higher threshold (0.85) — the tight × template scores ~0.93 when
        // genuinely present and stays under 0.85 against empty dark regions.
        match match_with_telemetry("doc_panel_close", scan, tpls, Tolerance::NORMAL, 0.85) {
            Ok(Some(hit)) => {
                let (x, y) = (
                    (hit.bounds.x1 + hit.bounds.x2) / 2,
                    (hit.bounds.y1 + hit.bounds.y2) / 2,
                );
                eprintln!(
                    "[deepseek] doc-preview panel detected (score={:.3}) — clicking × at ({x},{y})",
                    hit.score
                );
                if let Err(e) = crate::human_simulations::move_mouse_single_click(x, y) {
                    eprintln!("[deepseek] doc-panel close click failed: {e}");
                    return false;
                }
                let _ = self.move_mouse_off_controls();
                std::thread::sleep(std::time::Duration::from_millis(500));
                debug_capture_region(
                    scan.x1,
                    scan.y1,
                    scan.x2 - scan.x1,
                    scan.y2 - scan.y1,
                    "doc_panel_close_after",
                );
                true
            }
            Ok(None) => {
                eprintln!("[deepseek] doc-panel × not matched in scan region (threshold 0.65)");
                false
            }
            Err(e) => {
                eprintln!("[deepseek] doc-panel scan failed: {e:?}");
                false
            }
        }
    }
}

impl Default for DeepSeekAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl WebUiAdapter for DeepSeekAdapter {
    fn name(&self) -> &'static str {
        "deepseek"
    }

    /// Raise Chrome to the foreground before each detection poll. Without
    /// this, Terminal or other windows can drift on top of the DeepSeek
    /// tab and every region capture reads the wrong pixels (we caught
    /// this during upload polling — Terminal log lines were being
    /// scanned for chip icons, returning either 0 hits or noisy pseudo-
    /// matches in the hundreds).
    fn prepare_for_detection(&self) -> Result<(), AdapterError> {
        let _ = crate::webui::browser_state::focus_app_by_name("Chrome");
        std::thread::sleep(std::time::Duration::from_millis(150));
        Ok(())
    }

    fn detect_state(&self) -> Result<WebUiState, AdapterError> {
        let url = classify_url();
        if matches!(url, UrlClass::OffSite) {
            return Ok(WebUiState::Unknown);
        }

        // The doc-preview side panel reshuffles every coordinate. Close it
        // before any region-based classification runs.
        self.dismiss_doc_panel_if_open();

        if matches!(url, UrlClass::Home) && self.detect_model_hit().is_ok() {
            return Ok(WebUiState::Ready);
        }

        for r in self.send_scan_regions_for_current_input() {
            if region_matches(r, templates::stop_square(), Tolerance::NORMAL, 0.70)?
                || find_white_stop_square(r)?
            {
                return Ok(WebUiState::Generating);
            }
        }

        for r in self.send_scan_regions_for_current_input() {
            if find_send_button_in_region(r)?.is_some() {
                return Ok(match url {
                    UrlClass::InChat => WebUiState::Complete,
                    _ => WebUiState::Ready,
                });
            }
        }

        if self.detect_submit_new_chat_button().is_some() {
            return Ok(WebUiState::Ready);
        }

        if matches!(url, UrlClass::InChat)
            && (region_matches(
                self.regions.copy_scan,
                templates::copy_button(),
                Tolerance::NORMAL,
                0.82,
            )? || region_matches(
                self.regions.response_actions,
                templates::response_actions(),
                Tolerance::NORMAL,
                0.82,
            )?)
        {
            return Ok(WebUiState::Complete);
        }

        if matches!(url, UrlClass::InChat) {
            let cur = region_variance(self.regions.conversation);
            let mut last = self.last_conv_var.lock().unwrap();
            let prev = *last;
            *last = Some(cur);
            drop(last);
            if let Some(prev) = prev {
                if (cur - prev).abs() > 50.0 {
                    *self.saw_variance_change.lock().unwrap() = true;
                    return Ok(WebUiState::Generating);
                }
            }
        }

        Ok(WebUiState::Unknown)
    }

    fn detect_model(&self) -> Result<Option<String>, AdapterError> {
        match self.detect_model_hit() {
            Ok((Some(DeepSeekModelMode::Expert), _)) => Ok(Some("Expert".to_string())),
            Ok((Some(DeepSeekModelMode::Instant), _)) => Ok(Some("Instant".to_string())),
            _ => Ok(None),
        }
    }

    fn submit_prompt(&self, text: &str) -> Result<(), AdapterError> {
        let (input_box, input_focus) = self.current_input_regions();
        let in_chat = matches!(classify_url(), UrlClass::InChat);
        let pre_url = browser_state::active_chrome_url().unwrap_or_default();

        let mut clip = arboard::Clipboard::new()
            .map_err(|e| AdapterError::Clipboard(format!("arboard new: {e}")))?;
        let saved_clip = clip.get_text().ok();
        clip.set_text(text.to_string())
            .map_err(|e| AdapterError::Clipboard(format!("clipboard set: {e}")))?;
        std::thread::sleep(std::time::Duration::from_millis(80));

        let (ix, iy) = center(input_focus);
        crate::human_simulations::move_mouse_single_click(ix, iy).map_err(AdapterError::Input)?;
        std::thread::sleep(std::time::Duration::from_millis(100));
        crate::human_simulations::press_key("cmd+a").map_err(AdapterError::Input)?;
        std::thread::sleep(std::time::Duration::from_millis(80));
        crate::human_simulations::press_key("backspace").map_err(AdapterError::Input)?;
        std::thread::sleep(std::time::Duration::from_millis(120));
        crate::human_simulations::press_key("cmd+v").map_err(AdapterError::Input)?;
        std::thread::sleep(std::time::Duration::from_millis(400));

        let pre_input_var = region_variance(input_focus);
        let _ = input_box;
        let (send_pos, submit_kind) = if let Some(pos) = self.detect_submit_new_chat_button() {
            (pos, "new-chat")
        } else {
            let pos = self
                .send_scan_regions_for_current_input()
                .into_iter()
                .find_map(|r| find_send_button_in_region(r).ok().flatten())
                .ok_or_else(|| AdapterError::NotFound {
                    what: format!(
                        "DeepSeek send button by matrix in {} prompt regions",
                        if in_chat { "follow-up" } else { "first" }
                    ),
                })?;
            (pos, "send")
        };
        log_info(
            "submit",
            &format!(
                "click {submit_kind} ({},{}) in_chat={in_chat}",
                send_pos.0, send_pos.1
            ),
        );
        crate::human_simulations::move_mouse_single_click(send_pos.0, send_pos.1)
            .map_err(AdapterError::Input)?;
        self.move_mouse_off_controls()?;
        std::thread::sleep(std::time::Duration::from_millis(900));

        let post_url = browser_state::active_chrome_url().unwrap_or_default();
        let post_input_var = region_variance(input_focus);
        let url_changed = !pre_url.is_empty() && post_url != pre_url && post_url.contains("/chat/");
        let input_cleared = pre_input_var > 20.0 && post_input_var < pre_input_var * 0.65;
        let stop_visible = self
            .send_scan_regions_for_current_input()
            .into_iter()
            .any(|r| {
                region_matches(r, templates::stop_square(), Tolerance::NORMAL, 0.80)
                    .unwrap_or(false)
            });

        if let Some(saved) = saved_clip {
            let _ = clip.set_text(saved);
        }

        if !(url_changed || input_cleared || stop_visible) {
            return Err(AdapterError::Input(format!(
                "DeepSeek send could not be confirmed: url_changed={url_changed} input_var {pre_input_var:.1}->{post_input_var:.1} stop_visible={stop_visible}"
            )));
        }

        *self.last_conv_var.lock().unwrap() = None;
        *self.saw_variance_change.lock().unwrap() = false;
        Ok(())
    }

    fn copy_response(&self) -> Result<String, AdapterError> {
        let pre_clip = crate::human_simulations::read_clipboard().unwrap_or_default();
        let candidates = find_template_centers(
            self.regions.copy_scan,
            templates::copy_button(),
            Tolerance::NORMAL,
            0.80,
        )?;
        log_info("copy", &format!("copy candidates: {candidates:?}"));
        let Some((mx, my)) = candidates
            .iter()
            .copied()
            .filter(|(x, _)| (500..=520).contains(x))
            .max_by_key(|&(_, y)| y)
        else {
            return Err(AdapterError::NotFound {
                what: "DeepSeek latest response copy button".to_string(),
            });
        };
        let (x, y) = (mx, my);
        log_info(
            "copy",
            &format!("copy matched ({mx},{my}); click ({x},{y})"),
        );
        log_info("copy", &format!("click copy ({x},{y})"));
        crate::human_simulations::move_mouse_single_click(x, y).map_err(AdapterError::Input)?;
        std::thread::sleep(std::time::Duration::from_millis(500));
        let post_clip =
            crate::human_simulations::read_clipboard().map_err(AdapterError::Clipboard)?;
        if post_clip != pre_clip && !post_clip.trim().is_empty() {
            Ok(post_clip)
        } else {
            Err(AdapterError::Clipboard(
                "DeepSeek copy button clicked but clipboard did not change".to_string(),
            ))
        }
    }

    fn start_new_chat(&self) -> Result<(), AdapterError> {
        // Debug: save the scan region so we can inspect if the template fails.
        let r = self.regions.new_chat_button;
        debug_capture_region(r.x1, r.y1, r.x2 - r.x1, r.y2 - r.y1, "new_chat_scan");

        let click = template_center_in_region(
            self.regions.new_chat_button,
            templates::new_chat(),
            Tolerance::NORMAL,
            0.60,
        )?
        .ok_or_else(|| AdapterError::NotFound {
            what: "DeepSeek New chat button by matrix".to_string(),
        })?;
        log_info("new_chat", &format!("click ({},{})", click.0, click.1));
        crate::human_simulations::move_mouse_single_click(click.0, click.1)
            .map_err(AdapterError::Input)?;
        self.move_mouse_off_controls()?;
        std::thread::sleep(std::time::Duration::from_millis(900));
        Ok(())
    }

    fn dismiss_modal(&self) -> Result<bool, AdapterError> {
        crate::human_simulations::press_key("escape").map_err(AdapterError::Input)?;
        Ok(true)
    }

    /// Rotate to a fresh chat mid-run, re-applying mode + toggles. Mirrors
    /// the in-page portion of `session_setup::run` (skips Chrome focus and
    /// fullscreen since we're already on the page). Reload + new-chat
    /// flushes any uploaded-but-unsent files server-side.
    fn rotate_for_long_session(
        &self,
        cfg: &crate::session::SessionConfig,
    ) -> Result<bool, AdapterError> {
        log_info("rotate", "starting fresh chat for long-running session");

        // Press Escape to clear any open dialogs / focus traps.
        let _ = crate::human_simulations::press_key("escape");
        std::thread::sleep(std::time::Duration::from_millis(300));

        // Right doc-preview panel and left sidebar must be in known states
        // before any region scan. Right panel must be closed; left sidebar
        // must be open.
        self.dismiss_doc_panel_if_open();
        self.ensure_left_sidebar_open();
        std::thread::sleep(std::time::Duration::from_millis(400));

        // Reload to home + new chat — flushes uploaded-but-unsent files
        // server-side. Same idempotent step session_setup uses on first run.
        crate::webui::browser_state::navigate_current_tab(
            crate::webui::deepseek::DEEPSEEK_HOME_URL,
        )
        .map_err(|e| AdapterError::Other(format!("rotate navigate: {e}")))?;
        std::thread::sleep(std::time::Duration::from_millis(2200));
        self.dismiss_doc_panel_if_open();
        self.ensure_left_sidebar_open();
        std::thread::sleep(std::time::Duration::from_millis(400));
        self.start_new_chat()?;
        std::thread::sleep(std::time::Duration::from_millis(900));

        if self.attachment_present() {
            return Err(AdapterError::Other(
                "rotate: chip still present after reload".into(),
            ));
        }

        self.select_model(cfg.deepseek_model)?;
        self.set_deep_thinking(cfg.deepseek_deep_thinking)?;
        self.set_smart_search(cfg.deepseek_smart_search)?;
        log_info("rotate", "fresh chat ready");
        Ok(true)
    }

    fn upload_file(&self, path: &std::path::Path) -> Result<bool, AdapterError> {
        self.upload_file_internal(path, /*preserve_chat=*/ false)
    }

    fn try_upload_preserving_chat(&self, path: &std::path::Path) -> Result<bool, AdapterError> {
        self.upload_file_internal(path, /*preserve_chat=*/ true)
    }
}

// Implementation detail — pulled out of the trait impl so we can share
// the upload pipeline between the reload-on-failure (`upload_file`) and
// preserve-chat (`try_upload_preserving_chat`) entry points.
impl DeepSeekAdapter {
    fn upload_file_internal(
        &self,
        path: &std::path::Path,
        preserve_chat: bool,
    ) -> Result<bool, AdapterError> {
        use std::time::Duration;

        let abs = path
            .canonicalize()
            .map_err(|e| AdapterError::Other(format!("upload_file: cannot resolve path: {e}")))?;
        let path_str = abs.to_string_lossy().into_owned();

        // Focus Chrome and park the cursor off the right-rail controls
        // so the previous upload's mouse click isn't covering the
        // paperclip icon when we scan for it.
        let _ = crate::webui::browser_state::focus_app_by_name("Chrome");
        std::thread::sleep(Duration::from_millis(300));
        let _ = self.move_mouse_off_controls();
        std::thread::sleep(Duration::from_millis(200));

        let url_class = classify_url();
        let (base_attach_scan, send_scan, _input_box) = match url_class {
            UrlClass::InChat => (
                self.regions.attach_scan_inchat,
                self.regions.send_scan_inchat,
                self.regions.input_box_inchat,
            ),
            _ => (
                self.regions.attach_scan_fresh,
                self.regions.send_scan_fresh,
                self.regions.input_box_fresh,
            ),
        };
        let submit_new_chat_hit = self.detect_submit_new_chat_hit();
        let attach_scan =
            self.attach_scan_for_followup_layout(base_attach_scan, submit_new_chat_hit);
        eprintln!(
            "[upload] url={:?} submit_new_chat_visible={} attach_scan=({},{})-({},{}) send_scan=({},{})-({},{})",
            url_class,
            submit_new_chat_hit.is_some(),
            attach_scan.x1,
            attach_scan.y1,
            attach_scan.x2,
            attach_scan.y2,
            send_scan.x1,
            send_scan.y1,
            send_scan.x2,
            send_scan.y2,
        );
        // Wide debug capture of the entire bottom-of-page region. Lets us
        // see what the actual post-prompt layout looks like vs what our
        // regions assume — if the UI shifted between the captured-region
        // calibration and the live page, the captures show it.
        let bottom_region = match url_class {
            UrlClass::InChat => Region {
                x1: 480,
                y1: 720,
                x2: 1290,
                y2: 970,
            },
            _ => Region {
                x1: 480,
                y1: 380,
                x2: 1290,
                y2: 800,
            },
        };
        debug_capture_region(
            bottom_region.x1,
            bottom_region.y1,
            bottom_region.x2 - bottom_region.x1,
            bottom_region.y2 - bottom_region.y1,
            "upload_layout_pre_attach",
        );

        // 1 repair retry = 2 total attempts per file (initial + 1 retry).
        // Tightened from 2 retries (3 attempts) — when DeepSeek's context
        // is full and persistently rejecting files, repeated retries are
        // wasted time. Faster to defer and let the runner move on.
        const MAX_REPAIR_RETRIES: usize = 1;
        for attempt in 0..=MAX_REPAIR_RETRIES {
            eprintln!(
                "[upload] attaching {path_str} (attempt {} of {}, preserve_chat={preserve_chat})",
                attempt + 1,
                MAX_REPAIR_RETRIES + 1,
            );
            self.click_attach_and_pick_file(attach_scan, &path_str, submit_new_chat_hit)?;

            match self.poll_upload_with_rejection_detection(send_scan) {
                UploadPollOutcome::Complete => {
                    eprintln!("[upload] file attached and ready");
                    return Ok(true);
                }
                UploadPollOutcome::ServerBusy => {
                    eprintln!(
                        "[upload] server busy — reloading page; runner will restart upload batch"
                    );
                    let _ = crate::webui::browser_state::navigate_current_tab(
                        crate::webui::deepseek::DEEPSEEK_HOME_URL,
                    );
                    std::thread::sleep(Duration::from_millis(2200));
                    self.dismiss_doc_panel_if_open();
                    self.ensure_left_sidebar_open();
                    let _ = self.start_new_chat();
                    std::thread::sleep(Duration::from_millis(900));
                    return Err(AdapterError::Other(SERVER_BUSY_RESTART_SENTINEL.into()));
                }
                UploadPollOutcome::Rejected if attempt < MAX_REPAIR_RETRIES => {
                    eprintln!(
                        "[upload] REJECTION CONFIRMED — chip up + send dull after warm-up. \
                         Beginning chip-delete repair."
                    );
                    if !self.delete_rightmost_chip_via_hover() {
                        return Err(AdapterError::Other(
                            "upload rejected and × delete failed — cannot recover".into(),
                        ));
                    }
                    eprintln!("[upload] repair: × clicked, retrying upload of {path_str}");
                    std::thread::sleep(Duration::from_millis(500));
                    continue;
                }
                UploadPollOutcome::Rejected => {
                    // Same file rejected on all 3 attempts. Two recovery
                    // paths depending on whether the caller is happy to
                    // lose the active conversation.
                    if preserve_chat {
                        // Mid-conversation retry of a previously-deferred
                        // file. Don't reload — just delete the rejected
                        // chip locally so the chip strip is clean, then
                        // surface a non-sentinel error so the runner
                        // leaves the file deferred and continues.
                        eprintln!(
                            "[upload] {} still rejected on retry ({} attempts) — deleting chip and leaving deferred (chat preserved)",
                            path_str,
                            MAX_REPAIR_RETRIES + 1,
                        );
                        let _ = self.delete_rightmost_chip_via_hover();
                        return Err(AdapterError::Other("deferred upload still failing".into()));
                    }
                    eprintln!(
                        "[upload] {} rejected after {} attempts — deleting chip, deferring, continuing batch (no reload)",
                        path_str,
                        MAX_REPAIR_RETRIES + 1,
                    );
                    let _ = self.delete_rightmost_chip_via_hover();
                    return Err(AdapterError::Other(FILE_REJECTED_DEFER_SENTINEL.into()));
                }
                UploadPollOutcome::Timeout => {
                    return Err(AdapterError::Timeout {
                        what: "upload polling timed out without chip or active send button".into(),
                        after_ms: 150_000,
                    });
                }
            }
        }
        Err(AdapterError::Other(
            "upload_file: exceeded repair retry budget".into(),
        ))
    }
}

impl DeepSeekAdapter {
    /// Poll for upload completion with in-flight rejection detection.
    ///
    /// Each poll checks two states:
    /// - Send button blue → `Complete`.
    /// - Rightmost chip shows the blue-doc icon (upload "succeeded" on the
    ///   chip widget) but send button is still dull → likely rejection.
    ///   Confirmed if the same state holds across `REJECTION_STREAK`
    ///   consecutive polls (≈30 s) — a normal upload can flash through
    ///   "chip + dull send" briefly while the server finishes processing.
    fn poll_upload_with_rejection_detection(&self, send_scan: Region) -> UploadPollOutcome {
        // Logic per user spec, evaluated every 10 s:
        //   1. Is the send button DULL (not blue)?
        //      No  → upload complete, return.
        //      Yes → step 2.
        //   2. Is the rightmost chip still uploading (no blue-doc icon
        //      detected yet)?
        //      Yes → wait 10 s and re-poll.
        //      No  → upload was rejected by DeepSeek; return Rejected so
        //            the caller deletes the chip and re-uploads.
        //
        // Critical: WAIT BEFORE THE FIRST POLL. A normal upload briefly
        // shows chip=BLUE-DOC + send=DULL while the server finishes
        // processing. Polling instantly would false-flag every successful
        // upload as rejected. The 10 s warm-up matches the inter-poll
        // gap so we always evaluate against settled state.
        const INITIAL_WAIT_MS: u64 = 15_000;
        const MAX_POLLS: u32 = 15;
        const POLL_INTERVAL_MS: u64 = 15_000;

        // Make sure Chrome is foremost — Terminal occasionally floats over
        // the composer when the file dialog closes, and the chip strip
        // capture would then read Terminal pixels (0 chip hits, score
        // 0.000 against an empty pseudo-region).
        let _ = crate::webui::browser_state::focus_app_by_name("Chrome");
        std::thread::sleep(std::time::Duration::from_millis(300));

        eprintln!(
            "[upload] warm-up sleep {}s before first poll (lets normal uploads finish)…",
            INITIAL_WAIT_MS / 1000,
        );
        std::thread::sleep(std::time::Duration::from_millis(INITIAL_WAIT_MS));

        eprintln!(
            "[upload] polling (every {}s, up to {} times = {}s) — send-button + chip checks each tick",
            POLL_INTERVAL_MS / 1000,
            MAX_POLLS,
            (POLL_INTERVAL_MS / 1000) * MAX_POLLS as u64,
        );

        for attempt in 1..=MAX_POLLS {
            // Check 1 (BEFORE everything else): is any chip showing the
            // "Server busy" reload-arrow icon? This state can persist
            // indefinitely without the chip ever transitioning to blue-
            // doc, so we'd miss it if we only checked the send button +
            // blue-doc combo.
            if self.detect_server_busy_chip() {
                eprintln!(
                    "[upload] poll {}/{}  chip=SERVER-BUSY — page reload required",
                    attempt, MAX_POLLS
                );
                return UploadPollOutcome::ServerBusy;
            }

            // Check 2: did DeepSeek replace Send with a wide New Chat
            // pill? This is also an upload-complete state, but the caller
            // must click New Chat and restart the batch into that fresh
            // chat.
            if self.detect_submit_new_chat_button().is_some() {
                eprintln!(
                    "[upload] poll {}/{}  submit=NEW_CHAT — upload complete but chat rollover required",
                    attempt, MAX_POLLS
                );
                return UploadPollOutcome::Complete;
            }

            // Check 3: is the send button blue (active)?
            if let Ok(Some(_)) = find_blue_send_button(send_scan) {
                eprintln!(
                    "[upload] poll {}/{}  send=ACTIVE — upload complete",
                    attempt, MAX_POLLS
                );
                return UploadPollOutcome::Complete;
            }

            // Send is dull. Check 4: is the rightmost chip showing the
            // blue-doc icon (= upload finished on the chip widget but
            // server rejected the file)?
            let chip_visible = self.find_rightmost_chip().is_some();
            if chip_visible {
                eprintln!(
                    "[upload] poll {}/{}  send=DULL chip=BLUE-DOC — REJECTED",
                    attempt, MAX_POLLS
                );
                return UploadPollOutcome::Rejected;
            }
            eprintln!(
                "[upload] poll {}/{}  send=DULL chip=PENDING — still uploading",
                attempt, MAX_POLLS
            );

            if attempt < MAX_POLLS {
                std::thread::sleep(std::time::Duration::from_millis(POLL_INTERVAL_MS));
            }
        }
        UploadPollOutcome::Timeout
    }

    /// Click the paperclip and drive the macOS file-open dialog to `path`.
    /// Pulled out so `upload_file`'s self-healing loop can repeat it.
    fn click_attach_and_pick_file(
        &self,
        attach_scan: Region,
        path_str: &str,
        new_chat_hit: Option<Region>,
    ) -> Result<(), AdapterError> {
        use std::time::Duration;

        close_chrome_file_dialog_if_open();

        let (ax, ay) = if let Some(new_chat) = new_chat_hit {
            let ax = new_chat.x1 - 25;
            let ay = (new_chat.y1 + new_chat.y2) / 2;
            eprintln!(
                "[upload] New Chat layout: using geometric paperclip target from button bounds ({},{})-({},{})",
                new_chat.x1, new_chat.y1, new_chat.x2, new_chat.y2
            );
            (ax, ay)
        } else {
            find_attach_button(attach_scan, templates::attach_button())
                .map_err(|e| AdapterError::Input(format!("attach scan failed: {e}")))?
                .ok_or_else(|| AdapterError::NotFound {
                    what: "paperclip/attach button not found in scan region".into(),
                })?
        };
        const ATTACH_CLICK_DY: i32 = 8;
        let click_x = ax;
        let click_y = ay + ATTACH_CLICK_DY;
        eprintln!("[upload] found attach button at ({ax}, {ay}); clicking ({click_x}, {click_y})");

        let dialog_check_region = Region {
            x1: 300,
            y1: 200,
            x2: 1200,
            y2: 800,
        };
        let pre_var = region_variance(dialog_check_region);

        crate::human_simulations::move_mouse_single_click(click_x, click_y)
            .map_err(|e| AdapterError::Input(format!("attach click failed: {e}")))?;
        std::thread::sleep(Duration::from_millis(1200));

        let post_var = region_variance(dialog_check_region);
        if (post_var - pre_var).abs() < 100.0 {
            return Err(AdapterError::NotFound {
                what: "file dialog did not appear after clicking attach button".into(),
            });
        }

        osascript_open_file_dialog(path_str)
            .map_err(|e| AdapterError::Input(format!("file dialog navigation failed: {e}")))?;

        // After the macOS file-open dialog dismisses, focus does NOT
        // always return to Chrome — sometimes Terminal (where the bot
        // logs run) ends up frontmost. Subsequent screen captures would
        // then read Terminal pixels instead of the DeepSeek tab and
        // every region scan returns 0 hits. Re-focus Chrome explicitly.
        let _ = crate::webui::browser_state::focus_app_by_name("Chrome");
        std::thread::sleep(Duration::from_millis(500));
        Ok(())
    }
}

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
    let mean = cap.data.iter().map(|&v| v as f64).sum::<f64>() / n as f64;
    cap.data
        .iter()
        .map(|&v| (v as f64 - mean).powi(2))
        .sum::<f64>()
        / n as f64
}

fn region_matches(
    region: Region,
    templates: &[GrayMatrix],
    tolerance: Tolerance,
    threshold: f32,
) -> Result<bool, AdapterError> {
    if templates.is_empty() {
        return Ok(false);
    }
    let cap = crate::image_matrix::capture::capture_gray_matrix(
        region.x1, region.y1, region.x2, region.y2,
    )
    .map_err(|e| AdapterError::Capture(e.to_string()))?;
    Ok(match_any_gray(&cap, templates, tolerance, threshold))
}

fn find_send_button_in_region(region: Region) -> Result<Option<(i32, i32)>, AdapterError> {
    let hits = template_hits_in_region(region, templates::send_arrow(), Tolerance::NORMAL, 0.55)?;
    let mut right_rail_hits: Vec<TemplateHit> = hits
        .into_iter()
        .filter(|hit| hit.bounds.x1 >= 1220)
        .collect();
    right_rail_hits.sort_by(|a, b| {
        let ay = (a.bounds.y1 + a.bounds.y2) / 2;
        let by = (b.bounds.y1 + b.bounds.y2) / 2;
        by.cmp(&ay).then_with(|| b.score.total_cmp(&a.score))
    });
    if let Some(hit) = right_rail_hits.into_iter().next() {
        log_info(
            "submit",
            &format!(
                "send arrow template lowest score={:.3} bounds={:?}",
                hit.score, hit.bounds
            ),
        );
        return Ok(Some(center(hit.bounds)));
    }
    find_blue_send_button(region)
}

fn find_blue_send_button(region: Region) -> Result<Option<(i32, i32)>, AdapterError> {
    let cap = crate::image_matrix::capture::capture_rgb_matrix(
        region.x1, region.y1, region.x2, region.y2,
    )
    .map_err(|e| AdapterError::Capture(e.to_string()))?;
    let w = cap.width as i32;
    let h = cap.height as i32;
    let mut min_x = w;
    let mut min_y = h;
    let mut max_x = 0;
    let mut max_y = 0;
    let mut count = 0usize;

    for y in 0..h {
        for x in 0..w {
            let [r, g, b] = cap.get(x as usize, y as usize);
            let blue_button = b >= 150 && g >= 90 && r <= 120 && b > r + 45;
            if blue_button {
                count += 1;
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x);
                max_y = max_y.max(y);
            }
        }
    }

    if count < 250 {
        return Ok(None);
    }
    let bw = max_x - min_x + 1;
    let bh = max_y - min_y + 1;
    if !(28..=70).contains(&bw) || !(28..=70).contains(&bh) {
        return Ok(None);
    }
    let cx = region.x1 + (min_x + max_x) / 2;
    let cy = region.y1 + (min_y + max_y) / 2;
    log_info(
        "submit",
        &format!(
            "send blue-button matrix pixels={count} bounds=({min_x},{min_y})→({max_x},{max_y}) click=({cx},{cy})"
        ),
    );
    Ok(Some((cx, cy)))
}

fn find_white_stop_square(region: Region) -> Result<bool, AdapterError> {
    let cap = crate::image_matrix::capture::capture_gray_matrix(
        region.x1, region.y1, region.x2, region.y2,
    )
    .map_err(|e| AdapterError::Capture(e.to_string()))?;
    let w = cap.width as i32;
    let h = cap.height as i32;
    if w < 12 || h < 12 {
        return Ok(false);
    }

    for y in 0..=(h - 12) {
        for x in 0..=(w - 12) {
            let mut bright = 0usize;
            for yy in y..(y + 12) {
                for xx in x..(x + 12) {
                    if cap.get(xx as usize, yy as usize) >= 210 {
                        bright += 1;
                    }
                }
            }
            if bright >= 95 {
                log_info(
                    "detect_state",
                    &format!(
                        "white stop square in send rail at ({},{})",
                        region.x1 + x,
                        region.y1 + y
                    ),
                );
                return Ok(true);
            }
        }
    }
    Ok(false)
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
            (
                region.x1 + m.col as i32 + needle.width as i32 / 2,
                region.y1 + m.row as i32 + needle.height as i32 / 2,
            )
        }),
    )
}

/// Same as `template_hit_in_region`, but records the match outcome
/// (template name, score, region) to the per-run JSONL telemetry stream.
/// Use this at every meaningful match call site so we get a per-template
/// time series of scores — a template that used to match at 0.93 and
/// starts matching at 0.78 across multiple polls is the early signal of
/// a UI change that will eventually break detection entirely.
fn match_with_telemetry(
    name: &'static str,
    region: Region,
    templates: &[GrayMatrix],
    tolerance: Tolerance,
    threshold: f32,
) -> Result<Option<TemplateHit>, AdapterError> {
    let result = template_hit_in_region(region, templates, tolerance, threshold)?;
    crate::image_matrix::telemetry::record(
        name,
        result.as_ref().map(|h| h.score),
        (region.x1, region.y1, region.x2, region.y2),
    );
    Ok(result)
}

fn template_hit_in_region(
    region: Region,
    templates: &[GrayMatrix],
    tolerance: Tolerance,
    threshold: f32,
) -> Result<Option<TemplateHit>, AdapterError> {
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
            TemplateHit {
                bounds: Region {
                    x1: region.x1 + m.col as i32,
                    y1: region.y1 + m.row as i32,
                    x2: region.x1 + m.col as i32 + needle.width as i32,
                    y2: region.y1 + m.row as i32 + needle.height as i32,
                },
                score: m.score,
            }
        }),
    )
}

fn template_hits_in_region(
    region: Region,
    templates: &[GrayMatrix],
    tolerance: Tolerance,
    threshold: f32,
) -> Result<Vec<TemplateHit>, AdapterError> {
    if templates.is_empty() {
        return Ok(Vec::new());
    }
    let cap = crate::image_matrix::capture::capture_gray_matrix(
        region.x1, region.y1, region.x2, region.y2,
    )
    .map_err(|e| AdapterError::Capture(e.to_string()))?;
    let mut out = Vec::new();
    for template in templates {
        for m in find_all_matches_gray(&cap, template, tolerance, threshold) {
            out.push(TemplateHit {
                bounds: Region {
                    x1: region.x1 + m.col as i32,
                    y1: region.y1 + m.row as i32,
                    x2: region.x1 + m.col as i32 + template.width as i32,
                    y2: region.y1 + m.row as i32 + template.height as i32,
                },
                score: m.score,
            });
        }
    }
    Ok(out)
}

fn find_template_centers(
    region: Region,
    templates: &[GrayMatrix],
    tolerance: Tolerance,
    threshold: f32,
) -> Result<Vec<(i32, i32)>, AdapterError> {
    if templates.is_empty() {
        return Ok(Vec::new());
    }
    let cap = crate::image_matrix::capture::capture_gray_matrix(
        region.x1, region.y1, region.x2, region.y2,
    )
    .map_err(|e| AdapterError::Capture(e.to_string()))?;
    let mut out = Vec::new();
    for template in templates {
        for hit in find_all_matches_gray(&cap, template, tolerance, threshold) {
            out.push((
                region.x1 + hit.col as i32 + template.width as i32 / 2,
                region.y1 + hit.row as i32 + template.height as i32 / 2,
            ));
        }
    }
    out.sort_by_key(|&(_, y)| y);
    Ok(out)
}

/// After the macOS file-open dialog closes, poll the send button region for up
/// to `timeout` until the upload finishes and the send button turns blue again.
/// Outcome of polling for upload completion.
#[derive(Debug)]
enum UploadPollOutcome {
    /// Send button went blue — file accepted, ready to submit.
    Complete,
    /// Chip shows the circular reload-arrow with "Server busy" subtitle.
    /// DeepSeek's backend is overloaded; the only reliable recovery is
    /// to reload the page and restart the entire upload batch.
    ServerBusy,
    /// Rightmost chip is in the blue-doc (uploaded) state but the send
    /// button refuses to activate. DeepSeek rejected the file ("can only
    /// read X% of the files…"); the repair path needs to delete the chip
    /// and re-upload just this one file.
    Rejected,
    /// Polled to the cap without ever seeing a chip OR an active send
    /// button — likely the file dialog never delivered the file.
    Timeout,
}

/// Sentinel emitted via `AdapterError::Other(SERVER_BUSY_RESTART_SENTINEL)`
/// when an upload aborts because DeepSeek hit a "Server busy" state and
/// the page has been reloaded. The runner pattern-matches this to know
/// the entire upload batch needs to start over.
pub const SERVER_BUSY_RESTART_SENTINEL: &str = "deepseek_server_busy_reload_restart_batch";

/// Distinct sentinel for the simpler "this one file got rejected too many
/// times — chip already deleted, just defer" case. Lets the runner skip
/// the file and continue the batch without reloading the page (which
/// would wipe every successfully-uploaded chip so far).
pub const FILE_REJECTED_DEFER_SENTINEL: &str = "deepseek_file_rejected_defer_continue";

/// Emitted when DeepSeek replaces the send-arrow with a wide blue
/// "New chat" button after we attach a file. The adapter has already
/// clicked it (we're now in a fresh chat with the just-attached file
/// as the only attachment); the runner needs to re-upload every other
/// project-map file so the new chat has full context.
pub const NEW_CHAT_TRIGGERED_SENTINEL: &str = "deepseek_new_chat_triggered_reupload_all";

/// Capture a region of the screen to a PNG in the run output folder for debugging.
/// `label` becomes part of the filename. Silently ignores errors.
fn debug_capture_region(x: i32, y: i32, w: i32, h: i32, label: &str) {
    let x1 = x.max(0);
    let y1 = y.max(0);
    let x2 = x1 + w;
    let y2 = y1 + h;
    let cap = match crate::image_matrix::capture::capture_rgb_matrix(x1, y1, x2, y2) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("[dbg-cap] capture failed ({label}): {e}");
            return;
        }
    };
    // Build path: outputs/debug/<timestamp>_<label>.png
    let ts = chrono::Local::now().format("%H%M%S%.3f").to_string();
    let dir = std::path::Path::new("outputs/debug");
    let _ = std::fs::create_dir_all(dir);
    let path = dir.join(format!("{ts}_{label}.png"));
    let flat: Vec<u8> = cap
        .data
        .into_iter()
        .flat_map(|[r, g, b]| [r, g, b])
        .collect();
    match image::RgbImage::from_raw(cap.width as u32, cap.height as u32, flat) {
        Some(img) => {
            if let Err(e) = img.save(&path) {
                eprintln!("[dbg-cap] save failed ({label}): {e}");
            } else {
                eprintln!("[dbg-cap] saved → {}", path.display());
            }
        }
        None => eprintln!("[dbg-cap] from_raw failed ({label})"),
    }
}

/// Matrix-scan `region` for the file-chip blue document icon.
/// Uses a higher threshold (0.80) than the paperclip button scan to reduce
/// false positives — the chip icon template has mostly dark background pixels
/// that match many regions at lower thresholds.
fn find_chip_icon(
    region: Region,
    templates: &[crate::image_matrix::types::GrayMatrix],
) -> Result<Option<(i32, i32)>, String> {
    if templates.is_empty() {
        return Ok(None);
    }
    let cap = crate::image_matrix::capture::capture_gray_matrix(
        region.x1, region.y1, region.x2, region.y2,
    )
    .map_err(|e| format!("capture failed: {e}"))?;

    for template in templates {
        let hits = crate::image_matrix::compare::find_all_matches_gray(
            &cap,
            template,
            crate::image_matrix::types::Tolerance::NORMAL,
            0.80,
        );
        if let Some(hit) = hits.into_iter().next() {
            let cx = region.x1 + hit.col as i32 + template.width as i32 / 2;
            let cy = region.y1 + hit.row as i32 + template.height as i32 / 2;
            return Ok(Some((cx, cy)));
        }
    }
    Ok(None)
}

/// Matrix-scan `region` for the paperclip/attach icon. Returns the
/// **highest-scoring** match. We previously preferred the rightmost
/// match to dodge a chip-icon false positive at x≈1120, but the scan
/// region is now tightened to x≥1180 so chips are out of range; the
/// rightmost-preference began picking phantom matches at the top of
/// the scan band (e.g. (1210, 564)) instead of the real paperclip
/// further down. Highest-score is the simplest correct rule.
fn find_attach_button(
    region: Region,
    templates: &[crate::image_matrix::types::GrayMatrix],
) -> Result<Option<(i32, i32)>, String> {
    if templates.is_empty() {
        return Err("attach_button template not loaded".into());
    }
    let cap = crate::image_matrix::capture::capture_gray_matrix(
        region.x1, region.y1, region.x2, region.y2,
    )
    .map_err(|e| format!("capture failed: {e}"))?;

    let mut best: Option<(i32, i32, f32)> = None;
    for template in templates {
        let hits = crate::image_matrix::compare::find_all_matches_gray(
            &cap,
            template,
            crate::image_matrix::types::Tolerance::NORMAL,
            0.70,
        );
        for hit in hits {
            let cx = region.x1 + hit.col as i32 + template.width as i32 / 2;
            let cy = region.y1 + hit.row as i32 + template.height as i32 / 2;
            if region.y1 >= 850 && cy < region.y1 + 35 {
                // In-chat false positives appear in the upper edge of
                // the scan band, just above the New Chat pill. The real
                // paperclip is on the bottom composer row.
                continue;
            }
            match best {
                Some((_, _, bs)) if hit.score <= bs => {}
                _ => best = Some((cx, cy, hit.score)),
            }
        }
    }
    let region_tuple = (region.x1, region.y1, region.x2, region.y2);
    match best {
        Some((cx, cy, sc)) => {
            crate::image_matrix::telemetry::record("attach_button", Some(sc), region_tuple);
            Ok(Some((cx, cy)))
        }
        None => {
            // Save what we scanned so we can see WHY the paperclip wasn't
            // found — typically Chrome lost focus and we're looking at
            // Terminal text, or the layout shifted out of band.
            debug_capture_region(
                region.x1,
                region.y1,
                region.x2 - region.x1,
                region.y2 - region.y1,
                "attach_button_miss",
            );
            crate::image_matrix::telemetry::record("attach_button", None, region_tuple);
            Ok(None)
        }
    }
}

/// Navigate the macOS file-open dialog to `path` using Cmd+Shift+G.
///
/// The dialog must already be open. Puts the path in the clipboard, opens
/// "Go to Folder", clears any existing text, pastes, and confirms twice.
fn osascript_open_file_dialog(path: &str) -> Result<(), String> {
    let mut clip = arboard::Clipboard::new().map_err(|e| format!("clipboard init: {e}"))?;
    clip.set_text(path)
        .map_err(|e| format!("clipboard set: {e}"))?;

    let script = r#"
tell application "Google Chrome" to activate
tell application "System Events"
    delay 0.8
    keystroke "g" using {shift down, command down}
    delay 1.0
    keystroke "a" using command down
    delay 0.3
    keystroke "v" using command down
    delay 0.8
    key code 36
    delay 1.0
    key code 36
end tell
"#;

    for attempt in 1..=2 {
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

        std::thread::sleep(std::time::Duration::from_millis(800));
        if !chrome_file_dialog_open() {
            return Ok(());
        }

        eprintln!(
            "[upload] file dialog still open after path selection attempt {attempt}; retrying"
        );
    }

    close_chrome_file_dialog_if_open();
    Err("file dialog stayed open after selecting target path".into())
}

fn chrome_file_dialog_open() -> bool {
    let script = r#"
tell application "System Events"
    if not (exists process "Google Chrome") then return "false"
    tell process "Google Chrome"
        if (count of sheets of windows) > 0 then return "true"
        repeat with w in windows
            try
                if (name of w is "Open") then return "true"
            end try
        end repeat
    end tell
end tell
return "false"
"#;
    let Ok(out) = std::process::Command::new("osascript")
        .args(["-e", script])
        .output()
    else {
        return false;
    };
    String::from_utf8_lossy(&out.stdout).trim() == "true"
}

fn close_chrome_file_dialog_if_open() {
    if !chrome_file_dialog_open() {
        return;
    }
    eprintln!("[upload] stale file dialog open — pressing Escape before continuing");
    let script = r#"
tell application "Google Chrome" to activate
tell application "System Events"
    delay 0.2
    key code 53
end tell
"#;
    let _ = std::process::Command::new("osascript")
        .args(["-e", script])
        .output();
    std::thread::sleep(std::time::Duration::from_millis(600));
}
