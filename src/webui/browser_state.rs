//! Cross-platform reads of the **browser's** state — frontmost app name,
//! the active Chrome tab's URL, page title, and tab navigation.
//!
//! Why is this its own module rather than part of `human_simulations`?
//! These calls are *browser-aware* (they ask "what's Chrome showing?"),
//! whereas `human_simulations` simulates raw input. Keeping them
//! separate means the watchdog and recovery can ask sensible questions
//! ("is Chrome showing arena.ai?") without dragging input simulation
//! into the read path.
//!
//! Implementation strategy:
//!
//! - **macOS** uses `osascript` (AppleScript). Zero extra dependencies.
//!   AppleScript can enumerate every Chrome window + tab, return URLs,
//!   and switch to a specific tab — exactly what we need to avoid
//!   clobbering the user's other tabs.
//! - **Windows** without UI Automation can't enumerate tabs. We expose
//!   the same trait but tab-discovery returns `Ok(false)` so the caller
//!   falls through to opening a new tab (always works, doesn't clobber).
//! - **Other** platforms return `Err`; the watchdog interprets that as
//!   "not enough information" and stays out of the way.

// ── Tab handle returned by enumeration ──────────────────────────────────────

#[derive(Debug, Clone)]
pub struct TabRef {
    pub window_index: usize,
    pub tab_index: usize,
    pub url: String,
    pub title: String,
}

// ── macOS impl ──────────────────────────────────────────────────────────────

#[cfg(target_os = "macos")]
mod imp {
    use std::process::Command;

    use super::TabRef;

    pub fn frontmost_app_name() -> Result<String, String> {
        run_osascript(
            "tell application \"System Events\" to \
             get name of first application process whose frontmost is true",
        )
    }

