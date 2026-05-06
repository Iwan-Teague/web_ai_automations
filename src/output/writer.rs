use std::error::Error;
use std::fs;
use std::path::PathBuf;
use std::sync::OnceLock;

/// Root directory for all session outputs.
pub const OUTPUTS_ROOT: &str = "outputs";
pub const RUNS_DIR: &str = "runs";
pub const RESULTS_DIR: &str = "results";
pub const SCREENSHOTS_DIR: &str = "screenshots";

static RUN_ROOT: OnceLock<PathBuf> = OnceLock::new();

pub fn run_root() -> PathBuf {
    RUN_ROOT
        .get_or_init(|| {
            let stamp = chrono::Local::now().format("run_%Y%m%d_%H%M%S").to_string();
            PathBuf::from(OUTPUTS_ROOT).join(RUNS_DIR).join(stamp)
        })
        .clone()
}

pub fn results_root() -> PathBuf {
    run_root().join(RESULTS_DIR)
}

pub fn screenshots_root() -> PathBuf {
    run_root().join(SCREENSHOTS_DIR)
}

pub fn screenshot_path(name: impl AsRef<str>) -> PathBuf {
    screenshots_root().join(name.as_ref())
}

pub fn ensure_run_dirs() -> Result<PathBuf, Box<dyn Error>> {
    let run = run_root();
    fs::create_dir_all(run.join(RESULTS_DIR))?;
    fs::create_dir_all(run.join(SCREENSHOTS_DIR))?;
    Ok(run)
}

/// `outputs/runs/<run>/results/task_{n}/iter_{i}_raw.md`
pub fn save_raw(
    task_index: usize,
    iteration: usize,
    content: &str,
) -> Result<PathBuf, Box<dyn Error>> {
    let path = task_dir(task_index).join(format!("iter_{}_raw.md", iteration + 1));
    write_file(&path, content)?;
    Ok(path)
}

/// `outputs/runs/<run>/results/task_{n}/iter_{i}_self_review.md`
pub fn save_self_review(
    task_index: usize,
    iteration: usize,
    content: &str,
) -> Result<PathBuf, Box<dyn Error>> {
    let path = task_dir(task_index).join(format!("iter_{}_self_review.md", iteration + 1));
    write_file(&path, content)?;
    Ok(path)
}

/// `outputs/runs/<run>/results/task_{n}/iter_{i}_debate/round_{r}_{role}.md`
pub fn save_debate_round(
    task_index: usize,
    iteration: usize,
    round: usize,
    role: DebateRole,
    content: &str,
) -> Result<PathBuf, Box<dyn Error>> {
    let dir = task_dir(task_index).join(format!("iter_{}_debate", iteration + 1));
    let file = format!("round_{}_{}.md", round + 1, role.as_str());
    let path = dir.join(file);
    write_file(&path, content)?;
    Ok(path)
}

#[derive(Clone, Copy, Debug)]
pub enum DebateRole {
    Bull,
    Bear,
    Judge,
}

impl DebateRole {
    pub fn as_str(self) -> &'static str {
        match self {
            DebateRole::Bull => "bull",
            DebateRole::Bear => "bear",
            DebateRole::Judge => "judge",
        }
    }
}

pub fn task_dir(task_index: usize) -> PathBuf {
    results_root().join(format!("task_{}", task_index + 1))
}

fn write_file(path: &PathBuf, content: &str) -> Result<(), Box<dyn Error>> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::write(path, content)?;
    Ok(())
}
