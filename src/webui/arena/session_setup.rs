//! State-verified one-shot setup that runs ONCE at the start of an Arena session.
//!
//! Each step:
//!   1. **Pre-check** — read external truth (Chrome title / URL / frontmost app)
//!      or sample the relevant screen patch.
//!   2. **Act** — click / navigate / fullscreen.
//!   3. **Verify** — re-read truth source OR detect a screen-state delta
//!      (brightness or variance change). If verification fails, scan a wider
//!      area for a candidate (variance-based) and retry once.
//!   4. **Abort** — if still unverified, return `Err`. Caller bails out
//!      rather than continuing with bad state.
//!
//! Truth sources (cross-OS via `browser_state`):
//!   - Chrome window title — Direct mode shows `"Model · Arena"`.
//!   - Chrome URL — `/c/<id>` = old chat, plain `arena.ai` = home.
//!   - Frontmost app name — confirms focus.
//!
//! Pixel features (cross-OS, theme-agnostic):
//!   - **Brightness delta** of menu region pre/post click → "did dropdown open?"
//!   - **Row variance profile** inside menu region → row centers without
//!     hardcoded y-formulas (text rows have high pixel variance; gaps are flat).
//!   - **Scan-grid variance** around expected coords → find a button when the
//!     UI shifted and the expected coord hits dead space.

use std::error::Error;

use crate::calibration::match_score;
use crate::calibration::store::CalibrationStore;
use crate::image_matrix::capture::capture_gray_matrix;
use crate::image_matrix::compare::match_any_gray;
use crate::image_matrix::types::{GrayMatrix, Tolerance};
use crate::session::PromptMode;
use crate::session::SessionConfig;
use crate::webui::adapter::{Region, WebUiAdapter};
use crate::webui::arena::ARENA_HOME_URL;
use crate::webui::arena::ArenaAdapter;
use crate::webui::arena::templates;
use crate::webui::browser_state;

fn ts() -> String {
    chrono::Local::now().format("%H:%M:%S%.3f").to_string()
}

#[derive(Debug, Default)]
pub struct SetupReport {
    pub browser_focused: bool,
    pub navigated_to_arena: bool,
    pub reused_existing_tab: bool,
    pub navigated_to_home: bool,
    pub forced_fullscreen: bool,
    pub sidebar_collapsed: bool,
    pub clicked_new_chat: bool,
    pub forced_direct_mode: bool,
    pub model_selected: bool,
}

