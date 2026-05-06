use std::error::Error;

use crate::intake::{banner, footer, read_bool, read_line, read_usize};
use crate::session::{DebateStrength, ReviewMode};

pub struct Stage5Data {
    pub review_mode: ReviewMode,
    pub debate_rounds: usize,
    pub bull_strength: DebateStrength,
    pub bear_strength: DebateStrength,
    pub auto_apply_judge: bool,
}

pub fn run() -> Result<Stage5Data, Box<dyn Error>> {
    banner("STAGE 5 OF 5 — REVIEW MODE");

    println!("Choose how output is reviewed:");
    println!("  1) Standard   — AI reviews its own output once");
    println!("  2) Debate     — Bull / Bear / Judge");
    println!("  3) None       — raw output only");

    let review_mode = loop {
        match read_line("Choose 1-3: ")?.as_str() {
            "1" => break ReviewMode::Standard,
            "2" => break ReviewMode::Debate,
            "3" => break ReviewMode::None,
            _ => println!("  please enter 1, 2, or 3"),
        }
    };

    // Defaults — used only when review_mode != Debate.
    let mut debate_rounds = 0usize;
    let mut bull_strength = DebateStrength::Balanced;
    let mut bear_strength = DebateStrength::Balanced;
    let mut auto_apply_judge = false;

    if review_mode == ReviewMode::Debate {
        debate_rounds = loop {
            let v = read_usize("Debate rounds", 1)?;
            if v >= 1 {
                break v;
            }
            println!("  debate_rounds must be at least 1.");
        };

        bull_strength = read_strength("Bull stance strength")?;
        bear_strength = read_strength("Bear stance strength")?;

        auto_apply_judge = read_bool(
            "Auto-apply Judge recommendations into the next iteration?",
            false,
        )?;
    }

    footer();
    Ok(Stage5Data {
        review_mode,
        debate_rounds,
        bull_strength,
        bear_strength,
        auto_apply_judge,
    })
}

fn read_strength(prompt: &str) -> Result<DebateStrength, Box<dyn Error>> {
    println!("{prompt}:");
    println!("  1) Mild");
    println!("  2) Balanced");
    println!("  3) Strong");
    println!("  4) Devil's Advocate");

    loop {
        match read_line("Choose 1-4: ")?.as_str() {
            "1" => return Ok(DebateStrength::Mild),
            "2" => return Ok(DebateStrength::Balanced),
            "3" => return Ok(DebateStrength::Strong),
            "4" => return Ok(DebateStrength::DevilsAdvocate),
            _ => println!("  please enter 1, 2, 3, or 4"),
        }
    }
}
