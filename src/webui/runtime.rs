//! Crash-resilient resume state.
//!
//! After every FSM transition the runner writes its current
//! `(task_index, iteration, state_name)` to `session/runtime.json`. On
//! startup, if that file exists, the runner offers to resume from there.
//!
//! This survives:
//! - The user killing the process and reopening it the next morning.
//! - The OS crashing mid-run.
//! - The runner halting because recovery escalated to `Failed`.
//!
//! It does NOT survive deleting the `outputs/` directory (because the
//! per-iteration markdown is gone), but the runtime file alone tells you
//! exactly where to pick up.

use std::error::Error;
use std::fs::{self, File};
use std::io::{BufReader, BufWriter};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const RUNTIME_PATH: &str = "session/runtime.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeState {
    pub task_index: usize,
    pub iteration: usize,
    /// FSM state name, e.g. `"Generating"`, `"Copying"`. Free-form so we
    /// don't have to keep this enum and the FSM enum in lockstep.
    pub last_state: String,
    /// RFC 3339 UTC timestamp of the most recent transition.
    pub updated_at: String,
    /// Optional adapter name — sanity-check that resume is for the same site.
    pub adapter: String,
}

impl RuntimeState {
    pub fn save(&self) -> Result<(), Box<dyn Error>> {
        save_to(self, Path::new(RUNTIME_PATH))
    }

    pub fn load() -> Option<Self> {
        load_from(Path::new(RUNTIME_PATH)).ok()
    }

    /// Remove the runtime file. Called when the run finishes cleanly so the
    /// next session does not offer a stale resume.
    pub fn clear() -> Result<(), Box<dyn Error>> {
        let p = Path::new(RUNTIME_PATH);
        if p.exists() {
            fs::remove_file(p)?;
        }
        Ok(())
    }
}

fn save_to(state: &RuntimeState, path: &Path) -> Result<(), Box<dyn Error>> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp: PathBuf = path.with_extension("json.tmp");
    {
        let f = File::create(&tmp)?;
        let w = BufWriter::new(f);
        serde_json::to_writer_pretty(w, state)?;
    }
    // Atomic rename — readers either see the old file or the new one,
    // never a partial write.
    fs::rename(&tmp, path)?;
    Ok(())
}

fn load_from(path: &Path) -> Result<RuntimeState, Box<dyn Error>> {
    let f = File::open(path)?;
    let r = BufReader::new(f);
    let s: RuntimeState = serde_json::from_reader(r)?;
    Ok(s)
}