pub fn run(adapter: &ArenaAdapter, cfg: &SessionConfig) -> Result<SetupReport, Box<dyn Error>> {
    let mut report = SetupReport::default();
    let r = adapter.regions();

    if let Ok((w, h)) = crate::image_matrix::capture::primary_screen_size() {
        println!("[{}] [setup] primary screen: {w}×{h} logical", ts());
    }
    println!(
        "[{}] [setup] regions @ {}×{}: \
         mode_dropdown=({},{})→({},{}) mode_menu=({},{})→({},{}) \
         model_dropdown=({},{})→({},{}) model_menu=({},{})→({},{})",
        ts(),
        r.screen_width,
        r.screen_height,
        r.mode_dropdown.x1,
        r.mode_dropdown.y1,
        r.mode_dropdown.x2,
        r.mode_dropdown.y2,
        r.mode_menu_expanded.x1,
        r.mode_menu_expanded.y1,
        r.mode_menu_expanded.x2,
        r.mode_menu_expanded.y2,
        r.model_dropdown.x1,
        r.model_dropdown.y1,
        r.model_dropdown.x2,
        r.model_dropdown.y2,
        r.model_menu_expanded.x1,
        r.model_menu_expanded.y1,
        r.model_menu_expanded.x2,
        r.model_menu_expanded.y2,
    );

    // ── 1. Focus Chrome (3-attempt retry) ───────────────────────────────────
    println!("[{}] [setup] step 1 — focus Chrome (≤3 attempts)", ts());
    let mut focused = false;
    for attempt in 1..=3 {
        if let Err(e) = browser_state::focus_app_by_name("Chrome") {
            eprintln!(
                "[{}] [setup] attempt {attempt} focus_app_by_name failed: {e}",
                ts()
            );
        }
        pause_ms(500);
        let app = browser_state::frontmost_app_name().unwrap_or_default();
        println!("[{}] [setup] attempt {attempt}: frontmost='{app}'", ts());
        if app.to_lowercase().contains("chrome") {
            println!("[{}] [setup] Chrome frontmost (verified)", ts());
            focused = true;
            break;
        }
        eprintln!(
            "[{}] [setup] WARN attempt {attempt} did not bring Chrome forward",
            ts()
        );
    }
    if !focused {
        return Err(
            "verify failed step 1: Chrome did not become frontmost after 3 attempts".into(),
        );
    }
    report.browser_focused = true;

    // ── 2. Find or open an arena chat tab ───────────────────────────────────
    //
    // Don't clobber the user's other tabs by navigating them. Order:
    //   a. If any existing Chrome tab is on arena.ai chat (excluding
    //      leaderboard / help / blog), focus that tab.
    //   b. If the front tab IS already arena home (arena.ai/) and looks
    //      like the chat app (verified by title), reuse it.
    //   c. Otherwise, open a NEW tab pointed at arena.ai.
    // After landing: verify we're actually on the chat app, NOT the
    // marketing/leaderboard page.
    println!("[{}] [setup] step 2 — locate arena chat tab", ts());
    let url0 = browser_state::active_chrome_url().unwrap_or_default();
    let title0 = browser_state::active_chrome_title().unwrap_or_default();
    println!(
        "[{}] [setup] current tab: url='{url0}'  title='{title0}'",
        ts()
    );

    let mut landed = false;
    match browser_state::focus_chrome_arena_chat_tab() {
        Ok(true) => {
            pause_ms(500);
            let url = browser_state::active_chrome_url().unwrap_or_default();
            println!("[{}] [setup] focused existing arena chat tab: {url}", ts());
            report.reused_existing_tab = true;
            landed = true;
        }
        Ok(false) => {
            println!("[{}] [setup] no existing arena chat tab found", ts());
        }
        Err(e) => {
            eprintln!(
                "[{}] [setup] focus_chrome_arena_chat_tab failed: {e} — falling through",
                ts()
            );
        }
    }

    if !landed {
        // No existing tab — open a NEW tab so we don't clobber whatever's open.
        println!("[{}] [setup] opening new tab → {ARENA_HOME_URL}", ts());
        browser_state::open_new_browser_tab(ARENA_HOME_URL)
            .map_err(|e| format!("open_new_browser_tab: {e}"))?;
        pause_ms(2200);
        report.navigated_to_home = true;
    }
    report.navigated_to_arena = true;
    report.clicked_new_chat = true;

    // Verify we're on the chat app, not the marketing/leaderboard page.
    // Coding uses Arena's Code filter and code-view matrices. Research
    // uses Arena's Text filter and text-view scaffold; that surface gets
    // its own calibration as screenshots are captured.
    let (target_category_url, target_segment) = match cfg.prompt_mode {
        PromptMode::Coding => ("https://arena.ai/code/direct", "/code/"),
        PromptMode::Research => ("https://arena.ai/text/direct", "/text/"),
    };
    let post_url = browser_state::active_chrome_url().unwrap_or_default();
    let post_title = browser_state::active_chrome_title().unwrap_or_default();
    println!(
        "[{}] [setup] landed: url='{post_url}'  title='{post_title}' surface={:?}",
        ts(),
        adapter.surface()
    );
    assert_no_modal("step-2-after-arena-load")?;
    if !post_url.contains("arena.ai") {
        return Err(format!("verify failed step 2: URL='{post_url}' is not arena.ai").into());
    }

    // Always navigate to the target category so the model picker shows
    // the right model list. This also handles marketing-page redirects.
    let already_on_target = post_url.contains(target_segment);
    if !already_on_target || is_marketing_title(&post_title) {
        println!(
            "[{}] [setup] navigating to category URL: {target_category_url}",
            ts()
        );
        browser_state::navigate_current_tab(target_category_url)
            .map_err(|e| format!("navigate to {target_category_url}: {e}"))?;
        pause_ms(2500);
        let retry_title = browser_state::active_chrome_title().unwrap_or_default();
        let retry_url = browser_state::active_chrome_url().unwrap_or_default();
        println!(
            "[{}] [setup] after category nav: url='{retry_url}'  title='{retry_title}'",
            ts()
        );
        if is_marketing_title(&retry_title) {
            return Err(format!(
                "verify failed step 2: still on marketing page after redirect (title '{retry_title}'). \
                 Log in at https://arena.ai then re-run."
            ).into());
        }
    } else {
        println!(
            "[{}] [setup] already on correct category ({post_url})",
            ts()
        );
    }

    // ── 3. Window-fills-screen check (lenient) ───────────────────────────────
    //
    // Goal: confirm Chrome's active window is wide enough that the
    // calibrated regions (which assume content covers full screen width)
    // will land on the right elements. We DON'T require AXFullScreen=true:
    //   - macOS native Fullscreen Space, "Zoomed", and edge-maximized all
    //     leave width = screen width.
    //   - On notched MBPs, Chrome's `bounds of front window` can report a
    //     content-area-style y-offset (~120px) even in fullscreen Space.
    // So we accept any window whose width matches screen width; height
    // and y-offset are informational only. Downstream pixel verifies
    // catch real region mismatches.
    println!("[{}] [setup] step 3 — window-fills-screen check", ts());
    let (sw, sh) = crate::image_matrix::capture::primary_screen_size()
        .map_err(|e| format!("primary_screen_size: {e}"))?;

    // Diagnostic: dump every Chrome window so multi-window cases are visible.
    match browser_state::chrome_list_windows() {
        Ok(wins) => {
            println!("[{}] [setup] {} Chrome window(s):", ts(), wins.len());
            for (idx, x1, y1, x2, y2, url) in &wins {
                let w = x2 - x1;
                let h = y2 - y1;
                println!(
                    "[{}] [setup]   win {idx}: bounds=({x1},{y1})→({x2},{y2})  size {w}×{h}  url={url}",
                    ts()
                );
            }
        }
        Err(e) => eprintln!("[{}] [setup] chrome_list_windows: {e}", ts()),
    }
    let ax_fs = browser_state::is_chrome_fullscreen().unwrap_or(false);
    println!("[{}] [setup] AXFullScreen={ax_fs}", ts());

    let bounds =
        browser_state::chrome_window_bounds().map_err(|e| format!("chrome_window_bounds: {e}"))?;
    let (bx1, by1, bx2, by2) = bounds;
    let bw = bx2 - bx1;
    let bh = by2 - by1;
    let width_pct = (bw as f64 / sw as f64) * 100.0;
    let height_pct = (bh as f64 / sh as f64) * 100.0;
    println!(
        "[{}] [setup] front window: bounds=({bx1},{by1})→({bx2},{by2})  size {bw}×{bh}  vs screen {sw}×{sh}  width={width_pct:.1}% height={height_pct:.1}%",
        ts()
    );

    let width_ok = bw >= sw - 8;
    if width_ok {
        println!(
            "[{}] [setup] window width matches screen — accepting (height_pct={height_pct:.1}%, AXFullScreen={ax_fs})",
            ts()
        );
        report.forced_fullscreen = true;
    } else {
        eprintln!(
            "[{}] [setup] WARN window width {bw} < screen width {sw} — attempting fullscreen toggle",
            ts()
        );
        if let Err(e) = browser_state::set_chrome_fullscreen(true) {
            return Err(format!("set_chrome_fullscreen: {e}").into());
        }
        for attempt in 1..=4 {
            pause_ms(700);
            let (cx1, cy1, cx2, cy2) =
                browser_state::chrome_window_bounds().unwrap_or((0, 0, 0, 0));
            let cw = cx2 - cx1;
            println!(
                "[{}] [setup] post-toggle attempt {attempt}: ({cx1},{cy1})→({cx2},{cy2}) width={cw}",
                ts()
            );
            if cw >= sw - 8 {
                println!("[{}] [setup] window width now matches screen", ts());
                report.forced_fullscreen = true;
                break;
            }
            if attempt == 4 {
                return Err(format!(
                    "verify failed step 3: window width {cw} still < screen width {sw} after fullscreen toggle"
                ).into());
            }
        }
    }

    // ── 4. Collapse sidebar ──────────────────────────────────────────────────
    println!("[{}] [setup] step 4 — collapse sidebar", ts());
    if !is_sidebar_expanded() {
        println!("[{}] [setup] sidebar already collapsed", ts());
    } else {
        collapse_sidebar(r)?;
    }
    verify_template_or_warn(
        "sidebar_collapsed_new_chat_icon",
        r.new_chat_button_when_collapsed,
        templates::new_chat_collapsed_only(),
        0.88,
    )?;
    report.sidebar_collapsed = true;
    log_url("after sidebar collapse");
    assert_no_modal("step-4-sidebar")?;

    // ── 5. Verify Direct mode via URL ──────────────────────────────────────
    //
    // Step 2 already navigated to /text/direct or /code/direct (based on
    // prompt_mode), so we should already be in Direct mode. Just verify.
    println!("[{}] [setup] step 5 — verify Direct mode", ts());
    let step5_url = browser_state::active_chrome_url().unwrap_or_default();
    println!("[{}] [setup] URL: '{step5_url}'", ts());
    if step5_url.contains("/direct") {
        println!(
            "[{}] [setup] URL contains /direct — already in Direct mode",
            ts()
        );
        report.forced_direct_mode = true;
    } else {
        // On an active /c/<id> chat or other URL — navigate to the right category.
        println!(
            "[{}] [setup] URL does not contain /direct — navigating to {target_category_url}",
            ts()
        );
        browser_state::navigate_current_tab(target_category_url)
            .map_err(|e| format!("navigate to {target_category_url}: {e}"))?;
        pause_ms(2500);
        let post_url = browser_state::active_chrome_url().unwrap_or_default();
        if post_url.contains("/direct") {
            println!("[{}] [setup] now on Direct mode: '{post_url}'", ts());
            report.forced_direct_mode = true;
        } else {
            return Err(format!(
                "verify failed step 5: URL '{post_url}' still doesn't contain /direct \
                 after navigation to {target_category_url}"
            )
            .into());
        }
    }
    verify_template_or_warn(
        "mode_dropdown_direct",
        r.mode_dropdown,
        templates::mode_direct(),
        0.78,
    )?;
    // Re-collapse sidebar if navigation expanded it.
    if is_sidebar_expanded() {
        println!(
            "[{}] [setup] sidebar re-expanded after step 5 — collapsing",
            ts()
        );
        collapse_sidebar(r)?;
    }
    let sidebar_is_expanded = is_sidebar_expanded();
    log_url("after Direct mode");
    assert_no_modal("step-5-after-direct")?;

    // ── 6. Select model ──────────────────────────────────────────────────────
    if let Some(ref model) = cfg.arena_model {
        println!("[{}] [setup] step 6 — model '{model}'", ts());
        let cur = adapter.title_model();
        println!("[{}] [setup] current title_model = {:?}", ts(), cur);
        let already = cur
            .as_deref()
            .map(|m| m.to_lowercase().contains(&model.to_lowercase()))
            .unwrap_or(false);
        if already {
            println!("[{}] [setup] model already correct — skipping", ts());
            report.model_selected = true;
        } else {
            let menu = r.model_menu_expanded;
            let header = if sidebar_is_expanded {
                r.model_dropdown_expanded
            } else {
                r.model_dropdown
            };
            let pre_header = capture_region(header);
            println!(
                "[{}] [setup] using {} model_dropdown region",
                ts(),
                if sidebar_is_expanded {
                    "EXPANDED"
                } else {
                    "collapsed"
                }
            );
            let (hx, hy) = center(header);

            if !open_dropdown_with_verify(hx, hy, header, menu, "model")? {
                return Err(
                    "verify failed step 6: model dropdown did not open after retry+scan".into(),
                );
            }

            // CLIPBOARD-PASTE FLOW with search-input-region verification.
            //
            // Why paste over typing:
            //   - char-by-char typing caused dropped chars / focus disruption
            //     (especially through cmd+a + delete sequences).
            //   - Cmd+V is ATOMIC — either the search receives the full model
            //     name in one shot, or nothing (we'd see no diff and abort).
            //
            // Sequence:
            //   1. Save existing clipboard (so we can restore it after).
            //   2. Set clipboard = model name.
            //   3. Capture search-input row pre-state.
            //   4. Click candidate search-input position.
            //   5. Cmd+V to paste.
            //   6. Capture search-input row, diff pre vs post. Threshold 8%
            //      because long string ≈ many text pixels (much more than 1
            //      char's ~2%).
            //   7. If unchanged → abort. NO Tab fallback (Tab moves focus to
            //      the clear-input X button on Arena's picker, breaking the
            //      sequence further).
            //   8. Down + Enter, verify picker closed.
            //   9. Restore original clipboard.
            pause_ms(250);

            let search_input_region = model_search_input_region(menu);
            let mut pre_input = capture_region(search_input_region).ok_or_else(|| {
                format!("verify failed step 6: could not capture search-input pre-state")
            })?;
            let mut pre_var = patch_variance(
                search_input_region.x1,
                search_input_region.y1,
                search_input_region.x2,
                search_input_region.y2,
            );
            // Save the PRE-paste search-input region so user can diff visually.
            let pre_stamp = chrono::Local::now().format("%H%M%S").to_string();
            let pre_input_png = crate::output::writer::screenshot_path(format!(
                "search_input_pre_paste_{pre_stamp}.png"
            ));
            let _ = save_screen_png(
                &pre_input_png,
                search_input_region.x1,
                search_input_region.y1,
                search_input_region.x2,
                search_input_region.y2,
            );
            println!(
                "[{}] [setup] pre-paste search-input PNG → {} (variance={pre_var:.1})",
                ts(),
                pre_input_png.display()
            );

            // Save current clipboard so we can restore it.
            let mut clip = arboard::Clipboard::new().map_err(|e| format!("arboard new: {e}"))?;
            let saved_clip = clip.get_text().ok();

            // Set clipboard to model name.
            clip.set_text(model.to_string())
                .map_err(|e| format!("clipboard set: {e}"))?;
            pause_ms(80);
            println!(
                "[{}] [setup] clipboard set to '{model}' (len={})",
                ts(),
                model.len()
            );

            // Click search input.
            let search_x = (menu.x1 + menu.x2) / 2;
            let search_y =
                match find_row_centers(menu.x1 + 20, menu.y1 + 4, menu.x2 - 20, menu.y1 + 80)
                    .first()
                {
                    Some(&y) => y,
                    None => menu.y1 + 25,
                };
            println!(
                "[{}] [setup] click search input ({search_x},{search_y})",
                ts()
            );
            crate::human_simulations::move_mouse_single_click(search_x, search_y)
                .map_err(|e| format!("search click: {e}"))?;
            pause_ms(320);

            // Clear any leftover text in the search input (could be from a
            // previous run that aborted with picker open). cmd+a selects
            // existing content; backspace deletes selection (NOT delete —
            // macOS Delete key is forward-delete and disrupted focus).
            if pre_var > 200.0 {
                println!(
                    "[{}] [setup] search input has leftover content (var={pre_var:.1}) — clearing with cmd+a + backspace",
                    ts()
                );
                let _ = crate::human_simulations::press_key("cmd+a");
                pause_ms(120);
                let _ = crate::human_simulations::press_key("backspace");
                pause_ms(180);
                let cleared_var = patch_variance(
                    search_input_region.x1,
                    search_input_region.y1,
                    search_input_region.x2,
                    search_input_region.y2,
                );
                println!(
                    "[{}] [setup] search input variance after clear: {cleared_var:.1}",
                    ts()
                );

                // Re-capture pre_input AFTER clearing so the paste comparison
                // is empty→filled, not old-text→same-text (which gives 0% diff
                // when the same model name was leftover from a prior run).
                pre_input = capture_region(search_input_region).ok_or_else(|| {
                    format!("verify failed step 6: could not re-capture search-input post-clear")
                })?;
                pre_var = cleared_var;
            }

            // Paste.
            println!("[{}] [setup] press cmd+v (atomic paste)", ts());
            crate::human_simulations::press_key("cmd+v")
                .map_err(|e| format!("press_key cmd+v: {e}"))?;
            pause_ms(900); // wait for picker to filter

            // Multi-signal verify: text in input adds (a) pixel changes
            // even if subtle (dark-on-dark), AND (b) ALWAYS increases
            // variance because text breaks up uniform background.
            let post_input = capture_region(search_input_region).ok_or_else(|| {
                format!("verify failed step 6: could not capture search-input post-paste")
            })?;
            let post_var = patch_variance(
                search_input_region.x1,
                search_input_region.y1,
                search_input_region.x2,
                search_input_region.y2,
            );
            let total_pixels = pre_input.data.len();
            let diff_count = crate::calibration::diff_pixel_count(&pre_input, &post_input, 5);
            let diff_pct: f32 = diff_count as f32 / total_pixels.max(1) as f32 * 100.0;
            let var_delta = (post_var - pre_var).abs();

            let pass_diff = diff_pct >= 2.0;
            let pass_var = post_var >= pre_var + 30.0 && post_var >= 80.0;
            let pasted = pass_diff && pass_var;

            println!(
                "[{}] [setup] paste verify: diff_count={diff_count}/{total_pixels} ({diff_pct:.2}%) @ tol 5  |  variance {pre_var:.1} → {post_var:.1} (Δ {var_delta:.1})  |  pass_diff={pass_diff} pass_var={pass_var} → ok={pasted}",
                ts()
            );

            // Diagnostic captures.
            let stamp = chrono::Local::now().format("%H%M%S").to_string();
            let input_png = crate::output::writer::screenshot_path(format!(
                "search_input_post_paste_{stamp}.png"
            ));
            let picker_png = crate::output::writer::screenshot_path(format!(
                "model_picker_post_search_{stamp}.png"
            ));
            let _ = save_screen_png(
                &input_png,
                search_input_region.x1,
                search_input_region.y1,
                search_input_region.x2,
                search_input_region.y2,
            );
            let _ = save_screen_png(&picker_png, menu.x1, menu.y1, menu.x2, menu.y2);
            println!(
                "[{}] [setup] search-input PNG → {}",
                ts(),
                input_png.display()
            );
            println!(
                "[{}] [setup] full picker PNG  → {}",
                ts(),
                picker_png.display()
            );

            if !pasted {
                if let Some(s) = saved_clip {
                    let _ = clip.set_text(s);
                }
                return Err(format!(
                    "verify failed step 6: cmd+v paste did NOT fill model search input. \
                     diff={diff_pct:.2}% var_delta={var_delta:.1} (need ≥2% diff AND post variance ≥ pre+30, post ≥80). \
                     Search input wasn't focused at ({search_x},{search_y}). \
                     Compare {} vs {}.",
                    pre_input_png.display(),
                    input_png.display()
                )
                .into());
            }

            // Highlight first match + select via Enter.
            println!("[{}] [setup] press Down then Enter", ts());
            crate::human_simulations::press_key("down")
                .map_err(|e| format!("press_key down: {e}"))?;
            pause_ms(150);
            crate::human_simulations::press_key("return")
                .map_err(|e| format!("press_key return: {e}"))?;
            pause_ms(900);

            // Verify the picker closed.
            let post_brt = patch_brightness(menu.x1, menu.y1, menu.x2, menu.y2);
            let post_var = patch_variance(menu.x1, menu.y1, menu.x2, menu.y2);
            println!(
                "[{}] [setup] picker post-Enter: brightness={post_brt:.1} variance={post_var:.1}",
                ts()
            );
            if is_menu_panel_open(post_brt, post_var) {
                if let Some(s) = saved_clip {
                    let _ = clip.set_text(s);
                }
                let path = crate::output::writer::screenshot_path(format!(
                    "model_picker_stuck_{stamp}.png"
                ));
                let _ = save_screen_png(&path, menu.x1, menu.y1, menu.x2, menu.y2);
                return Err(format!(
                    "verify failed step 6: picker still open after Down+Enter (b={post_brt:.1} v={post_var:.1}). \
                     See {}.",
                    path.display()
                ).into());
            }
            // The page title on /c/<id> chats is GENERIC ("Chat with Multiple
            // Frontier AI Models") and does NOT reflect the selected model.
            // Title is therefore unreliable for confirming model choice.
            // Truth sources we DO have:
            //   - Picker filtered correctly during typing  (verified above)
            //   - Picker closed after Down+Enter           (verified above)
            //   - The model dropdown header should now show the selected
            //     model's name. We can:
            //       a) Compare to a calibrated reference if user has saved
            //          one for THIS specific model (e.g. via custom calibration)
            //       b) Save a diagnostic PNG of the header for visual review
            //       c) Verify it CHANGED from before the picker click
            let post_title = browser_state::active_chrome_title().unwrap_or_default();
            let post_url = browser_state::active_chrome_url().unwrap_or_default();
            println!(
                "[{}] [setup] post-select title='{post_title}' url='{post_url}'",
                ts()
            );

            // Capture model dropdown header post-select for visual review.
            let post_header = capture_region(header);
            let header_png = crate::output::writer::screenshot_path(format!(
                "model_header_after_select_{stamp}.png"
            ));
            let _ = save_screen_png(&header_png, header.x1, header.y1, header.x2, header.y2);
            println!(
                "[{}] [setup] model header capture → {} (visually confirm '{model}')",
                ts(),
                header_png.display()
            );
            if let (Some(pre), Some(post)) = (&pre_header, &post_header) {
                let diff = crate::calibration::diff_pixel_count(pre, post, 8);
                let total = pre.data.len().max(1);
                let diff_pct = diff as f32 / total as f32 * 100.0;
                println!(
                    "[{}] [setup] model header matrix diff after select: {diff}/{total} ({diff_pct:.2}%)",
                    ts()
                );
                if diff_pct < 0.5 {
                    eprintln!(
                        "[{}] [setup] WARN model header barely changed ({diff_pct:.2}%). \
                         Continuing because picker search/paste/close were verified; add calibration ref for strict model-name proof.",
                        ts()
                    );
                }
            }

            // Try a per-model reference match if the user has calibrated one.
            let model_ref_name = format!(
                "model_dropdown_{}_selected",
                model.replace(['.', '/', ' ', '·'], "_")
            );
            let store = CalibrationStore::default_arena();
            if store.exists(&model_ref_name) {
                if let Some(score) = ref_score(&model_ref_name, header) {
                    println!(
                        "[{}] [setup] model header ref '{model_ref_name}' score={score:.3}",
                        ts()
                    );
                    if score < 0.85 {
                        return Err(format!(
                            "verify failed step 6: model header doesn't match calibrated '{model_ref_name}' (score {score:.3} < 0.85). \
                             Captured → {}. Recalibrate or check model name.",
                            header_png.display()
                        ).into());
                    }
                    println!(
                        "[{}] [setup] model '{model}' confirmed via calibration ref",
                        ts()
                    );
                }
            } else {
                println!(
                    "[{}] [setup] no calibration ref '{model_ref_name}' — model selection trusted via filter+close. \
                     To strictly verify, set the model in Arena, then save assets/macos/arena.ai/calibration/{model_ref_name}.png \
                     (manual capture or calibrate subcommand if added).",
                    ts()
                );
            }

            // Restore the user's clipboard so we don't pollute it.
            if let Some(s) = saved_clip {
                if let Err(e) = clip.set_text(s) {
                    eprintln!("[{}] [setup] WARN clipboard restore failed: {e}", ts());
                } else {
                    println!("[{}] [setup] clipboard restored to original contents", ts());
                }
            } else {
                println!(
                    "[{}] [setup] (no original clipboard contents to restore)",
                    ts()
                );
            }

            report.model_selected = true;
        }
        log_url("after model select");
        assert_no_modal("step-6-after-model")?;
    } else {
        println!(
            "[{}] [setup] step 6 skipped — no arena_model configured",
            ts()
        );
    }

    // ── 7. Force fresh new chat (no context bleed between iterations) ────────
    //
    // Even if we just verified the right model, the browser may be on an
    // old /c/<id> chat with prior messages. Sending a new prompt there
    // would feed those messages back to the model. The user explicitly
    // requires a fresh chat at session start.
    println!("[{}] [setup] step 7 — start fresh new chat", ts());
    let pre_chat_url = browser_state::active_chrome_url().unwrap_or_default();
    let input_pre_var = patch_variance(
        r.input_box.x1,
        r.input_box.y1,
        r.input_box.x2,
        r.input_box.y2,
    );
    println!(
        "[{}] [setup] pre-new-chat: url='{pre_chat_url}'  input_variance={input_pre_var:.1}",
        ts()
    );

    if let Err(e) = adapter.start_new_chat() {
        return Err(format!("verify failed step 7: start_new_chat: {e:?}").into());
    }
    pause_ms(800);

    let post_chat_url = browser_state::active_chrome_url().unwrap_or_default();
    let input_post_var = patch_variance(
        r.input_box.x1,
        r.input_box.y1,
        r.input_box.x2,
        r.input_box.y2,
    );
    println!(
        "[{}] [setup] post-new-chat: url='{post_chat_url}'  input_variance={input_post_var:.1}",
        ts()
    );
    // Acceptance: any URL NOT containing /c/ means we're on Arena's fresh
    // chat entry (text/direct, code/direct, /, etc.) — no prior context.
    // The adapter handles both "was already fresh" (no-op) and "was on /c/
    // → switched off" cases; the only failure is staying on /c/ after.
    if post_chat_url.contains("/c/") {
        return Err(format!(
            "verify failed step 7: still on active chat URL '{post_chat_url}' after start_new_chat. \
             New-chat button missed or page didn't reset."
        ).into());
    }
    println!(
        "[{}] [setup] verified: URL '{post_chat_url}' is a fresh-chat entry (no /c/<id>)",
        ts()
    );
    // Input should be empty / placeholder only — variance under ~150 typically.
    // Don't hard-fail; just warn loudly because some chat-app empty states
    // include greeting text inside the input region.
    if input_post_var > 300.0 {
        eprintln!(
            "[{}] [setup] WARN input region variance {input_post_var:.1} suggests input isn't empty",
            ts()
        );
    } else {
        println!(
            "[{}] [setup] input box appears empty (variance {input_post_var:.1} ≤ 300)",
            ts()
        );
    }
    verify_template_or_warn(
        "fresh_send_button_visible",
        r.send_or_stop,
        templates::send_button(),
        0.78,
    )?;
    assert_no_modal("step-7-after-new-chat")?;
    log_url("after new chat");

    // ── 8. Final ─────────────────────────────────────────────────────────────
    let final_url = browser_state::active_chrome_url().unwrap_or_else(|_| "<unread>".into());
    let final_title = browser_state::active_chrome_title().unwrap_or_else(|_| "<unread>".into());
    println!("[{}] [setup] final URL:   {final_url}", ts());
    println!("[{}] [setup] final title: {final_title}", ts());
    println!("[{}] [setup] report: {report:?}", ts());
    let _ = adapter.detect_state();
    Ok(report)
}

