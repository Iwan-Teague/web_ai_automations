//! One-shot DeepSeek setup before the FSM starts.

use std::error::Error;

use crate::session::SessionConfig;
use crate::webui::adapter::WebUiAdapter;
use crate::webui::browser_state;
use crate::webui::deepseek::DeepSeekAdapter;
use crate::webui::deepseek::{DEEPSEEK_HOME_URL, DEEPSEEK_URL_SUBSTR};

fn ts() -> String {
    chrono::Local::now().format("%H:%M:%S%.3f").to_string()
}

#[derive(Debug, Default)]
pub struct SetupReport {
    pub browser_focused: bool,
    pub navigated_to_deepseek: bool,
    pub reused_existing_tab: bool,
    pub forced_fullscreen: bool,
    pub clicked_new_chat: bool,
    pub model_selected: bool,
    pub deep_thinking_set: bool,
    pub smart_search_set: bool,
}

pub fn run(adapter: &DeepSeekAdapter, cfg: &SessionConfig) -> Result<SetupReport, Box<dyn Error>> {
    let mut report = SetupReport::default();

    println!("[{}] [deepseek-setup] focus Chrome", ts());
    let mut focused = false;
    for attempt in 1..=3 {
        let _ = browser_state::focus_app_by_name("Chrome");
        pause_ms(500);
        let app = browser_state::frontmost_app_name().unwrap_or_default();
        println!(
            "[{}] [deepseek-setup] focus attempt {attempt}: frontmost='{app}'",
            ts()
        );
        if app.to_lowercase().contains("chrome") {
            focused = true;
            break;
        }
    }
    if !focused {
        return Err("DeepSeek setup failed: Chrome did not become frontmost".into());
    }
    report.browser_focused = true;

    match browser_state::focus_chrome_tab_by_url(DEEPSEEK_URL_SUBSTR) {
        Ok(true) => {
            report.reused_existing_tab = true;
            pause_ms(500);
        }
        Ok(false) => {
            println!(
                "[{}] [deepseek-setup] opening new tab → {DEEPSEEK_HOME_URL}",
                ts()
            );
            browser_state::open_new_browser_tab(DEEPSEEK_HOME_URL)
                .map_err(|e| format!("open_new_browser_tab: {e}"))?;
            pause_ms(2200);
        }
        Err(e) => {
            eprintln!("[{}] [deepseek-setup] tab focus failed: {e}", ts());
            browser_state::open_new_browser_tab(DEEPSEEK_HOME_URL)
                .map_err(|e| format!("open_new_browser_tab: {e}"))?;
            pause_ms(2200);
        }
    }
    let url = browser_state::active_chrome_url().unwrap_or_default();
    if !url.contains(DEEPSEEK_URL_SUBSTR) {
        browser_state::navigate_current_tab(DEEPSEEK_HOME_URL)
            .map_err(|e| format!("navigate_current_tab: {e}"))?;
        pause_ms(1800);
    }
    let url = browser_state::active_chrome_url().unwrap_or_default();
    if !url.contains(DEEPSEEK_URL_SUBSTR) {
        return Err(format!("DeepSeek setup failed: current URL '{url}' is not DeepSeek").into());
    }
    report.navigated_to_deepseek = true;

    // ── Step 1: fullscreen ──────────────────────────────────────────────
    ensure_full_width()?;
    report.forced_fullscreen = true;

    // Close any leftover doc-preview side panel BEFORE any region-based
    // scan runs. The panel shifts the composer layout, so every button
    // location is wrong while it's open — the new-chat-button click in
    // Step 2 would miss.
    //
    // Two-pronged: press Escape unconditionally (a no-op when nothing's
    // open, but reliably closes DeepSeek's side panels without needing a
    // template asset on disk), then run the template-based detector for
    // panels Escape doesn't dismiss.
    if let Err(e) = crate::human_simulations::press_key("escape") {
        eprintln!("[{}] [deepseek-setup] escape press failed: {e}", ts());
    }
    pause_ms(300);
    if adapter.dismiss_doc_panel_if_open() {
        println!(
            "[{}] [deepseek-setup] closed leftover doc-preview panel",
            ts()
        );
        pause_ms(400);
    }

    // Ensure the left chat-history sidebar is OPEN. The right doc-preview
    // panel and the left history panel are mutually exclusive, so this
    // runs strictly AFTER the right-panel dismiss. When the sidebar is
    // collapsed the new-chat button moves into a compact pill at the top
    // of the viewport — start_new_chat would fail to find its template.
    if adapter.ensure_left_sidebar_open() {
        println!("[{}] [deepseek-setup] left sidebar open", ts());
        pause_ms(400);
    } else {
        eprintln!(
            "[{}] [deepseek-setup] could not confirm left sidebar open — proceeding anyway",
            ts()
        );
    }

    // ── Step 2: new chat button ─────────────────────────────────────────
    adapter.start_new_chat()?;
    report.clicked_new_chat = true;
    pause_ms(700);

    // ── Step 3: reload page (always, to flush any unsent uploads) ───────
    //
    // DeepSeek caches uploaded-but-unsent files server-side and rebinds
    // them as chips on the next composer load. Detection-based clearing
    // (template scan + JS hover-click) is unreliable. A full reload to
    // the home URL is the only deterministic way to reach an empty
    // composer. Safe here because we have no in-flight conversation —
    // setup runs before the first prompt.
    println!(
        "[{}] [deepseek-setup] reloading page to ensure clean composer",
        ts()
    );
    browser_state::navigate_current_tab(DEEPSEEK_HOME_URL)
        .map_err(|e| format!("reload navigate_current_tab: {e}"))?;
    pause_ms(2200);
    adapter.start_new_chat()?;
    pause_ms(900);

    // ── Step 4: verify prompt box empty ─────────────────────────────────
    //
    // Uses the existing chip-icon template (assets/macos/deepseek/
    // templates/attach_chip_icon_dark.png) — chip presence is the only
    // visible indicator the composer is non-empty at this stage.
    if adapter.attachment_present() {
        return Err(
            "DeepSeek setup failed: chip still present after reload — composer not empty".into(),
        );
    }
    println!("[{}] [deepseek-setup] composer verified empty", ts());

    // ── Step 5: select mode (Instant / Expert) ──────────────────────────
    adapter.select_model(cfg.deepseek_model)?;
    report.model_selected = true;

    // ── Step 6: deep-thinking + smart-search toggles ────────────────────
    adapter.set_deep_thinking(cfg.deepseek_deep_thinking)?;
    report.deep_thinking_set = true;
    adapter.set_smart_search(cfg.deepseek_smart_search)?;
    report.smart_search_set = true;

    // Steps 7–9 (upload doc, paste prompt, send) run after setup returns:
    // upload via task_runner::prepare_site → adapter.upload_file, then
    // paste/send via the FSM's submit_prompt.

    println!("[{}] [deepseek-setup] report: {report:?}", ts());
    let _ = adapter.detect_state();
    Ok(report)
}

fn ensure_full_width() -> Result<(), Box<dyn Error>> {
    let (sw, _) = crate::image_matrix::capture::primary_screen_size()
        .map_err(|e| format!("primary_screen_size: {e}"))?;
    let (x1, _, x2, _) =
        browser_state::chrome_window_bounds().map_err(|e| format!("chrome_window_bounds: {e}"))?;
    let width = x2 - x1;
    if width >= sw - 8 {
        return Ok(());
    }
    browser_state::set_chrome_fullscreen(true)
        .map_err(|e| format!("set_chrome_fullscreen: {e}"))?;
    pause_ms(1200);
    let (x1, _, x2, _) =
        browser_state::chrome_window_bounds().map_err(|e| format!("chrome_window_bounds: {e}"))?;
    let width = x2 - x1;
    if width >= sw - 8 {
        Ok(())
    } else {
        Err(format!("DeepSeek setup failed: Chrome width {width} < screen width {sw}").into())
    }
}

fn pause_ms(ms: u64) {
    std::thread::sleep(std::time::Duration::from_millis(ms));
}