    pub fn focus_app_by_name(name_substr: &str) -> Result<bool, String> {
        let candidates = candidate_app_names(name_substr);
        for app in candidates {
            let script = format!("tell application \"{app}\" to activate");
            if Command::new("osascript")
                .arg("-e")
                .arg(&script)
                .status()
                .map(|s| s.success())
                .unwrap_or(false)
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub fn active_chrome_url() -> Result<String, String> {
        run_osascript(
            "tell application \"Google Chrome\" to \
             get URL of active tab of front window",
        )
    }

    pub fn active_chrome_title() -> Result<String, String> {
        run_osascript(
            "tell application \"Google Chrome\" to \
             get title of active tab of front window",
        )
    }

    /// Enumerate every tab in every Chrome window.
    ///
    /// Returns the lot — order is window-major, tab-major. Empty list
    /// means Chrome is running but has no windows; `Err` means Chrome
    /// isn't reachable.
    pub fn list_chrome_tabs() -> Result<Vec<TabRef>, String> {
        // The script emits one record per tab using ASCII unit/record
        // separators that won't appear in URLs or titles, so we can
        // safely split the output without quoting heroics.
        let script = r#"
            set output to ""
            tell application "Google Chrome"
                repeat with wi from 1 to count windows
                    set theWindow to window wi
                    repeat with ti from 1 to count tabs of theWindow
                        set theTab to tab ti of theWindow
                        set output to output & wi & "|" & ti & "|" & (URL of theTab) & "|" & (title of theTab) & linefeed
                    end repeat
                end repeat
            end tell
            return output
        "#;
        let raw = run_osascript(script)?;
        let mut tabs = Vec::new();
        for line in raw.lines() {
            let parts: Vec<&str> = line.splitn(4, '|').collect();
            if parts.len() < 4 {
                continue;
            }
            if let (Ok(wi), Ok(ti)) = (parts[0].parse::<usize>(), parts[1].parse::<usize>()) {
                tabs.push(TabRef {
                    window_index: wi,
                    tab_index: ti,
                    url: parts[2].to_string(),
                    title: parts[3].to_string(),
                });
            }
        }
        Ok(tabs)
    }

    /// Find the first Chrome tab on an Arena **chat** page (not
    /// leaderboard, help, or articles, and not `lmarena.ai` which is a
    /// different product) and bring it to front.
    ///
    /// Returns `Ok(true)` with the URL of the focused tab on success,
    /// `Ok(false)` if no usable chat tab exists, `Err` only on
    /// osascript failure.
    pub fn focus_chrome_arena_chat_tab() -> Result<bool, String> {
        // Match `://arena.ai` exactly so `lmarena.ai` is NOT matched.
        // Filter out non-chat arena pages (leaderboard, help, articles,
        // help.arena.*, blog.arena.*).
        let script = r#"
            tell application "Google Chrome"
                set foundWin to missing value
                set foundTab to 0
                set foundUrl to ""
                repeat with wi from 1 to count windows
                    set theWindow to window wi
                    repeat with ti from 1 to count tabs of theWindow
                        set u to URL of tab ti of theWindow
                        set isArena to (u contains "://arena.ai")
                        set badPath to (u contains "/leaderboard") or (u contains "/help") or (u contains "/articles/")
                        set badHost to (u contains "help.arena.") or (u contains "blog.arena.")
                        if isArena and (not badPath) and (not badHost) then
                            set foundWin to theWindow
                            set foundTab to ti
                            set foundUrl to u
                            exit repeat
                        end if
                    end repeat
                    if foundWin is not missing value then exit repeat
                end repeat
                if foundWin is missing value then
                    return "not_found"
                end if
                set active tab index of foundWin to foundTab
                set index of foundWin to 1
                activate
                return "ok|" & foundUrl
            end tell
        "#;
        let out = run_osascript(script)?;
        let trimmed = out.trim();
        if trimmed == "not_found" {
            return Ok(false);
        }
        if let Some(url) = trimmed.strip_prefix("ok|") {
            eprintln!("[browser_state] focused arena chat tab: {url}");
            return Ok(true);
        }
        Err(format!("unexpected osascript output: {trimmed}"))
    }

    /// Bring the first Chrome tab whose URL contains `url_substr` to
    /// front (window forward + active tab set + Chrome activated).
    ///
    /// Returns `Ok(true)` if a matching tab existed and was focused.
    /// Returns `Ok(false)` if no tab matched — caller should fall through
    /// to `open_new_browser_tab`. Returns `Err` only on osascript failure.
    pub fn focus_chrome_tab_by_url(url_substr: &str) -> Result<bool, String> {
        // AppleScript `contains` is case-insensitive by default, which is
        // what we want. Escape any double quotes the caller passed.
        let escaped = url_substr.replace('"', "\\\"");
        let script = format!(
            r#"
            tell application "Google Chrome"
                set foundWin to missing value
                set foundTab to 0
                repeat with wi from 1 to count windows
                    set theWindow to window wi
                    repeat with ti from 1 to count tabs of theWindow
                        if URL of tab ti of theWindow contains "{escaped}" then
                            set foundWin to theWindow
                            set foundTab to ti
                            exit repeat
                        end if
                    end repeat
                    if foundWin is not missing value then exit repeat
                end repeat
                if foundWin is missing value then
                    return "not_found"
                end if
                set active tab index of foundWin to foundTab
                set index of foundWin to 1
                activate
                return "ok"
            end tell
        "#
        );
        let out = run_osascript(&script)?;
        Ok(out.trim() == "ok")
    }

    fn run_osascript(script: &str) -> Result<String, String> {
        let out = Command::new("osascript")
            .arg("-e")
            .arg(script)
            .output()
            .map_err(|e| format!("osascript spawn failed: {e}"))?;
        if !out.status.success() {
            return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
        }
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    }

    /// Returns `true` if Chrome's front window is in **native macOS
    /// fullscreen** (the green-traffic-light fullscreen, not the
    /// presentation/zoom mode).
    ///
    /// Implementation reads the `AXFullScreen` accessibility attribute
    /// via System Events. Requires Accessibility permission, which the
    /// project already needs for `enigo`.
    pub fn is_chrome_fullscreen() -> Result<bool, String> {
        let out = run_osascript(
            r#"
            tell application "System Events"
                tell process "Google Chrome"
                    if (count of windows) is 0 then
                        return "no_window"
                    end if
                    try
                        return (value of attribute "AXFullScreen" of window 1) as text
                    on error
                        return "unknown"
                    end try
                end tell
            end tell
        "#,
        )?;
        match out.as_str() {
            "true" => Ok(true),
            "false" => Ok(false),
            "no_window" => Err("no_window".into()),
            "unknown" => Err("unknown".into()),
            other => Err(format!("unexpected AXFullScreen value: {other}")),
        }
    }

    /// Dump every Chrome window with its bounds and active-tab URL.
    /// Returns `(window_index, x1, y1, x2, y2, active_url)` per window.
    /// Lets the caller diagnose multi-window / Stage-Manager confusion
    /// when "front window" doesn't match what the user perceives as the
    /// active Chrome window.
    pub fn chrome_list_windows() -> Result<Vec<(i32, i32, i32, i32, i32, String)>, String> {
        // Per-window record fields are joined with `|`; records are
        // separated by linefeed. Same scheme as `list_chrome_tabs`.
        let raw = run_osascript(
            r#"
            set output to ""
            tell application "Google Chrome"
                repeat with wi from 1 to count windows
                    set theWin to window wi
                    set b to bounds of theWin
                    set u to ""
                    try
                        set u to URL of active tab of theWin
                    end try
                    set output to output & wi & "|" & ((item 1 of b) as text) & "|" & ((item 2 of b) as text) & "|" & ((item 3 of b) as text) & "|" & ((item 4 of b) as text) & "|" & u & linefeed
                end repeat
            end tell
            return output
        "#,
        )?;
        let mut out = Vec::new();
        for line in raw.lines() {
            let parts: Vec<&str> = line.splitn(6, '|').collect();
            if parts.len() != 6 {
                continue;
            }
            let idx: i32 = parts[0].parse().unwrap_or(-1);
            let x1: i32 = parts[1].parse().unwrap_or(0);
            let y1: i32 = parts[2].parse().unwrap_or(0);
            let x2: i32 = parts[3].parse().unwrap_or(0);
            let y2: i32 = parts[4].parse().unwrap_or(0);
            out.push((idx, x1, y1, x2, y2, parts[5].to_string()));
        }
        Ok(out)
    }

    /// Front Chrome window bounds as `(x1, y1, x2, y2)` in screen logical
    /// pixels. Works for native fullscreen, "Zoomed" maximize, AND ordinary
    /// windowed mode — caller compares to `primary_screen_size()` to decide
    /// "covers screen?" rather than relying on `AXFullScreen`.
    pub fn chrome_window_bounds() -> Result<(i32, i32, i32, i32), String> {
        let raw = run_osascript(
            r#"
            tell application "Google Chrome"
                if (count of windows) is 0 then return "no_window"
                set b to bounds of front window
                return ((item 1 of b) as text) & "," & ((item 2 of b) as text) & "," & ((item 3 of b) as text) & "," & ((item 4 of b) as text)
            end tell
        "#,
        )?;
        if raw == "no_window" {
            return Err("no_window".into());
        }
        let parts: Vec<&str> = raw.split(',').map(|s| s.trim()).collect();
        if parts.len() != 4 {
            return Err(format!("unexpected bounds output: '{raw}'"));
        }
        let nums: Result<Vec<i32>, _> = parts.iter().map(|s| s.parse::<i32>()).collect();
        let v = nums.map_err(|e| format!("bounds parse: {e} from '{raw}'"))?;
        Ok((v[0], v[1], v[2], v[3]))
    }

    /// Force Chrome's front window into native fullscreen. Idempotent —
    /// setting to `true` while already fullscreen is a no-op.
    pub fn set_chrome_fullscreen(on: bool) -> Result<(), String> {
        let val = if on { "true" } else { "false" };
        run_osascript(&format!(
            r#"
            tell application "System Events"
                tell process "Google Chrome"
                    if (count of windows) is 0 then
                        return "no_window"
                    end if
                    set value of attribute "AXFullScreen" of window 1 to {val}
                    return "ok"
                end tell
            end tell
        "#
        ))?;
        Ok(())
    }

    /// Navigate the **current** active Chrome tab to `url`, reusing the
    /// existing tab (no new tab opened). Prefer this over
    /// `open_new_browser_tab` when you want to stay on the same tab —
    /// e.g. resetting Arena to the home page between runs.
    pub fn navigate_current_tab(url: &str) -> Result<(), String> {
        let escaped = url.replace('"', "\\\"");
        run_osascript(&format!(
            "tell application \"Google Chrome\" to \
             set URL of active tab of front window to \"{escaped}\""
        ))?;
        Ok(())
    }

    fn candidate_app_names(hint: &str) -> Vec<&'static str> {
        let h = hint.to_lowercase();
        if h.contains("chrome") {
            return vec!["Google Chrome"];
        }
        if h.contains("safari") {
            return vec!["Safari"];
        }
        if h.contains("firefox") {
            return vec!["Firefox"];
        }
        if h.contains("edge") {
            return vec!["Microsoft Edge"];
        }
        if h.contains("brave") {
            return vec!["Brave Browser"];
        }
        if h.contains("arc") {
            return vec!["Arc"];
        }
        vec!["Google Chrome"]
    }
}

// ── Windows impl ────────────────────────────────────────────────────────────

#[cfg(target_os = "windows")]
mod imp {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    use std::ptr;