// ── Helpers ─────────────────────────────────────────────────────────────────

fn pause_ms(ms: u64) {
    std::thread::sleep(std::time::Duration::from_millis(ms));
}

/// Capture a region as `GrayMatrix`. Returns None on capture failure.
fn capture_region(region: Region) -> Option<GrayMatrix> {
    match capture_gray_matrix(region.x1, region.y1, region.x2, region.y2) {
        Ok(m) => Some(m),
        Err(e) => {
            eprintln!("[{}] [setup] capture {region:?} failed: {e}", ts());
            None
        }
    }
}

/// Compare a captured region against a saved calibration reference.
/// Returns the match score (0.0–1.0). Returns None if no reference is
/// stored for `state_name` (caller should treat as "skip absolute check
/// — fall back to heuristic").
fn ref_score(state_name: &str, region: Region) -> Option<f32> {
    let store = CalibrationStore::default_arena();
    if !store.exists(state_name) {
        return None;
    }
    let cap = capture_region(region)?;
    let reference = match store.load(state_name) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[{}] [setup] could not load ref '{state_name}': {e}", ts());
            return None;
        }
    };
    let score = match_score(&cap, &reference, 15);
    Some(score)
}

fn verify_template_or_warn(
    label: &str,
    region: Region,
    templates_: &[GrayMatrix],
    threshold: f32,
) -> Result<bool, Box<dyn Error>> {
    if templates_.is_empty() {
        eprintln!(
            "[{}] [setup] WARN no template assets loaded for '{label}'",
            ts()
        );
        return Ok(false);
    }
    let cap = match capture_region(region) {
        Some(c) => c,
        None => {
            return Err(
                format!("verify failed: could not capture template region '{label}'").into(),
            );
        }
    };
    let matched = match_any_gray(&cap, templates_, Tolerance::NORMAL, threshold);
    println!(
        "[{}] [setup] template-matrix '{label}' matched={} threshold={threshold:.2} region=({},{})→({},{})",
        ts(),
        matched,
        region.x1,
        region.y1,
        region.x2,
        region.y2
    );
    if !matched {
        let stamp = chrono::Local::now().format("%H%M%S").to_string();
        let path =
            crate::output::writer::screenshot_path(format!("template_miss_{stamp}_{label}.png"));
        let _ = save_screen_png(&path, region.x1, region.y1, region.x2, region.y2);
        eprintln!(
            "[{}] [setup] WARN template '{label}' not found; captured → {}",
            ts(),
            path.display()
        );
    }
    Ok(matched)
}

