//! DeepSeek chat adapter.
//!
//! Uses the same FSM contract as Arena: site-specific code owns regions,
//! templates, setup, and click/copy behavior; the runner stays generic.

pub mod adapter;
pub mod regions;
pub mod session_setup;
pub mod templates;

pub use adapter::DeepSeekAdapter;

pub const DEEPSEEK_URL_SUBSTR: &str = "chat.deepseek.com";
pub const DEEPSEEK_TITLE_SUBSTR: &str = "DeepSeek";
pub const BROWSER_TITLE_SUBSTR: &str = "Chrome";
pub const DEEPSEEK_HOME_URL: &str = "https://chat.deepseek.com";