    use winapi::shared::windef::HWND;
    use winapi::um::winuser::{
        FindWindowW, GetForegroundWindow, GetWindowTextLengthW, GetWindowTextW, SW_RESTORE,
        SetForegroundWindow, ShowWindow,
    };

    use super::TabRef;

    pub fn frontmost_app_name() -> Result<String, String> {
        let hwnd = unsafe { GetForegroundWindow() };
        if hwnd.is_null() {
            return Err("GetForegroundWindow returned null".into());
        }
        window_text(hwnd)
    }

    pub fn focus_app_by_name(name_substr: &str) -> Result<bool, String> {
        let wide: Vec<u16> = name_substr
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let hwnd = unsafe { FindWindowW(ptr::null(), wide.as_ptr()) };
        if hwnd.is_null() {
            return Ok(false);
        }
        unsafe {
            ShowWindow(hwnd, SW_RESTORE);
            SetForegroundWindow(hwnd);
        }
        Ok(true)
    }

    pub fn active_chrome_url() -> Result<String, String> {
        Err("active_chrome_url: unsupported on Windows without a UIA bridge".into())
    }

    pub fn active_chrome_title() -> Result<String, String> {
        // Chrome's window title is "<active tab title> - Google Chrome".
        frontmost_app_name()
    }

    pub fn list_chrome_tabs() -> Result<Vec<TabRef>, String> {
        // Without UIA we can't enumerate Chrome's tabs. The recovery
        // dispatcher treats this as "not discoverable" and falls back to
        // opening a new tab (which always works).
        Err("list_chrome_tabs: unsupported on Windows without a UIA bridge".into())
    }