/// Detect a centered modal/dialog overlay (CAPTCHA, security check,
/// "session expired" dialog, etc.). Pattern observed:
///   - The page dims behind a translucent overlay → edges of the screen
///     show very LOW brightness and very LOW variance (uniform dim wash).
///   - A bright dialog panel sits in the centre → high variance.
///
/// Heuristic: edge regions average variance ≤ 35 AND average brightness ≤
/// 30 AND centre region variance ≥ 250 AND centre is visibly brighter
/// than the dimmed edges. Returns a description on detection.
///
/// Saves the offending screen to the run screenshots folder for
/// later visual review.
fn detect_modal_overlay(label: &str) -> Option<String> {
    let (sw, sh) = match crate::image_matrix::capture::primary_screen_size() {
        Ok(s) => s,
        Err(_) => return None,
    };

    // Strict template check first if user calibrated the captcha.
    let store = CalibrationStore::default_arena();
    if store.exists("captcha_modal") {
        let region = Region {
            x1: 0,
            y1: 0,
            x2: sw,
            y2: sh,
        };
        if let Some(score) = ref_score("captcha_modal", region) {
            println!("[{}] [setup] captcha ref-match score={score:.3}", ts());
            if score >= 0.85 {
                return Some(format!(
                    "captcha modal matched calibration template (score={score:.3})"
                ));
            }
        }
    }
    // Four edge sample patches — well inside the screen but well clear
    // of any centred modal. 60×60 each.
    let patches = [
        (60, sh / 2),      // far left middle
        (sw - 60, sh / 2), // far right middle
        (sw / 2, sh - 80), // bottom middle
        (sw / 4, 300),     // upper-left quadrant
        (3 * sw / 4, 300), // upper-right quadrant
    ];
    let half = 30;
    let mut br_sum = 0.0;
    let mut var_sum = 0.0;
    for &(x, y) in &patches {
        br_sum += patch_brightness(x - half, y - half, x + half, y + half);
        var_sum += patch_variance(x - half, y - half, x + half, y + half);
    }
    let edge_b_avg = br_sum / patches.len() as f64;
    let edge_v_avg = var_sum / patches.len() as f64;

    // Centre — a 200×200 patch around screen centre.
    let cx = sw / 2;
    let cy = sh / 2;
    let centre_b = patch_brightness(cx - 100, cy - 100, cx + 100, cy + 100);
    let centre_v = patch_variance(cx - 100, cy - 100, cx + 100, cy + 100);

    let dimmed_edges = edge_b_avg <= 30.0 && edge_v_avg <= 35.0;
    let centre_panel = centre_v >= 250.0 && centre_b >= 35.0 && centre_b >= edge_b_avg + 8.0;
    if dimmed_edges && centre_panel {
        // Save a diagnostic capture so the user can inspect what we saw.
        let stamp = chrono::Local::now().format("%H%M%S").to_string();
        let path =
            crate::output::writer::screenshot_path(format!("modal_detected_{stamp}_{label}.png"));
        if let Err(e) = save_screen_png(&path, 0, 0, sw, sh) {
            eprintln!("[{}] [setup] could not save diagnostic png: {e}", ts());
        } else {
            println!(
                "[{}] [setup] saved modal diagnostic → {}",
                ts(),
                path.display()
            );
        }
        Some(format!(
            "modal overlay detected after '{label}': edge_brightness={edge_b_avg:.1}, edge_variance={edge_v_avg:.1}, centre_brightness={centre_b:.1}, centre_variance={centre_v:.1}"
        ))
    } else {
        None
    }
}

