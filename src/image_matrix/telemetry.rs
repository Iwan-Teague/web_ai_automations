//! Per-match score telemetry.
//!
//! Every template-match call funnels through `record()`, which writes a
//! JSON line to `outputs/runs/<latest>/match_scores.jsonl`. This gives us
//! a chronological record of how each template scores over time so we can
//! spot drift (DeepSeek/Arena ships a UI tweak → a template that used to
//! match at 0.93 starts matching at 0.78 → eventually misses).
//!
//! Goals:
//! - Cheap: one append-only line per match call. No JSON-parsing, no
//!   global state beyond a `OnceLock<Mutex<File>>`.
//! - Resilient: never fail the calling match if logging fails. A broken
//!   filesystem must not break detection.
//! - Self-locating: writes go to whichever run folder is currently
//!   active, picked up at first use of `record()` per process.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

static LOG_FILE: OnceLock<Option<Mutex<File>>> = OnceLock::new();

/// Append one JSONL record describing a template-match call. `name` is a
/// stable label for the template being matched (e.g. `"new_chat"`,
/// `"chip_close_x"`). `score` is the best score the matcher returned;
/// passing `None` indicates the template wasn't found at all. `region`
/// is the search box in screen coords for retrospective sanity checks.
pub fn record(name: &str, score: Option<f32>, region: (i32, i32, i32, i32)) {
    let Some(handle) = LOG_FILE.get_or_init(open_log_file) else {
        return;
    };
    let Ok(mut file) = handle.lock() else {
        return;
    };
    let unix_secs = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let score_field = match score {
        Some(s) => format!("{s:.4}"),
        None => "null".to_string(),
    };
    let line = format!(
        "{{\"ts\":{unix_secs},\"template\":\"{name}\",\"score\":{score_field},\"region\":[{},{},{},{}]}}\n",
        region.0, region.1, region.2, region.3
    );
    let _ = file.write_all(line.as_bytes());
}

fn open_log_file() -> Option<Mutex<File>> {
    let path = locate_log_path()?;
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .ok()
        .map(Mutex::new)
}

/// Pick a destination for the score log. Prefers the most recent
/// `outputs/runs/run_*` folder (the runner creates one per session); if
/// no run folder exists yet (early CLI invocations, tests), falls back
/// to a top-level file under `outputs/`.
fn locate_log_path() -> Option<PathBuf> {
    let runs_dir = PathBuf::from("outputs/runs");
    if let Ok(read) = fs::read_dir(&runs_dir) {
        let mut latest: Option<(SystemTime, PathBuf)> = None;
        for entry in read.flatten() {
            let p = entry.path();
            if !p.is_dir() {
                continue;
            }
            let mtime = entry
                .metadata()
                .and_then(|m| m.modified())
                .unwrap_or(SystemTime::UNIX_EPOCH);
            match &latest {
                Some((cur, _)) if *cur >= mtime => {}
                _ => latest = Some((mtime, p)),
            }
        }
        if let Some((_, path)) = latest {
            return Some(path.join("match_scores.jsonl"));
        }
    }
    Some(PathBuf::from("outputs/match_scores.jsonl"))
}
