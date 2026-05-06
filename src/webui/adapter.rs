//! Adapter trait + shared types every per-site adapter speaks.
//!
//! Per-site adapters live in `src/webui/<site>/`. They own their own template
//! images, screen regions, and click coordinates. The runner only ever calls
//! the methods on this trait, so swapping Arena for ChatGPT is a one-line
//! change in `task_runner`.

use std::error::Error as StdError;
use std::fmt;
use std::path::Path;

use crate::image_matrix::types::Tolerance;

// ── Perceived screen state ──────────────────────────────────────────────────

/// What the adapter thinks the screen is showing right now.
///
/// `Unknown` is a distinct, valid result — it means none of the known
/// templates matched, so the runner should pause and let recovery decide
/// what to do. It is *not* an error.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WebUiState {
    /// Input box is visible and accepting text. Ready to submit a prompt.
    Ready,
    /// Send button has been clicked, no generation feedback yet.
    Submitting,
    /// Generation in progress (e.g., "Stop generating" button visible).
    Generating,
    /// Generation finished, response is on screen, copy is available.
    Complete,
    /// Error banner / "Regenerate" button visible.
    Error,
    /// Conversation-too-long modal / "Start new chat" suggestion.
    ChatFull,
    /// None of the above. Diagnostic state — recovery should run.
    Unknown,
}

impl fmt::Display for WebUiState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            WebUiState::Ready => "Ready",
            WebUiState::Submitting => "Submitting",
            WebUiState::Generating => "Generating",
            WebUiState::Complete => "Complete",
            WebUiState::Error => "Error",
            WebUiState::ChatFull => "ChatFull",
            WebUiState::Unknown => "Unknown",
        })
    }
}

// ── Anchors and search regions ──────────────────────────────────────────────

/// A bounded rectangle on the screen, in absolute screen coordinates.
///
/// `x2`/`y2` are exclusive (matches the `capture_*_matrix` convention).
#[derive(Clone, Copy, Debug)]
pub struct Region {
    pub x1: i32,
    pub y1: i32,
    pub x2: i32,
    pub y2: i32,
}

impl Region {
    pub fn width(self) -> i32 {
        self.x2 - self.x1
    }
    pub fn height(self) -> i32 {
        self.y2 - self.y1
    }
}

/// Configuration for one detection — *what* to look for, *where* to look,
/// and *how strictly* to match.
///
/// The actual `GrayMatrix` lives in a static cache (per-site). The anchor
/// itself is a config record so it stays cheap to build a list of them and
/// pass them into a detection function.
#[derive(Clone, Debug)]
pub struct TemplateAnchor {
    /// Stable label used for logging, debug screenshots, and error messages.
    pub label: &'static str,
    /// Where on the screen this template can plausibly appear.
    pub region: Region,
    /// Per-channel tolerance (ITU greyscale).
    pub tolerance: Tolerance,
    /// Required match-rate (0.0–1.0) for the search to count as a hit.
    pub threshold: f32,
    /// Asset path, relative to the project root. Used by per-site loaders.
    pub asset_path: &'static str,
}

// ── Adapter trait ───────────────────────────────────────────────────────────

/// Errors a per-site adapter can return.
///
/// These are intentionally categorised so the recovery dispatcher can
/// pattern-match on them without parsing strings.
#[derive(Debug)]
pub enum AdapterError {
    /// An expected element was not on screen within the allotted time.
    NotFound { what: String },
    /// An operation timed out (waiting for a state change, etc.).
    Timeout { what: String, after_ms: u64 },
    /// Screen capture failed.
    Capture(String),
    /// Mouse/keyboard input simulation failed.
    Input(String),
    /// Clipboard read/write failed.
    Clipboard(String),
    /// An asset (template PNG) could not be loaded.
    Asset { path: String, reason: String },
    /// Any other adapter-level failure.
    Other(String),
}

impl fmt::Display for AdapterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AdapterError::NotFound { what } => write!(f, "not found: {what}"),
            AdapterError::Timeout { what, after_ms } => {
                write!(f, "timeout waiting for {what} after {after_ms}ms")
            }
            AdapterError::Capture(s) => write!(f, "capture failed: {s}"),
            AdapterError::Input(s) => write!(f, "input failed: {s}"),
            AdapterError::Clipboard(s) => write!(f, "clipboard failed: {s}"),
            AdapterError::Asset { path, reason } => {
                write!(f, "asset load failed: {path} ({reason})")
            }
            AdapterError::Other(s) => write!(f, "{s}"),
        }
    }
}

