// Library crate root — exposes modules for integration tests under `tests/`
// and for any future binaries that want to reuse the runner. The `main.rs`
// binary uses the same module tree via `web_ai_automation::*`.
pub mod calibration;
pub mod human_simulations;
pub mod image_matrix;
pub mod intake;
pub mod output;
pub mod project_map;
pub mod prompt;
pub mod runner;
pub mod session;
pub mod webui;
