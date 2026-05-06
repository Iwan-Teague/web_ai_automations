//! Human-friendly chronological transcript of every prompt + response.
//!
//! Written to `outputs/runs/<run>/transcript.md`, append-only.
//!
//! Every entry is a self-contained markdown section with timestamp, model
//! name (if known), the prompt, and the response.

use std::error::Error;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;

use crate::output::writer::run_root;

/// One record appended to the transcript.
#[derive(Debug, Clone)]
pub struct TranscriptEntry<'a> {
    pub task_index: usize,
    pub iteration: usize,
    pub adapter: &'a str,
    pub model: Option<&'a str>,
    pub prompt: &'a str,
    pub response: &'a str,
    /// Free-form note (e.g., constraint violations, judge verdict).
    pub note: Option<&'a str>,
}

pub fn append(entry: &TranscriptEntry) -> Result<PathBuf, Box<dyn Error>> {
    let path = run_root().join("transcript.md");
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }

    let timestamp = chrono::Utc::now().to_rfc3339();
    let model = entry.model.unwrap_or("(unknown)");

    let mut block = String::new();
    block.push_str(&format!(
        "\n## Task {} · Iter {} — {} · model: {}\n",
        entry.task_index + 1,
        entry.iteration + 1,
        timestamp,
        model,
    ));
    block.push_str(&format!("**Adapter:** `{}`\n\n", entry.adapter));
    if let Some(note) = entry.note {
        block.push_str(&format!("**Note:** {note}\n\n"));
    }
    block.push_str("### Prompt\n\n");
    block.push_str("```\n");
    block.push_str(entry.prompt);
    if !entry.prompt.ends_with('\n') {
        block.push('\n');
    }
    block.push_str("```\n\n");
    block.push_str("### Response\n\n");
    block.push_str("```\n");
    block.push_str(entry.response);
    if !entry.response.ends_with('\n') {
        block.push('\n');
    }
    block.push_str("```\n");
    block.push_str("\n---\n");

    let mut f = OpenOptions::new().create(true).append(true).open(&path)?;
    f.write_all(block.as_bytes())?;
    Ok(path)
}
