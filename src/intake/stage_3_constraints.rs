use std::error::Error;

use crate::intake::{banner, footer, read_list, read_multiline};

pub struct Stage3Data {
    pub hard_constraints: Vec<String>,
    pub background_context: String,
    pub tech_constraints: String,
}

pub fn run() -> Result<Stage3Data, Box<dyn Error>> {
    banner("STAGE 3 OF 5 — CONSTRAINTS & CONTEXT");

    let hard_constraints = loop {
        let v = read_list("What must the AI never do or suggest? (hard constraints)")?;
        if !v.is_empty() {
            break v;
        }
        println!("  at least one hard_constraint is required.");
    };

    let background_context =
        read_multiline("Background context the AI needs to understand your project")?;

    let tech_constraints = read_multiline("Tech stack / language constraints")?;

    footer();
    Ok(Stage3Data {
        hard_constraints,
        background_context,
        tech_constraints,
    })
}
