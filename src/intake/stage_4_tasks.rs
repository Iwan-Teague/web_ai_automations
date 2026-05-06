use std::error::Error;

use crate::intake::{banner, footer, read_bool, read_list, read_usize};

pub struct Stage4Data {
    pub tasks: Vec<String>,
    pub iterations_per_task: usize,
    pub auto_continue: bool,
}

pub fn run() -> Result<Stage4Data, Box<dyn Error>> {
    banner("STAGE 4 OF 5 — TASKS");

    let tasks = loop {
        let v = read_list("List the specific tasks for the AI to work through")?;
        if !v.is_empty() {
            break v;
        }
        println!("  at least one task is required.");
    };

    let iterations_per_task = loop {
        let v = read_usize("How many prompt iterations per task?", 1)?;
        if v >= 1 {
            break v;
        }
        println!("  iterations_per_task must be at least 1.");
    };

    let auto_continue = !read_bool("Pause for confirmation between iterations?", true)?;

    footer();
    Ok(Stage4Data {
        tasks,
        iterations_per_task,
        auto_continue,
    })
}