/// Convenience: bail with a clear error if a modal overlay (CAPTCHA etc.)
/// is up. Use after every step's actions complete.
fn assert_no_modal(label: &str) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(why) = detect_modal_overlay(label) {
        return Err(format!(
            "MODAL/CAPTCHA detected after step '{label}'. {why}\n\
             Solve the modal manually in Chrome, then re-run the session."
        )
        .into());
    }
    Ok(())
}

/// Save a screen rectangle to disk as RGB PNG for diagnostic review.
fn save_screen_png<P: AsRef<std::path::Path>>(
    path: P,
    x1: i32,
    y1: i32,
    x2: i32,
    y2: i32,
) -> Result<(), String> {
    use std::fs;
    let path = path.as_ref();
    let cap = crate::image_matrix::capture::capture_rgb_matrix(x1, y1, x2, y2)
        .map_err(|e| format!("capture_rgb_matrix: {e}"))?;
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let mut flat: Vec<u8> = Vec::with_capacity(cap.data.len() * 3);
    for px in &cap.data {
        flat.extend_from_slice(px);
    }
    let img = image::RgbImage::from_raw(cap.width as u32, cap.height as u32, flat)
        .ok_or_else(|| "from_raw failed".to_string())?;
    img.save(path).map_err(|e| format!("png save: {e}"))?;
    Ok(())
}

