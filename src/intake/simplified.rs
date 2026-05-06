use std::error::Error;
use std::path::PathBuf;

use super::{banner, footer, read_line, read_multiline};
use crate::project_map;
use crate::session::{
    CodingSpecialty, ContentType, DebateStrength, DeepSeekModelMode, PromptMode, ReviewMode,
    SessionConfig, WebUiTarget, arena_models_for_mode,
};

pub fn run() -> Result<SessionConfig, Box<dyn Error>> {
    banner("Quick Setup");
    println!("║                                                                  ║");
    println!("║  Mode → Site → Goal → Constraints → Context → Repo Map → Model  ║");
    footer();

    // 1. Mode
    println!("\nSelect mode:");
    println!("  1) Research  — analysis, investigation, structured output");
    println!("  2) Coding    — implementation, code generation, debugging");
    let mode = loop {
        let raw = read_line("Mode [1]: ")?;
        match raw.as_str() {
            "" | "1" | "research" => break PromptMode::Research,
            "2" | "coding" => break PromptMode::Coding,
            _ => println!("  enter 1 or 2"),
        }
    };
    println!("  → {mode}\n");

    let target_webui = read_site()?;

    let coding_specialty = if mode == PromptMode::Coding {
        read_focus("Coding focus")?
    } else {
        CodingSpecialty::General
    };
    let research_specialty = if mode == PromptMode::Research {
        read_focus("Research focus")?
    } else {
        CodingSpecialty::General
    };

    // 2. Goal
    let goal = read_multiline("Goal — what should the AI accomplish?")?;
    if goal.is_empty() {
        return Err("goal cannot be empty".into());
    }

    // 3. Constraints
    let constraints_raw =
        read_multiline("Constraints — rules the AI must follow (leave blank for none)")?;
    let constraints: Vec<String> = if constraints_raw.is_empty() {
        vec![]
    } else {
        constraints_raw
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect()
    };

    // 4. Context
    let context = read_multiline(
        "Context — source-of-truth project background, architecture, assumptions, or layout",
    )?;
    if context.is_empty() {
        return Err("context cannot be empty".into());
    }

    // 5. Optional Rust project map (may be split per crate for large workspaces)
    let (project_map_path, project_map_extra_paths) = read_project_map()?;

    // 5. Site-specific model
    let arena_model = if target_webui == WebUiTarget::Arena {
        Some(read_arena_model(mode)?)
    } else {
        None
    };
    let (deepseek_model, deepseek_deep_thinking, deepseek_smart_search) =
        if target_webui == WebUiTarget::DeepSeek {
            read_deepseek_settings()?
        } else {
            (DeepSeekModelMode::default(), false, false)
        };

    // 6. Iterations (total prompts in the main chat). Entering 40 means
    // exactly 40 prompts: 1 initial prompt + 39 follow-ups.
    let iters_raw = read_line("Prompt iterations (total prompts; 1 = just initial) [2]: ")?;
    let iterations: usize = if iters_raw.is_empty() {
        2
    } else {
        iters_raw.parse().unwrap_or(2).max(1)
    };

    let debate_enabled = if mode == PromptMode::Research {
        let debate_raw =
            read_line("Debate mode? good-cop/bad-cop reviews for every research report [y/N]: ")?
                .to_lowercase();
        matches!(debate_raw.as_str(), "y" | "yes" | "true" | "1")
    } else {
        println!("Debate mode is only available in Research mode.");
        false
    };
    if debate_enabled {
        println!(
            "  → Debate mode recorded. New-chat debate orchestration is documented but not enabled yet."
        );
    }

    let content_type = match mode {
        PromptMode::Coding => ContentType::OtherCode,
        PromptMode::Research => ContentType::Document,
    };

    Ok(SessionConfig {
        repo_url: String::new(),
        branch: None,
        content_type,

        end_goal: goal.clone(),
        success_criteria: String::new(),

        hard_constraints: constraints,
        background_context: context,
        tech_constraints: String::new(),

        tasks: vec![goal],
        iterations_per_task: iterations,
        auto_continue: true,

        review_mode: ReviewMode::None,
        debate_rounds: if debate_enabled { 1 } else { 0 },
        bull_strength: DebateStrength::Balanced,
        bear_strength: DebateStrength::Balanced,
        auto_apply_judge: false,
        debate_mode_enabled: debate_enabled,

        prompt_mode: mode,
        coding_specialty,
        research_specialty,
        target_webui,
        arena_model,
        deepseek_model,
        deepseek_deep_thinking,
        deepseek_smart_search,
        project_map_path,
        project_map_extra_paths,
    })
}

fn read_site() -> Result<WebUiTarget, Box<dyn Error>> {
    println!("Select site:");
    println!("  1) arena.ai");
    println!("  2) Null adapter — local smoke test");
    println!("  3) ChatGPT — not implemented yet");
    println!("  4) Claude — not implemented yet");
    println!("  5) DeepSeek");

    let site = loop {
        let raw = read_line("Site [1]: ")?.to_lowercase();
        let selected = match raw.as_str() {
            "" | "1" | "arena" | "arena.ai" => WebUiTarget::Arena,
            "2" | "null" => WebUiTarget::Null,
            "3" | "chatgpt" | "chat gpt" => WebUiTarget::ChatGpt,
            "4" | "claude" => WebUiTarget::Claude,
            "5" | "deepseek" | "deep seek" => WebUiTarget::DeepSeek,
            _ => {
                println!("  enter 1-5");
                continue;
            }
        };

        if selected.is_implemented() {
            break selected;
        }
        println!("  {selected} is listed for future support, but no adapter exists yet.");
    };

    println!("  → {site}\n");
    Ok(site)
}