    pub fn focus_chrome_tab_by_url(_url_substr: &str) -> Result<bool, String> {
        // Same reason — see `list_chrome_tabs`. Returning Ok(false)
        // (rather than Err) keeps the caller's fall-through logic clean:
        // "couldn't find an existing tab → open a new one".
        Ok(false)
    }

    pub fn focus_chrome_arena_chat_tab() -> Result<bool, String> {
        Ok(false)
    }

    pub fn navigate_current_tab(_url: &str) -> Result<(), String> {
        Err("navigate_current_tab: not supported on Windows without a UIA bridge".into())
    }

    pub fn is_chrome_fullscreen() -> Result<bool, String> {
        // Best Windows analogue: window state == maximised. Without UIA
        // we can't detect borderless fullscreen reliably; report Err so
        // the watchdog falls back to no-op rather than fighting Windows.
        Err("is_chrome_fullscreen: not supported on Windows without UIA".into())
    }

    pub fn set_chrome_fullscreen(_on: bool) -> Result<(), String> {
        // F11 toggles Chrome's fullscreen on Windows; pressing it
        // unconditionally would un-fullscreen if already fullscreen.
        // Implement properly when we have a fullscreen-state read path.
        Err("set_chrome_fullscreen: not supported on Windows yet".into())
    }