/// Detector for "is a dropdown panel currently rendered in this region?"
/// Used only after selecting an item. Loose by design: if menu still has
/// high density, it probably did not close.
fn is_menu_panel_open(brightness: f64, variance: f64) -> bool {
    variance >= 400.0 || brightness >= 60.0
}

fn model_search_input_region(menu: Region) -> Region {
    Region {
        x1: menu.x1 + 30,
        y1: menu.y1 + 8,
        x2: menu.x2 - 30,
        y2: menu.y1 + 48,
    }
}

fn dropdown_search_row_open(menu: Region, label: &str) -> bool {
    let search = model_search_input_region(menu);
    let var = patch_variance(search.x1, search.y1, search.x2, search.y2);
    let brightness = patch_brightness(search.x1, search.y1, search.x2, search.y2);

    // Empty open picker search rows have a low but non-trivial signal from
    // search icon + placeholder (observed ~26-35 var). Closed/background
    // rows are flatter (~16 var). Normal chat page content can be much
    // noisier (~190 var) but not bright enough; reject that case so we do
    // not paste the model into the chat input.
    let open = (22.0..=150.0).contains(&var) || brightness >= 50.0;
    println!(
        "[{}] [setup] {label} search-row matrix: brightness={brightness:.1} variance={var:.1} open={open} region=({},{})→({},{})",
        ts(),
        search.x1,
        search.y1,
        search.x2,
        search.y2
    );
    open
}

