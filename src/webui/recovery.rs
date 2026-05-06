//! Recovery action library + dispatcher.
//!
//! Watchdog detects a `Problem`. The dispatcher walks its action list in
//! cost order, picks the cheapest action that applies, and runs it. The
//! action returns one of:
//!
//! - `Success`  — problem resolved, FSM may resume.
//! - `Retry`    — try this same action again (with backoff).
//! - `Escalate` — give up on this action; try the next applicable one.
//!
//! When every applicable action escalates, the dispatcher returns `Failed`
//! and the FSM halts the iteration with a full diagnostic dump.

use std::error::Error as StdError;
use std::fmt;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use crate::webui::adapter::WebUiAdapter;

// ── Problem catalogue ───────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum Problem {
    BrowserNotForeground {
        seen_title: String,
    },
    WrongUrl {
        seen_url: String,
    },
    UnexpectedTab,
    UnexpectedModal,
    BrowserMissing,
    UserActivity,
    /// Adapter returned `Unknown` repeatedly — we have no idea what's on screen.
    UnknownScreen,
    /// Adapter reported `Error` — generation failed.
    GenerationError,
    /// Adapter reported `ChatFull` — must start a new chat.
    ChatFull,
    /// Browser is not in native fullscreen — the regions calibration
    /// assumes it is, so non-fullscreen state breaks every click.
    BrowserNotFullscreen,
    /// Anything not categorised. Carries a free-form description.
    Other(String),
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Problem::BrowserNotForeground { seen_title } => {
                write!(f, "browser not foreground (saw '{seen_title}')")
            }
            Problem::WrongUrl { seen_url } => write!(f, "wrong URL (saw '{seen_url}')"),
            Problem::UnexpectedTab => write!(f, "unexpected tab"),
            Problem::UnexpectedModal => write!(f, "unexpected modal"),
            Problem::BrowserMissing => write!(f, "browser missing"),
            Problem::UserActivity => write!(f, "user activity detected"),
            Problem::UnknownScreen => write!(f, "unknown screen"),
            Problem::GenerationError => write!(f, "generation error"),
            Problem::ChatFull => write!(f, "chat full"),
            Problem::BrowserNotFullscreen => write!(f, "browser not fullscreen"),
            Problem::Other(s) => write!(f, "other: {s}"),
        }
    }
}

// ── Outcomes ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryOutcome {
    /// Problem is resolved, FSM may resume.
    Success,
    /// Run this action again (after a short backoff).
    Retry,
    /// Move to the next applicable action.
    Escalate,
}

#[derive(Debug)]
pub enum DispatchResult {
    Recovered { action: &'static str },
    Failed { tried: Vec<&'static str> },
}

// ── Recovery context ────────────────────────────────────────────────────────

/// Everything an action might need to act on the world.
///
/// Held by `Arc` because actions are stored as trait objects and the
/// dispatcher hands the same context to each one.
pub struct RecoveryContext {
    pub adapter: Arc<dyn WebUiAdapter>,
    pub site: SiteHints,
}

/// Per-site hints used by generic recovery actions.
#[derive(Clone, Debug)]
pub struct SiteHints {
    pub browser_title_substr: &'static str,
    pub site_url: &'static str,
    pub site_title_substr: &'static str,
}

// ── Action trait ────────────────────────────────────────────────────────────

pub trait RecoveryAction: Send + Sync {
    /// Stable name used in logs and the dispatch result.
    fn name(&self) -> &'static str;

    /// Lower = cheaper / safer. Actions are tried in ascending cost order.
    fn cost(&self) -> u8;

    /// Whether this action makes sense for this problem.
    fn applies_to(&self, problem: &Problem) -> bool;

    /// Run the action. Should be idempotent where possible.
    fn execute(&self, ctx: &RecoveryContext) -> RecoveryOutcome;
}

// ── Dispatcher ──────────────────────────────────────────────────────────────

pub struct RecoveryDispatcher {
    actions: Vec<Box<dyn RecoveryAction>>,
    /// How many times to retry a single action before escalating.
    max_retries: u8,
    /// Pause between retries.
    backoff: Duration,
}

impl RecoveryDispatcher {
    pub fn new(actions: Vec<Box<dyn RecoveryAction>>) -> Self {
        let mut me = Self {
            actions,
            max_retries: 2,
            backoff: Duration::from_millis(750),
        };
        me.actions.sort_by_key(|a| a.cost());
        me
    }

    /// Try every applicable action in cost order. First `Success` wins.
    pub fn handle(&self, problem: &Problem, ctx: &RecoveryContext) -> DispatchResult {
        let mut tried: Vec<&'static str> = Vec::new();

        for action in self.actions.iter().filter(|a| a.applies_to(problem)) {
            tried.push(action.name());

            for _attempt in 0..=self.max_retries {
                match action.execute(ctx) {
                    RecoveryOutcome::Success => {
                        return DispatchResult::Recovered {
                            action: action.name(),
                        };
                    }
                    RecoveryOutcome::Retry => {
                        thread::sleep(self.backoff);
                        continue;
                    }
                    RecoveryOutcome::Escalate => break,
                }
            }
        }

        DispatchResult::Failed { tried }
    }
}

// ── Default action set ──────────────────────────────────────────────────────

/// Click the browser's window in the taskbar (or otherwise focus it).
pub struct RefocusBrowser;

impl RecoveryAction for RefocusBrowser {
    fn name(&self) -> &'static str {
        "refocus_browser"
    }
    fn cost(&self) -> u8 {
        10
    }

    fn applies_to(&self, p: &Problem) -> bool {
        matches!(
            p,
            Problem::BrowserNotForeground { .. } | Problem::UnexpectedTab
        )
    }

    fn execute(&self, ctx: &RecoveryContext) -> RecoveryOutcome {
        eprintln!(
            "[recovery] refocus_browser firing — substr '{}'",
            ctx.site.browser_title_substr
        );
        match crate::human_simulations::focus_window_by_title(ctx.site.browser_title_substr) {
            Ok(true) => RecoveryOutcome::Success,
            Ok(false) => RecoveryOutcome::Escalate,
            Err(_) => RecoveryOutcome::Escalate,
        }
    }
}

/// Search every Chrome window for an existing tab on the configured site
/// and bring it to front. **Never clobbers the user's current tab** — if
/// no matching tab is found this action escalates to the next one (which
/// will typically open a fresh tab).
///
/// Cheaper than `NavigateBackToSite` because it touches no input — just
/// asks AppleScript / UIA to switch tabs.
pub struct FocusSiteTab;

impl RecoveryAction for FocusSiteTab {
    fn name(&self) -> &'static str {
        "focus_site_tab"
    }
    fn cost(&self) -> u8 {
        20
    }

