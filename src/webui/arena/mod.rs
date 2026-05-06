//! Arena.ai adapter.
//!
//! Splits cleanly into three files so each concern is editable in isolation:
//!
//! - [`regions`]   — bounded screen regions (where elements can appear).
//! - [`templates`] — `OnceLock` template caches + asset path constants.
//! - [`adapter`]   — `WebUiAdapter` impl that ties the two together.
//!
//! Arena is the first site implementation; new sites copy this file layout
//! into a sibling submodule (`webui::chatgpt`, `webui::gemini`, …).

pub mod adapter;
pub mod regions;
pub mod session_setup;
pub mod templates;

pub use adapter::ArenaAdapter;
pub use regions::ArenaSurface;

/// Site hints used by the watchdog and recovery dispatcher.
///
/// `arena.ai` is a substring match — it covers both `arena.ai` (the
/// product) and `lmarena.ai` (the older leaderboard). If you need to
/// disambiguate, replace this with the more specific host you target.
pub const ARENA_URL_SUBSTR: &str = "arena.ai";
pub const ARENA_TITLE_SUBSTR: &str = "Arena";
pub const BROWSER_TITLE_SUBSTR: &str = "Chrome";

/// Full URL the session-setup uses when opening a fresh Arena tab.
pub const ARENA_HOME_URL: &str = "https://arena.ai";
