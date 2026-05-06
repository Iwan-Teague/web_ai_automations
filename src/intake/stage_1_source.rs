use std::error::Error;

use crate::intake::{banner, footer, read_line};
use crate::session::ContentType;

pub struct Stage1Data {
    pub repo_url: String,
    pub branch: Option<String>,
    pub content_type: ContentType,
}

pub fn run() -> Result<Stage1Data, Box<dyn Error>> {
    banner("STAGE 1 OF 5 — PROJECT SOURCE");

    let repo_url = loop {
        let v = read_line("Repository URL or local path: ")?;
        if !v.is_empty() {
            break v;
        }
        println!("  repo_url is required.");
    };

    let branch_raw = read_line("Branch or commit (blank = default): ")?;
    let branch = if branch_raw.is_empty() {
        None
    } else {
        Some(branch_raw)
    };

    println!("Content type:");
    println!("  1) Rust codebase");
    println!("  2) Other code");
    println!("  3) Document / spec");
    println!("  4) Mixed");

    let content_type = loop {
        match read_line("Choose 1-4: ")?.as_str() {
            "1" => break ContentType::RustCode,
            "2" => break ContentType::OtherCode,
            "3" => break ContentType::Document,
            "4" => break ContentType::Mixed,
            _ => println!("  please enter 1, 2, 3, or 4"),
        }
    };

    footer();
    Ok(Stage1Data {
        repo_url,
        branch,
        content_type,
    })
}