    fn applies_to(&self, p: &Problem) -> bool {
        matches!(p, Problem::WrongUrl { .. } | Problem::UnexpectedTab)
    }

    fn execute(&self, ctx: &RecoveryContext) -> RecoveryOutcome {
        eprintln!(
            "[recovery] focus_site_tab firing for site '{}'",
            ctx.site.site_url
        );
        match crate::webui::browser_state::focus_chrome_tab_by_url(ctx.site.site_url) {
            Ok(true) => {
                eprintln!("[recovery] focus_site_tab → matched");
                RecoveryOutcome::Success
            }
            Ok(false) => {
                eprintln!("[recovery] focus_site_tab → no matching tab; escalating");
                RecoveryOutcome::Escalate
            }
            Err(e) => {
                eprintln!("[recovery] focus_site_tab → error: {e}; escalating");
                RecoveryOutcome::Escalate
            }
        }
    }
}

/// Open a brand-new browser tab pointing at the site URL.
///
/// Used when no existing tab matches. We deliberately open a NEW tab
/// rather than retyping into the current tab's URL bar — that way the
/// user's other open tabs survive untouched.
pub struct NavigateBackToSite;

impl RecoveryAction for NavigateBackToSite {
    fn name(&self) -> &'static str {
        "navigate_back_to_site"
    }
    fn cost(&self) -> u8 {
        35
    }

    fn applies_to(&self, p: &Problem) -> bool {
        matches!(p, Problem::WrongUrl { .. } | Problem::UnexpectedTab)
    }

    fn execute(&self, ctx: &RecoveryContext) -> RecoveryOutcome {
        // Build a full URL — `site_url` is just a substring like
        // "arena.ai", so prefix `https://` to make it navigable.
        let url = if ctx.site.site_url.starts_with("http") {
            ctx.site.site_url.to_string()
        } else {
            format!("https://{}", ctx.site.site_url)
        };
        eprintln!("[recovery] navigate_back_to_site firing — opening NEW tab to {url}");
        match crate::webui::browser_state::open_new_browser_tab(&url) {
            Ok(_) => RecoveryOutcome::Success,
            Err(e) => {
                eprintln!("[recovery] open_new_browser_tab failed: {e}");
                RecoveryOutcome::Escalate
            }
        }
    }
}

/// Ask the adapter to dismiss any visible modal.
pub struct DismissModal;

impl RecoveryAction for DismissModal {
    fn name(&self) -> &'static str {
        "dismiss_modal"
    }
    fn cost(&self) -> u8 {
        5
    }

    fn applies_to(&self, p: &Problem) -> bool {
        matches!(p, Problem::UnexpectedModal | Problem::UnknownScreen)
    }

    fn execute(&self, ctx: &RecoveryContext) -> RecoveryOutcome {
        match ctx.adapter.dismiss_modal() {
            Ok(true) => RecoveryOutcome::Success,
            Ok(false) => RecoveryOutcome::Escalate,
            Err(_) => RecoveryOutcome::Escalate,
        }
    }
}

/// Press Escape — a cheap, almost-always-safe attempt at closing popups.
pub struct PressEscape;

