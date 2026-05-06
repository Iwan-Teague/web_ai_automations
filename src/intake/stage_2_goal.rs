use std::error::Error;

use crate::intake::{banner, footer, read_multiline};

pub struct Stage2Data {
    pub end_goal: String,
    pub success_criteria: String,
}

pub fn run() -> Result<Stage2Data, Box<dyn Error>> {
    banner("STAGE 2 OF 5 — END GOAL  (★ KEY ★)");

    println!("This will be shown to the AI at the start of every prompt.");

    let end_goal = loop {
        let v = read_multiline("State your end goal clearly and specifically")?;
        if !v.is_empty() {
            break v;
        }
        println!("  end_goal is required.");
    };

    let success_criteria = loop {
        let v = read_multiline("How will you know the goal is achieved? (success criteria)")?;
        if !v.is_empty() {
            break v;
        }
        println!("  success_criteria is required.");
    };

    footer();
    Ok(Stage2Data {
        end_goal,
        success_criteria,
    })
}
