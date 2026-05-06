//! Cross-platform input + clipboard primitives.
//!
//! Built on top of:
//! - [`enigo`]   — mouse and keyboard, supports macOS / Windows / Linux.
//! - [`arboard`] — clipboard read/write, same coverage.
//!
//! macOS notes (read this once, save yourself half an hour):
//!
//! 1. The first call that controls input will make macOS prompt for
//!    **Accessibility** permission. Grant it under
//!    `System Settings → Privacy & Security → Accessibility`. Until you do,
//!    `enigo` silently no-ops on Mac.
//! 2. Screen capture (used elsewhere) needs **Screen Recording** permission
//!    in the same panel.
//! 3. Mouse coordinates are in *logical* pixels (Cocoa coordinates).
//!    On Retina, the screen reports 1512×982 logical for a 3024×1964
//!    panel. Click `(750, 500)` lands at the visible centre regardless of
//!    DPI — that's what we want.
//!
//! Windows notes:
//!
//! 1. Mouse coordinates are physical pixels, but if the process is DPI-
//!    aware, `enigo` translates correctly. The `screenshots` crate runs
//!    DPI-aware by default, so the same `(x, y)` works for both capture
//!    and click.

use std::sync::atomic::{AtomicI32, Ordering};
use std::time::Duration;

use arboard::Clipboard;
use enigo::{Enigo, Key, KeyboardControllable, MouseButton, MouseControllable};
use rand::Rng;

// ── Mouse ───────────────────────────────────────────────────────────────────

static LAST_BOT_MOUSE_X: AtomicI32 = AtomicI32::new(i32::MIN);
static LAST_BOT_MOUSE_Y: AtomicI32 = AtomicI32::new(i32::MIN);

/// Move the mouse to absolute screen coordinates without clicking.
pub fn move_mouse(x: i32, y: i32) -> Result<(), String> {
    let mut enigo = Enigo::new();
    move_mouse_humanized(&mut enigo, x, y);
    Ok(())
}

/// Move to `(x, y)` and left-click once.
pub fn move_mouse_single_click(x: i32, y: i32) -> Result<(), String> {
    let mut enigo = Enigo::new();
    move_mouse_humanized(&mut enigo, x, y);
    click_current_position(&mut enigo, MouseButton::Left);
    Ok(())
}

/// Left-click at the **current** cursor position without moving the mouse.
///
/// Use this after `move_mouse` when you need a hover-then-click sequence —
/// keeps an open dropdown stable by not re-triggering the move event.
pub fn left_click() -> Result<(), String> {
    let mut enigo = Enigo::new();
    click_current_position(&mut enigo, MouseButton::Left);
    Ok(())
}

/// Scroll the mouse wheel. Positive = down, negative = up.
pub fn scroll(length: i32) -> Result<(), String> {
    let mut enigo = Enigo::new();
    enigo.mouse_scroll_y(length);
    Ok(())
}

/// Read the current cursor position.
///
/// `enigo` 0.1 doesn't expose this, so we go direct on platforms we can.
/// On unsupported platforms returns `Err` and the watchdog's user-activity
/// invariant degrades to "always Ok" rather than spuriously firing.
#[cfg(target_os = "macos")]
pub fn cursor_position() -> Result<(i32, i32), String> {
    let enigo = Enigo::new();
    Ok(enigo.mouse_location())
}