impl RecoveryAction for PressEscape {
    fn name(&self) -> &'static str {
        "press_escape"
    }
    fn cost(&self) -> u8 {
        1
    }

    fn applies_to(&self, p: &Problem) -> bool {
        matches!(p, Problem::UnexpectedModal | Problem::UnknownScreen)
    }

    fn execute(&self, _ctx: &RecoveryContext) -> RecoveryOutcome {
        match crate::human_simulations::press_key("escape") {
            Ok(_) => RecoveryOutcome::Retry, // verify by re-detecting state
            Err(_) => RecoveryOutcome::Escalate,
        }
    }
}

/// Force Chrome's front window into native fullscreen. The whole
/// `regions` calibration assumes the browser viewport fills the
/// screen — non-fullscreen state silently breaks every click.
///
/// Idempotent on macOS (`AXFullScreen ← true` is a no-op when already
/// true), so the action is safe to fire whenever the invariant trips.
pub struct MaximizeBrowser;

impl RecoveryAction for MaximizeBrowser {
    fn name(&self) -> &'static str {
        "maximize_browser"
    }
    fn cost(&self) -> u8 {
        15
    }

    fn applies_to(&self, p: &Problem) -> bool {
        matches!(p, Problem::BrowserNotFullscreen)
    }

    fn execute(&self, _ctx: &RecoveryContext) -> RecoveryOutcome {
        eprintln!("[recovery] maximize_browser firing");
        match crate::webui::browser_state::set_chrome_fullscreen(true) {
            Ok(_) => RecoveryOutcome::Success,
            Err(e) => {
                eprintln!("[recovery] set_chrome_fullscreen failed: {e}");
                RecoveryOutcome::Escalate
            }
        }
    }
}

/// Open a new chat — the only sensible response to `ChatFull`.
pub struct StartNewChat;

impl RecoveryAction for StartNewChat {
    fn name(&self) -> &'static str {
        "start_new_chat"
    }
    fn cost(&self) -> u8 {
        50
    }

    fn applies_to(&self, p: &Problem) -> bool {
        matches!(p, Problem::ChatFull)
    }

    fn execute(&self, ctx: &RecoveryContext) -> RecoveryOutcome {
        match ctx.adapter.start_new_chat() {
            Ok(_) => RecoveryOutcome::Success,
            Err(_) => RecoveryOutcome::Escalate,
        }
    }
}

/// Pause the run when the user has touched the mouse. Resume once the cursor
/// has stayed still for a short idle window, instead of terminating the run.
pub struct WaitForUserIdle;

impl RecoveryAction for WaitForUserIdle {
    fn name(&self) -> &'static str {
        "wait_for_user_idle"
    }
    fn cost(&self) -> u8 {
        1
    }

    fn applies_to(&self, p: &Problem) -> bool {
        matches!(p, Problem::UserActivity)
    }

    fn execute(&self, _ctx: &RecoveryContext) -> RecoveryOutcome {
        let idle_for = Duration::from_secs(5);
        let poll = Duration::from_millis(500);

        eprintln!("[recovery] user activity detected — pausing until cursor idle for 5s");

        let mut last_pos = match crate::human_simulations::cursor_position() {
            Ok(pos) => pos,
            Err(e) => {
                eprintln!("[recovery] cursor_position failed while waiting for idle: {e}");
                return RecoveryOutcome::Success;
            }
        };
        let mut stable_since = Instant::now();
        let mut last_log = Instant::now();

        loop {
            thread::sleep(poll);
            let pos = match crate::human_simulations::cursor_position() {
                Ok(pos) => pos,
                Err(e) => {
                    eprintln!("[recovery] cursor_position failed while waiting for idle: {e}");
                    return RecoveryOutcome::Success;
                }
            };

            if pos != last_pos {
                last_pos = pos;
                stable_since = Instant::now();
                if last_log.elapsed() >= Duration::from_secs(30) {
                    eprintln!("[recovery] still paused for user activity");
                    last_log = Instant::now();
                }
                continue;
            }

            if stable_since.elapsed() >= idle_for {
                eprintln!("[recovery] cursor idle — resuming automation");
                return RecoveryOutcome::Success;
            }
        }
    }
}

/// Build the default action set. Per-site adapters may override.
///
/// Order is by `cost()` (the dispatcher sorts internally). Cheaper, less
/// invasive actions run first; clobber-risk actions run last.
pub fn default_actions() -> Vec<Box<dyn RecoveryAction>> {
    vec![
        Box::new(PressEscape),
        Box::new(DismissModal),
        Box::new(WaitForUserIdle),
        Box::new(RefocusBrowser),
        Box::new(MaximizeBrowser),
        Box::new(FocusSiteTab),
        Box::new(NavigateBackToSite),
        Box::new(StartNewChat),
    ]
}

// ── Recovery error wrapper ──────────────────────────────────────────────────

#[derive(Debug)]
pub struct UnrecoveredProblem {
    pub problem: Problem,
    pub tried: Vec<&'static str>,
}

impl fmt::Display for UnrecoveredProblem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "unrecovered problem '{}', tried [{}]",
            self.problem,
            self.tried.join(", "),
        )
    }
}

impl StdError for UnrecoveredProblem {}
