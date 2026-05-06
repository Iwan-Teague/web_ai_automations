use std::error::Error;
use std::fmt::Write as _;
use std::path::PathBuf;

use crate::output::writer::run_root;
use crate::session::SessionConfig;

/// One per task/iteration. Captured by the runner and handed to `write_summary`.
#[derive(Clone, Debug)]
pub struct IterationOutcome {
    pub task_index: usize,
    pub iteration: usize,

    /// "ACCEPT" / "REJECT" / "REVISE" if a judge ran, otherwise None.
    pub verdict: Option<String>,
    /// Score out of 10 if a judge ran, otherwise None.
    pub score: Option<f32>,
    /// Constraints flagged by `constraint_check::check_constraint_violations`.
    pub violations: Vec<String>,
}

/// Build `outputs/runs/<run>/summary.md` from the session config and outcomes.
pub fn write_summary(
    cfg: &SessionConfig,
    outcomes: &[IterationOutcome],
) -> Result<PathBuf, Box<dyn Error>> {
    let mut s = String::new();

    let _ = writeln!(s, "# Session Summary");
    let _ = writeln!(s);
    let _ = writeln!(s, "## End Goal");
    let _ = writeln!(s, "{}", cfg.end_goal);
    let _ = writeln!(s);
    let _ = writeln!(s, "**Success criteria:** {}", cfg.success_criteria);
    let _ = writeln!(s);
    let _ = writeln!(
        s,
        "**Source:** {} ({})",
        cfg.repo_url,
        cfg.branch.as_deref().unwrap_or("default")
    );
    let _ = writeln!(s, "**Review mode:** {:?}", cfg.review_mode);
    let _ = writeln!(s);

    let _ = writeln!(s, "## Hard Constraints");
    for c in &cfg.hard_constraints {
        let _ = writeln!(s, "- {c}");
    }
    let _ = writeln!(s);

    let _ = writeln!(s, "## Iteration Results");
    let _ = writeln!(s, "| Task | Iter | Verdict | Score | Violations |");
    let _ = writeln!(s, "|------|------|---------|-------|------------|");
    for o in outcomes {
        let verdict = o.verdict.as_deref().unwrap_or("—");
        let score = o
            .score
            .map(|v| format!("{v:.1}"))
            .unwrap_or_else(|| "—".to_string());
        let viol = if o.violations.is_empty() {
            "none".to_string()
        } else {
            o.violations.join("; ")
        };
        let _ = writeln!(
            s,
            "| {} | {} | {} | {} | {} |",
            o.task_index + 1,
            o.iteration + 1,
            verdict,
            score,
            viol,
        );
    }
    let _ = writeln!(s);

    let avg_score = average_score(outcomes);
    if let Some(avg) = avg_score {
        let _ = writeln!(s, "**Average judge score:** {avg:.2}/10");
    }
    let total_viol: usize = outcomes.iter().map(|o| o.violations.len()).sum();
    let _ = writeln!(s, "**Total constraint flags:** {total_viol}");
    let _ = writeln!(s);

    let _ = writeln!(s, "## Goal Assessment");
    if let Some(avg) = avg_score {
        let line = if avg >= 8.0 {
            "Goal appears well-served by the produced outputs."
        } else if avg >= 5.0 {
            "Goal partially served — see judge revision notes per round."
        } else {
            "Goal NOT served — recommend manual review and re-run."
        };
        let _ = writeln!(s, "{line}");
    } else {
        let _ = writeln!(s, "No judge scores recorded; manual review recommended.");
    }

    let path = run_root().join("summary.md");
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&path, s)?;
    Ok(path)
}

fn average_score(outcomes: &[IterationOutcome]) -> Option<f32> {
    let scores: Vec<f32> = outcomes.iter().filter_map(|o| o.score).collect();
    if scores.is_empty() {
        return None;
    }
    Some(scores.iter().sum::<f32>() / scores.len() as f32)
}