/// True when the title looks like the arena.ai MARKETING/leaderboard page,
/// NOT the chat app. The marketing page title is long and SEO-formatted;
/// the chat app's title is short ("Arena" or "<Model> · Arena").
pub(crate) fn is_marketing_title(title: &str) -> bool {
    let t = title.to_lowercase();
    t.contains("leaderboard")
        || t.contains("ranking")
        || t.contains("the official")
        || t.contains("llm leaderboard")
}

fn center(r: Region) -> (i32, i32) {
    ((r.x1 + r.x2) / 2, (r.y1 + r.y2) / 2)
}

/// Reliable sidebar-expanded detection using variance over a wide content
/// area. Arena's dark theme makes brightness unreliable (both states are
/// dark). Variance detects the TEXT content ("New Chat", "Leaderboard",
/// "Search", chat history) that's present only when expanded.
///
/// Check area: x=60-180, y=160-300 — well inside the expanded sidebar's
/// content column (x=0-220) but fully outside the collapsed sidebar's
/// icon strip (x=0-55). When collapsed, this region is uniform dark
/// page background → variance < 30. When expanded, text content pushes
/// variance > 100.
fn is_sidebar_expanded() -> bool {
    let var = patch_variance(60, 160, 180, 300);
    let bright = patch_brightness(60, 160, 180, 300);
    println!(
        "[{}] [setup] sidebar detect: variance={var:.1} brightness={bright:.1} → {}",
        ts(),
        if var >= 50.0 { "EXPANDED" } else { "collapsed" }
    );
    var >= 50.0
}

/// Click the sidebar toggle to collapse and verify.
fn collapse_sidebar(
    r: &crate::webui::arena::regions::Regions,
) -> Result<(), Box<dyn std::error::Error>> {
    for attempt in 1..=3 {
        let (tx, ty) = center(r.sidebar_toggle_when_expanded);
        println!(
            "[{}] [setup] collapse sidebar attempt {attempt}: click ({tx},{ty})",
            ts()
        );
        crate::human_simulations::move_mouse_single_click(tx, ty)
            .map_err(|e| format!("sidebar toggle click attempt {attempt}: {e}"))?;
        pause_ms(600);
        if !is_sidebar_expanded() {
            println!("[{}] [setup] sidebar collapsed (verified)", ts());
            return Ok(());
        }
        eprintln!(
            "[{}] [setup] WARN sidebar still expanded after attempt {attempt}",
            ts()
        );
    }
    Err("verify failed step 4: sidebar still expanded after 3 collapse attempts".into())
}

fn log_url(label: &str) {
    match browser_state::active_chrome_url() {
        Ok(u) => println!("[{}] [setup] URL {label}: {u}", ts()),
        Err(e) => println!("[{}] [setup] URL {label}: <unread: {e}>", ts()),
    }
}

/// True when Chrome's front window bounds cover the entire primary
/// screen (within a 4-px tolerance per edge). Treats native macOS
/// fullscreen, "Zoomed", and full-edge maximize identically — all are
/// "good enough" for region-based clicks.
/// Mean gray brightness (0–255) of a screen rectangle.
fn patch_brightness(x1: i32, y1: i32, x2: i32, y2: i32) -> f64 {
    match crate::image_matrix::capture::capture_gray_matrix(x1, y1, x2, y2) {
        Ok(c) => {
            let n = c.data.len().max(1);
            c.data.iter().map(|&v| v as u64).sum::<u64>() as f64 / n as f64
        }
        Err(e) => {
            eprintln!(
                "[{}] [setup] patch_brightness({x1},{y1},{x2},{y2}) failed: {e}",
                ts()
            );
            0.0
        }
    }
}

