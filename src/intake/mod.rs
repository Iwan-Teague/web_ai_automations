use std::error::Error;

use crate::session::{SessionConfig, WebUiTarget};

pub mod simplified;
pub mod stage_1_source;
pub mod stage_2_goal;
pub mod stage_3_constraints;
pub mod stage_4_tasks;
pub mod stage_5_review;

// ── Shared stdin helpers ─────────────────────────────────────────────────────

use std::io::{self, BufRead, Write};

/// Print a header banner for a stage.
pub(crate) fn banner(title: &str) {
    println!();
    println!("╔══════════════════════════════════════════════════════════════════╗");
    println!("║  {:<64}║", title);
    println!("╠══════════════════════════════════════════════════════════════════╣");
}

pub(crate) fn footer() {
    println!("╚══════════════════════════════════════════════════════════════════╝");
}

/// Prompt the user for a single line of input.
pub(crate) fn read_line(prompt: &str) -> Result<String, Box<dyn Error>> {
    print!("{prompt}");
    io::stdout().flush()?;

    let stdin = io::stdin();
    let mut line = String::new();
    stdin.lock().read_line(&mut line)?;
    Ok(line.trim().to_string())
}

/// Read multiple lines until the user enters two consecutive blank lines.
pub(crate) fn read_multiline(prompt: &str) -> Result<String, Box<dyn Error>> {
    println!("{prompt} (end with two empty lines):");
    let stdin = io::stdin();
    let mut buf = String::new();
    let mut pending_blank = false;
    for line in stdin.lock().lines() {
        let line = line?;
        if line.is_empty() {
            if pending_blank {
                break;
            }
            pending_blank = true;
            continue;
        }
        if pending_blank && !buf.is_empty() {
            buf.push('\n');
        }
        pending_blank = false;
        buf.push_str(&line);
        buf.push('\n');
    }
    Ok(buf.trim_end().to_string())
}

/// Read a list of items, one per line, until blank.
pub(crate) fn read_list(prompt: &str) -> Result<Vec<String>, Box<dyn Error>> {
    println!("{prompt} (one per line, blank to finish):");
    let stdin = io::stdin();
    let mut items = Vec::new();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.is_empty() {
            break;
        }
        items.push(line.trim().to_string());
    }
    Ok(items)
}

/// Read an integer, retrying on parse failure.
pub(crate) fn read_usize(prompt: &str, default: usize) -> Result<usize, Box<dyn Error>> {
    loop {
        let raw = read_line(&format!("{prompt} [{default}]: "))?;
        if raw.is_empty() {
            return Ok(default);
        }
        match raw.parse::<usize>() {
            Ok(v) => return Ok(v),
            Err(_) => println!("  not a valid integer — try again"),
        }
    }
}

/// Read a yes/no with a default.
pub(crate) fn read_bool(prompt: &str, default: bool) -> Result<bool, Box<dyn Error>> {
    let hint = if default { "[Y/n]" } else { "[y/N]" };
    loop {
        let raw = read_line(&format!("{prompt} {hint}: "))?.to_lowercase();
        if raw.is_empty() {
            return Ok(default);
        }
        match raw.as_str() {
            "y" | "yes" => return Ok(true),
            "n" | "no" => return Ok(false),
            _ => println!("  please answer y or n"),
        }
    }
}

// ── Top-level intake driver ──────────────────────────────────────────────────

/// Run all five intake stages in order and return a fully populated `SessionConfig`.
pub fn run_intake() -> Result<SessionConfig, Box<dyn Error>> {
    let s1 = stage_1_source::run()?;
    let s2 = stage_2_goal::run()?;
    let s3 = stage_3_constraints::run()?;
    let s4 = stage_4_tasks::run()?;
    let s5 = stage_5_review::run()?;

    Ok(SessionConfig {
        repo_url: s1.repo_url,
        branch: s1.branch,
        content_type: s1.content_type,

        end_goal: s2.end_goal,
        success_criteria: s2.success_criteria,

        hard_constraints: s3.hard_constraints,
        background_context: s3.background_context,
        tech_constraints: s3.tech_constraints,

        tasks: s4.tasks,
        iterations_per_task: s4.iterations_per_task,
        auto_continue: s4.auto_continue,

        review_mode: s5.review_mode,
        debate_rounds: s5.debate_rounds,
        bull_strength: s5.bull_strength,
        bear_strength: s5.bear_strength,
        auto_apply_judge: s5.auto_apply_judge,
        debate_mode_enabled: matches!(s5.review_mode, crate::session::ReviewMode::Debate),

        prompt_mode: crate::session::PromptMode::default(),
        coding_specialty: crate::session::CodingSpecialty::default(),
        research_specialty: crate::session::CodingSpecialty::default(),
        target_webui: WebUiTarget::default(),
        arena_model: None,
        deepseek_model: crate::session::DeepSeekModelMode::default(),
        deepseek_deep_thinking: false,
        deepseek_smart_search: false,
        project_map_path: None,
        project_map_extra_paths: Vec::new(),
    })
}