impl StdError for AdapterError {}

/// Trait every per-site adapter implements.
///
/// Methods take `&self` so an adapter can be shared across the runner and
/// recovery dispatcher without `Mutex` ceremony. Adapters that need interior
/// state (e.g., `NullAdapter`) use `Mutex` internally.
pub trait WebUiAdapter: Send + Sync {
    /// Short human-readable name used in logs (e.g., `"arena"`).
    fn name(&self) -> &'static str;

    /// Recovery hook called by the FSM before every `detect_state` poll.
    ///
    /// Adapters use this to refresh whatever invariants their state-reading
    /// depends on — most commonly, raising the browser to the foreground so
    /// region captures don't read pixels from another window that drifted on
    /// top. Default: no-op. Failures are logged and ignored; they should
    /// never block detection.
    fn prepare_for_detection(&self) -> Result<(), AdapterError> {
        Ok(())
    }

    /// Read the screen and classify into a `WebUiState`.
    ///
    /// Must not click, type, or otherwise change the screen — purely a read.
    fn detect_state(&self) -> Result<WebUiState, AdapterError>;

    /// Try to read the active model name. Returns `None` if not visually
    /// present in the current screen layout.
    fn detect_model(&self) -> Result<Option<String>, AdapterError>;

    /// Type the prompt and click the send button.
    ///
    /// Should NOT block waiting for generation — that is the FSM's job.
    fn submit_prompt(&self, text: &str) -> Result<(), AdapterError>;

    /// Click the response's copy button and return the clipboard contents.
    ///
    /// Always reads from the clipboard rather than OCR'ing rendered text —
    /// far more reliable across font/theme changes.
    fn copy_response(&self) -> Result<String, AdapterError>;

    /// Open a fresh chat (used to recover from `ChatFull`).
    fn start_new_chat(&self) -> Result<(), AdapterError>;

    /// Try to dismiss any visible modal/overlay. Returns `true` if a known
    /// modal was found and dismissed, `false` if nothing dismissable was on
    /// screen.
    fn dismiss_modal(&self) -> Result<bool, AdapterError>;

    /// Download generated artifacts (e.g. code zip from a code panel).
    /// Returns `true` if something was downloaded, `false` if nothing to
    /// download (no code panel, or not a code task). Default: no-op.
    fn download_artifacts(&self) -> Result<bool, AdapterError> {
        Ok(false)
    }

    /// Download generated artifacts into a caller-owned output directory.
    /// Default keeps older adapters working.
    fn download_artifacts_to(&self, dest_dir: &Path) -> Result<bool, AdapterError> {
        let _ = dest_dir;
        self.download_artifacts()
    }

    /// Attach a file to the current chat input before submitting the first prompt.
    ///
    /// Clicks the site's file-upload button, then uses osascript to navigate the
    /// macOS file-open dialog to `path`. Returns `true` if the file was attached,
    /// `false` if this adapter does not support file upload. Default: no-op.
    fn upload_file(&self, path: &Path) -> Result<bool, AdapterError> {
        let _ = path;
        Ok(false)
    }

    /// Best-effort file upload that MUST NOT reload the page on persistent
    /// rejection. Used by the runner to retry previously-deferred uploads
    /// inside an active chat — losing the conversation to a reload would
    /// be worse than just leaving the file deferred for another iteration.
    /// Default: delegate to `upload_file`. DeepSeek overrides to delete
    /// the rejected chip locally and return `Err` instead of reloading.
    fn try_upload_preserving_chat(&self, path: &Path) -> Result<bool, AdapterError> {
        self.upload_file(path)
    }

    /// Rotate to a fresh chat for long-running sessions, re-applying every
    /// adapter-specific setting (model selection, toggles, panel layout).
    ///
    /// Called periodically by the runner to keep DeepSeek's context window
    /// from saturating across many prompts. Returns `true` if the adapter
    /// performed a rotation, `false` if the operation is a no-op for this
    /// adapter (default).
    fn rotate_for_long_session(
        &self,
        cfg: &crate::session::SessionConfig,
    ) -> Result<bool, AdapterError> {
        let _ = cfg;
        Ok(false)
    }
}