/// Pixel-value variance (theme-agnostic "is something here?" feature).
fn patch_variance(x1: i32, y1: i32, x2: i32, y2: i32) -> f64 {
    let cap = match crate::image_matrix::capture::capture_gray_matrix(x1, y1, x2, y2) {
        Ok(c) => c,
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

/// Open a dropdown by clicking `(hx,hy)` then verifying that the menu
/// region went from "background" to "panel showing." Verified by:
///   - brightness delta ≥ 8.0, OR
///   - variance delta ≥ 50.0,  OR
///   - absolute brightness ≥ 65 (the prior threshold).
///
/// On first miss, scan a 120×40 grid around `(hx,hy)` for the highest-
/// variance patch and retry there. Returns Ok(true) when verified open.
fn open_dropdown_with_verify(
    hx: i32,
    hy: i32,
    header: Region,
    menu: Region,
    label: &str,
) -> Result<bool, Box<dyn Error>> {
    let pre_b = patch_brightness(menu.x1, menu.y1, menu.x2, menu.y2);
    let pre_v = patch_variance(menu.x1, menu.y1, menu.x2, menu.y2);
    println!(
        "[{}] [setup] {label} pre: brightness={pre_b:.1} variance={pre_v:.1}",
        ts()
    );

    if dropdown_search_row_open(menu, label) {
        println!(
            "[{}] [setup] {label} dropdown already open — no header click needed",
            ts()
        );
        return Ok(true);
    }

    // Attempt 1 — click expected position.
    println!("[{}] [setup] {label} click attempt 1 ({hx},{hy})", ts());
    crate::human_simulations::move_mouse_single_click(hx, hy)
        .map_err(|e| format!("{label} click 1: {e}"))?;
    pause_ms(550);
    let _ = menu_changed(menu, pre_b, pre_v, label);
    if dropdown_search_row_open(menu, label) {
        return Ok(true);
    }

    // Attempt 2 — scan WITHIN the header rectangle only.
    eprintln!(
        "[{}] [setup] WARN {label} dropdown not detected open — scanning inside header bounds",
        ts()
    );
    let header_w = (header.x2 - header.x1).max(40);
    let header_h = (header.y2 - header.y1).max(20);
    let scan_w = (header_w / 2 - 4).max(10);
    let scan_h = (header_h / 2 - 2).max(8);

    // The first click may have ACTUALLY opened the dropdown but the
    // brightness/variance Δ was below threshold. The scan-candidate
    // click would then CLOSE it (toggle). To handle this, we try up
    // to 2 more clicks: if click-2 closes it, click-3 re-opens it.
    let mut click_pos = (hx, hy);
    if let Some((sx, sy)) = scan_for_button(hx, hy, scan_w, scan_h, 200.0) {
        click_pos = (sx, sy);
    } else {
        eprintln!(
            "[{}] [setup] {label} scan found no candidate — retrying original",
            ts()
        );
    }

    for attempt in 2..=4 {
        println!(
            "[{}] [setup] {label} click attempt {attempt} ({},{})",
            ts(),
            click_pos.0,
            click_pos.1
        );
        crate::human_simulations::move_mouse_single_click(click_pos.0, click_pos.1)
            .map_err(|e| format!("{label} click {attempt}: {e}"))?;
        pause_ms(650);
        let _ = menu_changed(menu, pre_b, pre_v, label);
        if dropdown_search_row_open(menu, label) {
            return Ok(true);
        }
    }

    // Save debug screenshot on failure.
    let _ = save_screen_png(
        crate::output::writer::screenshot_path(format!("debug_{label}_dropdown_failed.png")),
        header.x1 - 20,
        header.y1 - 10,
        menu.x2 + 20,
        menu.y2 + 10,
    );
    Ok(false)
}

fn menu_changed(menu: Region, pre_b: f64, pre_v: f64, label: &str) -> bool {
    let post_b = patch_brightness(menu.x1, menu.y1, menu.x2, menu.y2);
    let post_v = patch_variance(menu.x1, menu.y1, menu.x2, menu.y2);
    let db = (post_b - pre_b).abs();
    let dv = (post_v - pre_v).abs();
    println!(
        "[{}] [setup] {label} post: brightness={post_b:.1} (Δ{db:.1}) variance={post_v:.1} (Δ{dv:.1})",
        ts()
    );
    db >= 8.0 || dv >= 50.0 || post_b >= 65.0
}

/// Variance-grid scan: find the (x,y) of the highest-variance 12×12 patch
/// within `±half_w × ±half_h` of `(cx,cy)` whose variance ≥ `min_var`.
/// Returns None if nothing in range exceeds the threshold.
fn scan_for_button(cx: i32, cy: i32, half_w: i32, half_h: i32, min_var: f64) -> Option<(i32, i32)> {
    let patch = 12i32;
    let half = patch / 2;
    let step = 14i32;
    let mut best: Option<(f64, i32, i32)> = None;
    let mut y = cy - half_h;
    while y <= cy + half_h {
        let mut x = cx - half_w;
        while x <= cx + half_w {
            let v = patch_variance(x - half, y - half, x + half, y + half);
            if v >= min_var {
                match best {
                    Some((bv, _, _)) if v > bv => best = Some((v, x, y)),
                    None => best = Some((v, x, y)),
                    _ => {}
                }
            }
            x += step;
        }
        y += step;
    }
    best.map(|(_, x, y)| (x, y))
}

/// Find row centers inside `(x1,y1)→(x2,y2)` by detecting horizontal bands
/// of high pixel variance (text/icons) separated by low-variance gaps.
/// Theme-agnostic: works for dark-on-light and light-on-dark UIs.
fn find_row_centers(x1: i32, y1: i32, x2: i32, y2: i32) -> Vec<i32> {
    if x2 - x1 < 5 || y2 - y1 < 8 {
        return vec![];
    }

    let cap = match crate::image_matrix::capture::capture_gray_matrix(x1, y1, x2, y2) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[{}] [setup] find_row_centers capture failed: {e}", ts());
            return vec![];
        }
    };
    let h = cap.height;
    let w = cap.width;
    if h < 8 || w < 5 {
        return vec![];
    }

    // Per-row variance.
    let mut prof = vec![0.0f64; h];
    for y in 0..h {
        let row = &cap.data[y * w..(y + 1) * w];
        let mean: f64 = row.iter().map(|&v| v as f64).sum::<f64>() / w as f64;
        prof[y] = row.iter().map(|&v| (v as f64 - mean).powi(2)).sum::<f64>() / w as f64;
    }

    let max_v: f64 = prof.iter().cloned().fold(0.0_f64, f64::max);
    if max_v < 5.0 {
        return vec![];
    } // empty / flat region
    let thresh = (max_v * 0.30).max(8.0);

    // Contiguous bands above threshold; min thickness 6 px.
    let mut bands: Vec<(usize, usize)> = vec![];
    let mut start: Option<usize> = None;
    for y in 0..h {
        if prof[y] >= thresh {
            if start.is_none() {
                start = Some(y);
            }
        } else if let Some(s) = start.take() {
            if y.saturating_sub(s) >= 6 {
                bands.push((s, y - 1));
            }
        }
    }
    if let Some(s) = start {
        let last = h - 1;
        if last.saturating_sub(s) >= 6 {
            bands.push((s, last));
        }
    }

    let raw: Vec<i32> = bands
        .iter()
        .map(|(s, e)| y1 + ((s + e) / 2) as i32)
        .collect();
    cluster_close_peaks(&raw, 28)
}

/// Merge peaks closer than `min_gap` into a single peak (their mean).
/// Arena's dropdown rows show title + subtitle, producing two variance
/// bands per row — clustering reduces those to one center per row.
fn cluster_close_peaks(peaks: &[i32], min_gap: i32) -> Vec<i32> {
    if peaks.is_empty() {
        return vec![];
    }
    let mut out: Vec<i32> = Vec::new();
    let mut group: Vec<i32> = vec![peaks[0]];
    for &p in &peaks[1..] {
        let last = *group.last().unwrap();
        if p - last < min_gap {
            group.push(p);
        } else {
            let sum: i32 = group.iter().sum();
            out.push(sum / group.len() as i32);
            group = vec![p];
        }
    }
    let sum: i32 = group.iter().sum();
    out.push(sum / group.len() as i32);
    out
}