fn read_deepseek_settings() -> Result<(DeepSeekModelMode, bool, bool), Box<dyn Error>> {
    println!("Select DeepSeek model:");
    println!("  1) Instant");
    println!("  2) Expert");
    let model = loop {
        let raw = read_line("DeepSeek model [1]: ")?.to_lowercase();
        match raw.as_str() {
            "" | "1" | "instant" => break DeepSeekModelMode::Instant,
            "2" | "expert" => break DeepSeekModelMode::Expert,
            _ => println!("  enter 1 or 2"),
        }
    };

    let deep_thinking = read_bool("Deep thinking? [y/N]: ")?;
    let smart_search = read_bool("Smart Search? [y/N]: ")?;
    println!("  → DeepSeek {model}, deep_thinking={deep_thinking}, smart_search={smart_search}\n");
    Ok((model, deep_thinking, smart_search))
}

fn read_bool(prompt: &str) -> Result<bool, Box<dyn Error>> {
    let raw = read_line(prompt)?.to_lowercase();
    Ok(matches!(raw.as_str(), "y" | "yes" | "true" | "1"))
}

fn read_focus(prompt: &str) -> Result<CodingSpecialty, Box<dyn Error>> {
    println!("Select {prompt}:");
    println!("  1) General");
    println!("  2) Web Design");
    println!("  3) Backend");
    println!("  4) Full Stack Application");
    println!("  5) Networking");
    println!("  6) Bug Finding");
    println!("  7) Testing and QA");
    println!("  8) Refactor and Architecture");
    println!("  9) DevOps and Tooling");
    let focus = loop {
        let raw = read_line(&format!("{prompt} [1]: "))?.to_lowercase();
        match raw.as_str() {
            "" | "1" | "general" => break CodingSpecialty::General,
            "2" | "web" | "web design" => break CodingSpecialty::WebDesign,
            "3" | "backend" => break CodingSpecialty::Backend,
            "4" | "full stack" | "application" | "app" => {
                break CodingSpecialty::FullStackApplication;
            }
            "5" | "networking" => break CodingSpecialty::Networking,
            "6" | "bug" | "bug finding" => break CodingSpecialty::BugFinding,
            "7" | "testing" | "qa" => break CodingSpecialty::TestingQa,
            "8" | "refactor" | "architecture" => break CodingSpecialty::RefactorArchitecture,
            "9" | "devops" | "tooling" => break CodingSpecialty::DevOpsTooling,
            _ => println!("  enter 1-9"),
        }
    };
    println!("  → {focus}\n");
    Ok(focus)
}

fn read_arena_model(mode: PromptMode) -> Result<String, Box<dyn Error>> {
    let models = arena_models_for_mode(mode);
    println!("Select Arena model:");
    for (idx, model) in models.iter().enumerate() {
        println!("  {}) {}", idx + 1, model);
    }
    if mode == PromptMode::Research {
        println!("  (Code-only models hidden in Research mode.)");
    }
    println!("  Or type a custom model name exactly as Arena shows it.");

    let raw = read_line("Arena model [1]: ")?;
    if raw.is_empty() {
        return Ok(models[0].to_string());
    }
    if let Ok(n) = raw.parse::<usize>() {
        if (1..=models.len()).contains(&n) {
            return Ok(models[n - 1].to_string());
        }
    }
    Ok(raw)
}

/// Returns (primary_path, extra_paths). For small projects, extras is empty.
/// For large workspaces, the map is split per crate — primary is the first
/// written file (workspace overview) and extras are the per-crate maps.
fn read_project_map() -> Result<(Option<String>, Vec<String>), Box<dyn Error>> {
    let want =
        read_bool("Include Rust project map? (uploads project files to AI as context) [y/N]: ")?;
    if !want {
        return Ok((None, Vec::new()));
    }

    let dir_raw = read_line("Project directory path: ")?;
    if dir_raw.is_empty() {
        println!("  → no path entered, skipping project map");
        return Ok((None, Vec::new()));
    }

    let root = PathBuf::from(&dir_raw);
    let out_path = PathBuf::from("session/rust_project_map.txt");

    print!("  Scanning {} … ", root.display());
    std::io::Write::flush(&mut std::io::stdout())?;

    match project_map::generate(&root, &out_path) {
        Ok(map) => {
            let mut paths_iter = map.paths.into_iter();
            let primary = paths_iter
                .next()
                .expect("generate returns at least one path");
            let extras: Vec<String> = paths_iter
                .map(|p| p.to_string_lossy().into_owned())
                .collect();
            let split_note = if extras.is_empty() {
                String::new()
            } else {
                format!(" (split into {} files)", extras.len() + 1)
            };
            println!(
                "{} files → {}{}",
                map.file_count,
                primary.display(),
                split_note
            );
            for extra in &extras {
                println!("    + {extra}");
            }
            Ok((Some(primary.to_string_lossy().into_owned()), extras))
        }
        Err(e) => {
            println!("FAILED: {e}");
            println!("  Continuing without project map.");
            Ok((None, Vec::new()))
        }
    }
}
