use std::error::Error;
use std::path::Path;

use web_ai_automation::calibration;
use web_ai_automation::intake;
use web_ai_automation::runner;
use web_ai_automation::session::{SESSION_PATH, SessionConfig};

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let subcommand = args.first().map(String::as_str).unwrap_or("");

    match subcommand {
        "calibrate" | "calibrate-arena" => {
            println!("web_ai_automation — calibration mode");
            calibration::walkthrough::run()
        }
        "" | "run" | "session" => {
            println!("web_ai_automation — startup");
            let mut cfg = if Path::new(SESSION_PATH).exists() {
                println!("Found existing session at {SESSION_PATH}");
                print!("Use existing session? [Y/n]: ");
                std::io::Write::flush(&mut std::io::stdout())?;
                let mut ans = String::new();
                std::io::BufRead::read_line(&mut std::io::stdin().lock(), &mut ans)?;
                let ans = ans.trim().to_lowercase();
                if ans == "n" || ans == "no" {
                    println!("Starting new session.");
                    let cfg = intake::simplified::run()?;
                    cfg.save(SESSION_PATH)?;
                    println!("Session saved to {SESSION_PATH}");
                    cfg
                } else {
                    println!("Loading session from {SESSION_PATH}");
                    SessionConfig::load(SESSION_PATH)?
                }
            } else {
                println!("No session.json found — running quick setup.");
                let cfg = intake::simplified::run()?;
                cfg.save(SESSION_PATH)?;
                println!("Session saved to {SESSION_PATH}");
                cfg
            };
            ensure_context(&mut cfg)?;
            runner::task_runner::run_session(&cfg)
        }
        "run-full" => {
            println!("web_ai_automation — full intake mode");
            let mut cfg = if Path::new(SESSION_PATH).exists() {
                println!("Loading existing session from {SESSION_PATH}");
                SessionConfig::load(SESSION_PATH)?
            } else {
                let cfg = intake::run_intake()?;
                cfg.save(SESSION_PATH)?;
                println!("Session saved to {SESSION_PATH}");
                cfg
            };
            ensure_context(&mut cfg)?;
            runner::task_runner::run_session(&cfg)
        }
        other => {
            eprintln!("unknown subcommand: '{other}'");
            eprintln!();
            eprintln!("usage:");
            eprintln!("  cargo run                  # run a session (default)");
            eprintln!("  cargo run -- run-full       # full 5-stage intake");
            eprintln!("  cargo run -- calibrate     # capture reference matrices");
            std::process::exit(2);
        }
    }
}

fn ensure_context(cfg: &mut SessionConfig) -> Result<(), Box<dyn Error>> {
    if !cfg.background_context.trim().is_empty() {
        return Ok(());
    }

    let context = read_multiline_required(
        "Context — source-of-truth project background, architecture, assumptions, or layout",
    )?;
    cfg.background_context = context;
    cfg.save(SESSION_PATH)?;
    println!("Session updated with Context.");
    Ok(())
}

fn read_multiline_required(prompt: &str) -> Result<String, Box<dyn Error>> {
    println!("{prompt} (end with two empty lines):");
    let stdin = std::io::stdin();
    let mut out = String::new();
    let mut blank_streak = 0usize;
    loop {
        let mut line = String::new();
        std::io::BufRead::read_line(&mut stdin.lock(), &mut line)?;
        if line.trim().is_empty() {
            blank_streak += 1;
            if blank_streak >= 2 {
                break;
            }
        } else {
            blank_streak = 0;
        }
        out.push_str(&line);
    }
    let out = out.trim().to_string();
    if out.is_empty() {
        Err("context cannot be empty".into())
    } else {
        Ok(out)
    }
}
