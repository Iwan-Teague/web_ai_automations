//! Environment invariants — the things that must remain true while the FSM
//! drives the UI. If any of these break, the FSM pauses and hands control to
//! the recovery dispatcher.
//!
//! Invariants are *cheap* checks. They run between every FSM poll cycle, so
//! anything that requires capturing the whole screen should be expressed as
//! a `WebUiAdapter` method instead.

use crate::webui::browser_state;
use crate::webui::recovery::Problem;
use std::time::{Duration, Instant};

/// Result of a single invariant check.
pub enum InvariantResult {
    Ok,
    Violated(Problem),
}

/// One environment invariant that must hold.
///
/// Implementations are platform-aware: where the platform can't answer the
/// question (e.g., reading Chrome's URL on Windows without UIA), the
/// invariant returns `Ok` so the watchdog never raises a problem it can't
/// later resolve.
pub trait Invariant: Send + Sync {
    fn name(&self) -> &'static str;
    fn check(&self) -> InvariantResult;
}

/// Run an invariant at most once per interval. Between checks, report Ok.
pub struct PeriodicInvariant {
    inner: Box<dyn Invariant>,
    interval: Duration,
    last_checked: std::sync::Mutex<Option<Instant>>,
}

impl PeriodicInvariant {
    pub fn new(inner: Box<dyn Invariant>, interval: Duration) -> Self {
        Self {
            inner,
            interval,
            last_checked: std::sync::Mutex::new(None),
        }
    }
}

impl Invariant for PeriodicInvariant {
    fn name(&self) -> &'static str {
        self.inner.name()
    }

    fn check(&self) -> InvariantResult {
        let mut guard = match self.last_checked.lock() {
            Ok(g) => g,
            Err(_) => return InvariantResult::Ok,
        };
        let now = Instant::now();
        if guard.is_some_and(|last| now.duration_since(last) < self.interval) {
            return InvariantResult::Ok;
        }
        *guard = Some(now);
        self.inner.check()
    }
}

/// Run every invariant, return all violations found.
pub fn check_all(invariants: &[Box<dyn Invariant>]) -> Vec<Problem> {
    invariants
        .iter()
        .filter_map(|inv| match inv.check() {
            InvariantResult::Ok => None,
            InvariantResult::Violated(p) => Some(p),
        })
        .collect()
}

// ── Default invariants ──────────────────────────────────────────────────────

/// The browser application must be the frontmost app.
///
/// On macOS this checks the frontmost process name; on Windows it checks
/// the foreground window's title (Chrome's title ends in "Google Chrome").
pub struct BrowserForegroundInvariant {
    pub expected_app_substr: &'static str,
}

impl Invariant for BrowserForegroundInvariant {
    fn name(&self) -> &'static str {
        "browser_foreground"
    }

    fn check(&self) -> InvariantResult {
        match browser_state::frontmost_app_name() {
            Ok(name) if name.contains(self.expected_app_substr) => InvariantResult::Ok,
            Ok(other) => {
                InvariantResult::Violated(Problem::BrowserNotForeground { seen_title: other })
            }
            // Can't tell — degrade to OK rather than spuriously firing.
            Err(_) => InvariantResult::Ok,
        }
    }
}

/// The active tab must be on the configured site.
///
/// Prefers the active-tab URL; falls back to the page title (whatever the
/// platform can give us). Both checks degrade to `Ok` on read failure.
pub struct ActiveSiteInvariant {
    pub expected_url_substr: &'static str,
    pub expected_title_substr: &'static str,
}

impl Invariant for ActiveSiteInvariant {
    fn name(&self) -> &'static str {
        "active_site"
    }

    fn check(&self) -> InvariantResult {
        if let Ok(url) = browser_state::active_chrome_url() {
            return if url.contains(self.expected_url_substr) {
                InvariantResult::Ok
            } else {
                InvariantResult::Violated(Problem::WrongUrl { seen_url: url })
            };
        }
        // Fallback to title.
        if let Ok(title) = browser_state::active_chrome_title() {
            if title.contains(self.expected_title_substr) {
                return InvariantResult::Ok;
            }
        }
        InvariantResult::Ok
    }
}

/// Chrome's front window must be in native fullscreen.
///
/// All region coordinates in `arena::regions` assume the browser
/// viewport fills the screen — if Chrome is in a windowed state every
/// click lands in the wrong place silently. This invariant catches
/// that and the `MaximizeBrowser` recovery action snaps the window
/// back into fullscreen.
///
/// Failure modes that degrade to `Ok` (rather than firing the problem):
/// - Read returned `Err` ("no_window", "unknown", platform unsupported)
///   — we don't know, so we don't fight.
pub struct BrowserFullscreenInvariant;

impl Invariant for BrowserFullscreenInvariant {
    fn name(&self) -> &'static str {
        "browser_fullscreen"
    }

    fn check(&self) -> InvariantResult {
        match browser_state::is_chrome_fullscreen() {
            Ok(true) => InvariantResult::Ok,
            Ok(false) => InvariantResult::Violated(Problem::BrowserNotFullscreen),
            Err(_) => InvariantResult::Ok,
        }
    }
}

/// Mouse must not have been moved by the user since we last looked.
///
/// On platforms where `cursor_position` is unsupported the invariant
/// reports `Ok` and never fires.
pub struct UserActivityInvariant {
    pub last_known: std::sync::Mutex<Option<(i32, i32)>>,
}

impl UserActivityInvariant {
    pub fn new() -> Self {
        Self {
            last_known: std::sync::Mutex::new(None),
        }
    }
}

impl Default for UserActivityInvariant {
    fn default() -> Self {
        Self::new()
    }
}

impl Invariant for UserActivityInvariant {
    fn name(&self) -> &'static str {
        "user_activity"
    }

    fn check(&self) -> InvariantResult {
        let pos = match crate::human_simulations::cursor_position() {
            Ok(p) => p,
            Err(_) => return InvariantResult::Ok,
        };
        let mut guard = match self.last_known.lock() {
            Ok(g) => g,
            Err(_) => return InvariantResult::Ok,
        };
        let result = match *guard {
            Some(prev)
                if prev != pos && !crate::human_simulations::is_last_bot_mouse_position(pos) =>
            {
                InvariantResult::Violated(Problem::UserActivity)
            }
            _ => InvariantResult::Ok,
        };
        *guard = Some(pos);
        result
    }
}

/// Build the default invariant set used by the FSM.
///
/// Site/window checks are throttled: they catch drift periodically without
/// spawning recovery loops on transient macOS window/fullscreen readings.
///
/// Per-site adapters can extend or replace this list before calling
/// `fsm::run_iteration`.
pub fn default_invariants(
    browser_title_substr: &'static str,
    site_url_substr: &'static str,
    site_title_substr: &'static str,
) -> Vec<Box<dyn Invariant>> {
    vec![
        Box::new(PeriodicInvariant::new(
            Box::new(BrowserForegroundInvariant {
                expected_app_substr: browser_title_substr,
            }),
            Duration::from_secs(10),
        )),
        Box::new(PeriodicInvariant::new(
            Box::new(ActiveSiteInvariant {
                expected_url_substr: site_url_substr,
                expected_title_substr: site_title_substr,
            }),
            Duration::from_secs(10),
        )),
        Box::new(UserActivityInvariant::new()),
    ]
}