#[cfg(target_os = "windows")]
pub fn cursor_position() -> Result<(i32, i32), String> {
    use winapi::shared::windef::POINT;
    use winapi::um::winuser::GetCursorPos;
    let mut p = POINT { x: 0, y: 0 };
    // SAFETY: GetCursorPos writes into the POINT we own, returns BOOL.
    let ok = unsafe { GetCursorPos(&mut p as *mut POINT) };
    if ok == 0 {
        return Err("GetCursorPos failed".into());
    }
    Ok((p.x, p.y))
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn cursor_position() -> Result<(i32, i32), String> {
    let enigo = Enigo::new();
    Ok(enigo.mouse_location())
}

pub fn is_last_bot_mouse_position(pos: (i32, i32)) -> bool {
    let x = LAST_BOT_MOUSE_X.load(Ordering::Relaxed);
    let y = LAST_BOT_MOUSE_Y.load(Ordering::Relaxed);
    (pos.0 - x).abs() <= 3 && (pos.1 - y).abs() <= 3
}

fn move_mouse_humanized(enigo: &mut Enigo, target_x: i32, target_y: i32) {
    let (start_x, start_y) = enigo.mouse_location();
    let dx = target_x - start_x;
    let dy = target_y - start_y;
    let distance = ((dx * dx + dy * dy) as f64).sqrt();

    if distance < 2.0 {
        enigo.mouse_move_to(target_x, target_y);
        return;
    }

    let mut rng = rand::thread_rng();
    let duration_ms = movement_duration_ms(distance, &mut rng);
    let steps = (duration_ms / rng.gen_range(8..=14)).clamp(8, 90);
    let curve = rng.gen_range(-0.55..=0.55) * distance.min(900.0) / 900.0;
    let unit_x = dx as f64 / distance;
    let unit_y = dy as f64 / distance;
    let perp_x = -unit_y;
    let perp_y = unit_x;
    let jitter_px = (distance / rng.gen_range(60.0..=130.0)).clamp(0.8, 11.0);
    let wobble_phase = rng.gen_range(0.0..=std::f64::consts::TAU);
    let wobble_cycles = rng.gen_range(1.2..=3.4);
    let accel_shape = rng.gen_range(2.2..=4.8);
    let approach_shape = rng.gen_range(1.6..=4.4);

    let mut last = (start_x, start_y);
    for i in 1..=steps {
        let t = i as f64 / steps as f64;
        let eased = variable_ease_out(t, accel_shape);
        let settle = 1.0 - eased;
        let base_x = start_x as f64 + dx as f64 * eased;
        let base_y = start_y as f64 + dy as f64 * eased;
        let arc = (std::f64::consts::PI * t).sin() * curve * 34.0;
        let wobble = (t * wobble_cycles * std::f64::consts::TAU + wobble_phase).sin()
            * jitter_px
            * settle
            * rng.gen_range(0.15..=0.55);
        let jitter = if i == steps {
            0.0
        } else {
            rng.gen_range(-jitter_px..=jitter_px) * settle
        };
        let forward_noise = if i == steps {
            0.0
        } else {
            rng.gen_range(-1.5..=1.5) * settle
        };
        let next_x =
            (base_x + perp_x * (arc + jitter + wobble) + unit_x * forward_noise).round() as i32;
        let next_y =
            (base_y + perp_y * (arc + jitter + wobble) + unit_y * forward_noise).round() as i32;
        let next = if i == steps {
            (target_x, target_y)
        } else {
            (next_x, next_y)
        };

        if next != last {
            enigo.mouse_move_to(next.0, next.1);
            last = next;
        }

        let base_sleep = duration_ms as f64 / steps as f64;
        let approach_slowdown = 1.0 + t.powf(approach_shape) * rng.gen_range(0.75..=1.95);
        let sleep_ms = (base_sleep * approach_slowdown * rng.gen_range(0.72..=1.22))
            .round()
            .clamp(3.0, 35.0) as u64;
        std::thread::sleep(Duration::from_millis(sleep_ms));
    }

    if rng.gen_bool(0.18) && distance > 80.0 {
        let overshoot_x = target_x + rng.gen_range(-2..=2);
        let overshoot_y = target_y + rng.gen_range(-2..=2);
        enigo.mouse_move_to(overshoot_x, overshoot_y);
        std::thread::sleep(Duration::from_millis(rng.gen_range(18..=45)));
        enigo.mouse_move_to(target_x, target_y);
    }
    note_bot_mouse_position(target_x, target_y);
}

fn movement_duration_ms(distance: f64, rng: &mut impl Rng) -> u64 {
    let quick_start = distance / rng.gen_range(2.4..=3.4);
    let settle = (distance.sqrt() * rng.gen_range(5.0..=8.0)).min(260.0);
    (quick_start + settle + rng.gen_range(20.0..=95.0))
        .round()
        .clamp(90.0, 900.0) as u64
}

fn variable_ease_out(t: f64, shape: f64) -> f64 {
    1.0 - (1.0 - t).powf(shape)
}

fn click_current_position(enigo: &mut Enigo, button: MouseButton) {
    let mut rng = rand::thread_rng();
    std::thread::sleep(Duration::from_millis(rng.gen_range(45..=145)));
    enigo.mouse_down(button);
    std::thread::sleep(Duration::from_millis(rng.gen_range(35..=90)));
    enigo.mouse_up(button);
}

fn note_bot_mouse_position(x: i32, y: i32) {
    LAST_BOT_MOUSE_X.store(x, Ordering::Relaxed);
    LAST_BOT_MOUSE_Y.store(y, Ordering::Relaxed);
}

// ── Keyboard ────────────────────────────────────────────────────────────────

/// Type a sequence of characters into whatever currently has focus.
pub fn type_text(text: &str) -> Result<(), String> {
    let mut enigo = Enigo::new();
    enigo.key_sequence(text);
    Ok(())
}

/// Press a key or key combo (e.g. `"return"`, `"escape"`, `"cmd+l"`,
/// `"ctrl+shift+o"`).
///
/// Modifier names accepted: `ctrl` / `control`, `shift`, `alt` / `option`,
/// `cmd` / `command` / `meta` / `win`. Single-character keys map to
/// `Key::Layout(c)`. Named keys: `return`, `enter`, `escape`, `esc`, `tab`,
/// `space`, `backspace`, `delete`, `del`, `up`, `down`, `left`, `right`.
pub fn press_key(combo: &str) -> Result<(), String> {
    let parts: Vec<&str> = combo.split('+').collect();
    if parts.is_empty() {
        return Err("empty key combo".into());
    }

    let mut enigo = Enigo::new();
    let main_key = parse_key(parts[parts.len() - 1])?;
    let mod_keys: Vec<Key> = parts[..parts.len() - 1]
        .iter()
        .map(|s| parse_key(s))
        .collect::<Result<_, _>>()?;

    for m in &mod_keys {
        enigo.key_down(*m);
    }
    enigo.key_click(main_key);
    for m in mod_keys.iter().rev() {
        enigo.key_up(*m);
    }
    Ok(())
}

fn parse_key(s: &str) -> Result<Key, String> {
    Ok(match s.to_lowercase().as_str() {
        "ctrl" | "control" => Key::Control,
        "shift" => Key::Shift,
        "alt" | "option" => Key::Alt,
        "cmd" | "command" | "meta" | "win" => Key::Meta,
        "return" | "enter" => Key::Return,
        "escape" | "esc" => Key::Escape,
        "tab" => Key::Tab,
        "space" => Key::Space,
        "backspace" => Key::Backspace,
        "delete" | "del" => Key::Delete,
        "up" | "uparrow" => Key::UpArrow,
        "down" | "downarrow" => Key::DownArrow,
        "left" | "leftarrow" => Key::LeftArrow,
        "right" | "rightarrow" => Key::RightArrow,
        "home" => Key::Home,
        "end" => Key::End,
        "pageup" => Key::PageUp,
        "pagedown" => Key::PageDown,
        s if s.chars().count() == 1 => Key::Layout(s.chars().next().unwrap()),
        other => return Err(format!("unknown key: {other}")),
    })
}

/// The keyboard modifier that triggers browser shortcuts on this platform.
/// Returns `"cmd"` on macOS, `"ctrl"` everywhere else.
pub fn primary_modifier() -> &'static str {
    if cfg!(target_os = "macos") {
        "cmd"
    } else {
        "ctrl"
    }
}

// ── Clipboard ───────────────────────────────────────────────────────────────

pub fn read_clipboard() -> Result<String, String> {
    let mut cb = Clipboard::new().map_err(|e| e.to_string())?;
    cb.get_text().map_err(|e| e.to_string())
}

pub fn write_clipboard(text: &str) -> Result<(), String> {
    let mut cb = Clipboard::new().map_err(|e| e.to_string())?;
    cb.set_text(text.to_string()).map_err(|e| e.to_string())
}

// ── Foreground window inspection (compatibility shims) ─────────────────────
//
// These wrap `webui::browser_state` so the existing `watchdog` imports keep
// working. Platform branching lives in `browser_state`.

pub fn current_foreground_title() -> Result<String, String> {
    crate::webui::browser_state::frontmost_app_name()
}

pub fn focus_window_by_title(substr: &str) -> Result<bool, String> {
    crate::webui::browser_state::focus_app_by_name(substr)
}

pub fn current_url() -> Result<String, String> {
    crate::webui::browser_state::active_chrome_url()
}
