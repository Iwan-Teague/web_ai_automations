use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::thread;
use std::time::Duration;
use rand::Rng;
use chrono::Local;

fn run_cmd(cmd: &str, args: &[&str], cwd: &PathBuf) -> Result<String, String> {
    let output = Command::new(cmd)
        .args(args)
        .current_dir(cwd)
        .output()
        .map_err(|e| format!("Failed to execute {}: {}", cmd, e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("{} failed: {}", cmd, stderr));
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn init_git(repo_path: &PathBuf) -> Result<(), String> {
    println!("[{}] Initializing git repo...", Local::now().format("%H:%M:%S"));

    run_cmd("git", &["init"], repo_path)?;
    run_cmd("git", &["config", "user.name", "Iwan Teague"], repo_path)?;
    run_cmd("git", &["config", "user.email", "iwanteague@gmail.com"], repo_path)?;
    run_cmd(
        "git",
        &["remote", "add", "origin", "https://github.com/Iwan-Teague/web_ai_automations.git"],
        repo_path,
    )?;

    Ok(())
}

fn get_all_files(repo_path: &PathBuf) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();

    let exclude_dirs = [".git", "target", ".DS_Store", "outputs", "session", "node_modules"];

    fn collect_files(
        dir: &PathBuf,
        files: &mut Vec<PathBuf>,
        exclude: &[&str],
    ) -> Result<(), String> {
        let entries = fs::read_dir(dir)
            .map_err(|e| format!("Failed to read dir {}: {}", dir.display(), e))?;

        for entry in entries {
            let entry = entry.map_err(|e| format!("Dir entry error: {}", e))?;
            let path = entry.path();
            let file_name = path.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("");

            if exclude.contains(&file_name) {
                continue;
            }

            if path.is_dir() {
                collect_files(&path, files, exclude)?;
            } else {
                files.push(path);
            }
        }
        Ok(())
    }

    collect_files(repo_path, &mut files, &exclude_dirs)?;
    files.sort();
    Ok(files)
}

fn stage_and_commit(
    repo_path: &PathBuf,
    files: &[PathBuf],
    batch_idx: usize,
    total_batches: usize,
) -> Result<(), String> {
    let start_idx = (batch_idx * files.len()) / total_batches;
    let end_idx = ((batch_idx + 1) * files.len()) / total_batches;

    if start_idx >= end_idx {
        return Ok(());
    }

    for file_path in &files[start_idx..end_idx] {
        let rel_path = file_path
            .strip_prefix(repo_path)
            .unwrap_or(file_path)
            .to_string_lossy();

        run_cmd("git", &["add", &rel_path], repo_path)?;
    }

    let commit_msg = format!("Add code batch {}/{}", batch_idx + 1, total_batches);
    run_cmd("git", &["commit", "-m", &commit_msg], repo_path)?;

    println!("[{}] Created commit {}/{}: {}",
        Local::now().format("%H:%M:%S"),
        batch_idx + 1,
        total_batches,
        commit_msg
    );

    Ok(())
}

fn main() {
    let repo_path = PathBuf::from("/Users/iwan/Desktop/web_ai_automation");

    if let Err(e) = run_scheduler(&repo_path) {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}

fn run_scheduler(repo_path: &PathBuf) -> Result<(), String> {
    println!("[{}] Starting commit scheduler", Local::now().format("%H:%M:%S"));

    init_git(repo_path)?;

    let files = get_all_files(repo_path)?;
    println!("[{}] Found {} files to commit", Local::now().format("%H:%M:%S"), files.len());

    let num_commits = 20;
    let mut rng = rand::thread_rng();

    for batch in 0..num_commits {
        if batch > 0 {
            let delay_secs = rng.gen_range(600..2400); // 10-40 minutes
            let delay_mins = delay_secs / 60;
            println!("[{}] Waiting {} minutes before next commit...",
                Local::now().format("%H:%M:%S"),
                delay_mins
            );
            thread::sleep(Duration::from_secs(delay_secs));
        }

        stage_and_commit(repo_path, &files, batch, num_commits)?;
    }

    println!("[{}] All commits created. Pushing to GitHub...", Local::now().format("%H:%M:%S"));

    run_cmd("git", &["branch", "-M", "main"], repo_path)?;
    run_cmd("git", &["push", "-u", "origin", "main"], repo_path)?;

    println!("[{}] Push complete! All {} commits sent to GitHub.",
        Local::now().format("%H:%M:%S"),
        num_commits
    );

    Ok(())
}
