//! Append-only JSONL log of every FSM state transition.
//!
//! Written to `outputs/runs/<run>/state_log.jsonl`. After an overnight run you can
//! replay this file linearly to see exactly where the system went and why.
//!
//! Each line is one `StateTransition`. The format is intentionally flat so
//! `jq`-style queries work directly.

use std::error::Error;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::output::writer::run_root;

/// One row of the state log.
#[derive(Debug, Clone, Serialize)]
pub struct StateTransition {
    /// RFC 3339 UTC timestamp.
    pub timestamp: String,
    /// Adapter name (`"arena"`, `"null"`, …).
    pub adapter: String,
    /// 0-based task index (1-based when shown to humans).
    pub task_index: usize,
    /// 0-based iteration index.
    pub iteration: usize,
    /// Previous FSM state name.
    pub from: String,
    /// New FSM state name.
    pub to: String,
    /// Optional human-readable detail (reason / error message).
    pub note: Option<String>,
    /// Optional path to a debug screenshot taken at this transition.
    pub screenshot: Option<String>,
}

/// Writes transitions one per line.
///
/// The handle is deliberately simple — open the file in append mode, write
/// JSON + `\n`, flush. No async, no buffering across calls. Crash-safe by
/// construction.
pub struct StateLog {
    path: PathBuf,
}

impl StateLog {
    /// Open the default log location (`outputs/runs/<run>/state_log.jsonl`).
    pub fn open_default() -> Result<Self, Box<dyn Error>> {
        Self::open(run_root().join("state_log.jsonl"))
    }

    pub fn open<P: Into<PathBuf>>(path: P) -> Result<Self, Box<dyn Error>> {
        let path = path.into();
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        Ok(Self { path })
    }

    /// Append one transition.
    pub fn append(&self, t: &StateTransition) -> Result<(), Box<dyn Error>> {
        let mut f = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        let json = serde_json::to_string(t)?;
        f.write_all(json.as_bytes())?;
        f.write_all(b"\n")?;
        Ok(())
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Build a transition with the current UTC timestamp filled in.
pub fn now_transition(
    adapter: &str,
    task_index: usize,
    iteration: usize,
    from: &str,
    to: &str,
    note: Option<String>,
    screenshot: Option<String>,
) -> StateTransition {
    StateTransition {
        timestamp: chrono::Utc::now().to_rfc3339(),
        adapter: adapter.to_string(),
        task_index,
        iteration,
        from: from.to_string(),
        to: to.to_string(),
        note,
        screenshot,
    }
}