    pub fn chrome_window_bounds() -> Result<(i32, i32, i32, i32), String> {
        Err("chrome_window_bounds: not supported on Windows without UIA".into())
    }

    pub fn chrome_list_windows() -> Result<Vec<(i32, i32, i32, i32, i32, String)>, String> {
        Err("chrome_list_windows: not supported on Windows without UIA".into())
    }

    fn window_text(hwnd: HWND) -> Result<String, String> {
        let len = unsafe { GetWindowTextLengthW(hwnd) };
        if len <= 0 {
            return Ok(String::new());
        }
        let mut buf: Vec<u16> = vec![0; (len + 1) as usize];
        let copied = unsafe { GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32) };
        if copied <= 0 {
            return Err("GetWindowTextW returned no characters".into());
        }
        buf.truncate(copied as usize);
        Ok(OsString::from_wide(&buf).to_string_lossy().into_owned())
    }
}

// ── Unsupported platforms ───────────────────────────────────────────────────

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod imp {
    use super::TabRef;

    pub fn frontmost_app_name() -> Result<String, String> {
        Err("frontmost_app_name: not supported on this platform".into())
    }
    pub fn focus_app_by_name(_n: &str) -> Result<bool, String> {
        Err("focus_app_by_name: not supported on this platform".into())
    }
    pub fn active_chrome_url() -> Result<String, String> {
        Err("active_chrome_url: not supported on this platform".into())
    }
    pub fn active_chrome_title() -> Result<String, String> {
        Err("active_chrome_title: not supported on this platform".into())
    }
    pub fn list_chrome_tabs() -> Result<Vec<TabRef>, String> {
        Err("list_chrome_tabs: not supported on this platform".into())
    }
    pub fn focus_chrome_tab_by_url(_s: &str) -> Result<bool, String> {
        Ok(false)
    }
    pub fn focus_chrome_arena_chat_tab() -> Result<bool, String> {
        Ok(false)
    }
    pub fn navigate_current_tab(_url: &str) -> Result<(), String> {
        Err("navigate_current_tab: not supported on this platform".into())
    }
    pub fn is_chrome_fullscreen() -> Result<bool, String> {
        Err("is_chrome_fullscreen: not supported on this platform".into())
    }
    pub fn set_chrome_fullscreen(_on: bool) -> Result<(), String> {
        Err("set_chrome_fullscreen: not supported on this platform".into())
    }
    pub fn chrome_window_bounds() -> Result<(i32, i32, i32, i32), String> {
        Err("chrome_window_bounds: not supported on this platform".into())
    }
    pub fn chrome_list_windows() -> Result<Vec<(i32, i32, i32, i32, i32, String)>, String> {
        Err("chrome_list_windows: not supported on this platform".into())
    }
}

// ── Public re-exports ───────────────────────────────────────────────────────

pub use imp::{
    active_chrome_title, active_chrome_url, chrome_list_windows, chrome_window_bounds,
    focus_app_by_name, focus_chrome_arena_chat_tab, focus_chrome_tab_by_url, frontmost_app_name,
    is_chrome_fullscreen, list_chrome_tabs, navigate_current_tab, set_chrome_fullscreen,
};

// ── Cross-platform: open a new tab and navigate ─────────────────────────────

/// Open a new browser tab and navigate to `url`.
///
/// Implementation: send `Cmd+T` (mac) / `Ctrl+T` (win/linux) to whatever
/// browser is in focus, type the URL, press Enter. Doesn't clobber the
/// current tab — that's the whole point. Caller must ensure the browser
/// is the foreground app first (`focus_app_by_name`).
pub fn open_new_browser_tab(url: &str) -> Result<(), String> {
    use crate::human_simulations as hs;
    let primary = hs::primary_modifier();
    hs::press_key(&format!("{primary}+t"))?;
    std::thread::sleep(std::time::Duration::from_millis(300));
    hs::type_text(url)?;
    std::thread::sleep(std::time::Duration::from_millis(80));
    hs::press_key("return")?;
    std::thread::sleep(std::time::Duration::from_millis(1500));
    Ok(())
}
