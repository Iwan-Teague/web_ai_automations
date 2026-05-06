use std::error::Error;
use std::fmt;
use std::fs::{self, File};
use std::io::{BufReader, BufWriter};
use std::path::Path;

use serde::{Deserialize, Serialize};

/// Canonical on-disk location for the session config.
pub const SESSION_PATH: &str = "session/session.json";

pub const ARENA_DEFAULT_MODEL: &str = "deepseek-v4-pro-thinking";
pub const ARENA_COMMON_MODELS: [&str; 4] = [
    "deepseek-v4-pro-thinking",
    "claude-sonnet-4.6",
    "kimi-k2-thinking-turbo",
    "glm-5.1",
];
pub const ARENA_CODE_ONLY_MODELS: [&str; 2] = ["qwen3.5-122b-a10b-code", "kimi-2.6"];

pub fn arena_models_for_mode(mode: PromptMode) -> Vec<&'static str> {
    let mut models = ARENA_COMMON_MODELS.to_vec();
    if mode == PromptMode::Coding {
        models.extend(ARENA_CODE_ONLY_MODELS);
    }
    models
}

pub fn arena_model_is_code_only(model: &str) -> bool {
    ARENA_CODE_ONLY_MODELS
        .iter()
        .any(|m| m.eq_ignore_ascii_case(model))
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SessionConfig {
    // Stage 1 — Project Source
    pub repo_url: String,
    pub branch: Option<String>,
    pub content_type: ContentType,

    // Stage 2 — End Goal
    pub end_goal: String,
    pub success_criteria: String,

    // Stage 3 — Constraints & Context
    pub hard_constraints: Vec<String>,
    pub background_context: String,
    pub tech_constraints: String,

    // Stage 4 — Tasks
    pub tasks: Vec<String>,
    pub iterations_per_task: usize,
    pub auto_continue: bool,

    // Stage 5 — Review Mode
    pub review_mode: ReviewMode,
    pub debate_rounds: usize,
    pub bull_strength: DebateStrength,
    pub bear_strength: DebateStrength,
    pub auto_apply_judge: bool,

    /// New good-cop / bad-cop debate workflow requested for the simplified
    /// Arena runner. Kept separate from legacy `ReviewMode::Debate` because
    /// the legacy path runs in the current chat and includes a judge.
    #[serde(default)]
    pub debate_mode_enabled: bool,

    /// Prompt mode — changes the tone and structure of prompts.
    #[serde(default)]
    pub prompt_mode: PromptMode,

    /// Optional coding focus. Only affects prompt wording when
    /// `prompt_mode == Coding`; it does not alter browser navigation.
    #[serde(default)]
    pub coding_specialty: CodingSpecialty,

    /// Optional research focus. Reuses the same work categories as coding
    /// so research prompts can request reports about code-oriented areas.
    #[serde(default)]
    pub research_specialty: CodingSpecialty,

    /// Which web UI to drive. Older session.json files predating this field
    /// fall back to Arena.
    #[serde(default)]
    pub target_webui: WebUiTarget,

    /// Arena Direct-mode model to select (e.g. `"claude-sonnet-4-5"`).
    /// `None` → leave whatever model is already selected.
    #[serde(default)]
    pub arena_model: Option<String>,

    /// DeepSeek mode selector. Only used when `target_webui == DeepSeek`.
    #[serde(default)]
    pub deepseek_model: DeepSeekModelMode,

    /// DeepSeek "Deep thinking" toggle. Only used by the DeepSeek adapter.
    #[serde(default)]
    pub deepseek_deep_thinking: bool,

    /// DeepSeek "Smart Search" toggle. Only used by the DeepSeek adapter.
    #[serde(default)]
    pub deepseek_smart_search: bool,

    /// Absolute path to a generated Rust project map `.txt` file.
    /// When set, the runner uploads it to the web UI before the first prompt.
    #[serde(default)]
    pub project_map_path: Option<String>,

    /// Additional project-map files to upload alongside `project_map_path`.
    /// Populated when the project is large enough that `project_map::generate`
    /// splits the output per crate. Empty for single-file maps.
    #[serde(default)]
    pub project_map_extra_paths: Vec<String>,
}

impl SessionConfig {
    /// All project-map files to upload (primary + extras), in upload order.
    pub fn all_project_map_paths(&self) -> Vec<&str> {
        let mut v: Vec<&str> = Vec::new();
        if let Some(p) = self.project_map_path.as_deref() {
            v.push(p);
        }
        v.extend(self.project_map_extra_paths.iter().map(|s| s.as_str()));
        v
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum WebUiTarget {
    Arena,
    ChatGpt,
    Claude,
    DeepSeek,
    /// Reads/writes nothing; the FSM runs end-to-end with stub responses.
    /// Used for tests and any non-Windows host that has no real adapter wired.
    Null,
}

impl Default for WebUiTarget {
    fn default() -> Self {
        WebUiTarget::Arena
    }
}

impl WebUiTarget {
    pub fn adapter_name(self) -> &'static str {
        match self {
            WebUiTarget::Arena => "arena",
            WebUiTarget::ChatGpt => "chatgpt",
            WebUiTarget::Claude => "claude",
            WebUiTarget::DeepSeek => "deepseek",
            WebUiTarget::Null => "null",
        }
    }

    pub fn is_implemented(self) -> bool {
        matches!(
            self,
            WebUiTarget::Arena | WebUiTarget::DeepSeek | WebUiTarget::Null
        )
    }
}

impl fmt::Display for WebUiTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            WebUiTarget::Arena => "arena.ai",
            WebUiTarget::ChatGpt => "ChatGPT",
            WebUiTarget::Claude => "Claude",
            WebUiTarget::DeepSeek => "DeepSeek",
            WebUiTarget::Null => "Null",
        };
        f.write_str(s)
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptMode {
    Research,
    Coding,
}

impl Default for PromptMode {
    fn default() -> Self {
        PromptMode::Research
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeepSeekModelMode {
    Instant,
    Expert,
}

impl Default for DeepSeekModelMode {
    fn default() -> Self {
        Self::Instant
    }
}

impl fmt::Display for DeepSeekModelMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DeepSeekModelMode::Instant => f.write_str("Instant"),
            DeepSeekModelMode::Expert => f.write_str("Expert"),
        }
    }
}

impl fmt::Display for PromptMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PromptMode::Research => f.write_str("Research"),
            PromptMode::Coding => f.write_str("Coding"),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodingSpecialty {
    General,
    WebDesign,
    Backend,
    FullStackApplication,
    Networking,
    BugFinding,
    TestingQa,
    RefactorArchitecture,
    DevOpsTooling,
}

impl Default for CodingSpecialty {
    fn default() -> Self {
        CodingSpecialty::General
    }
}

impl fmt::Display for CodingSpecialty {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            CodingSpecialty::General => "General",
            CodingSpecialty::WebDesign => "Web Design",
            CodingSpecialty::Backend => "Backend",
            CodingSpecialty::FullStackApplication => "Full Stack Application",
            CodingSpecialty::Networking => "Networking",
            CodingSpecialty::BugFinding => "Bug Finding",
            CodingSpecialty::TestingQa => "Testing and QA",
            CodingSpecialty::RefactorArchitecture => "Refactor and Architecture",
            CodingSpecialty::DevOpsTooling => "DevOps and Tooling",
        };
        f.write_str(s)
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentType {
    RustCode,
    OtherCode,
    Document,
    Mixed,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewMode {
    Standard,
    Debate,
    None,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum DebateStrength {
    Mild,
    Balanced,
    Strong,
    DevilsAdvocate,
}

impl fmt::Display for DebateStrength {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            DebateStrength::Mild => "Mild",
            DebateStrength::Balanced => "Balanced",
            DebateStrength::Strong => "Strong",
            DebateStrength::DevilsAdvocate => "Devil's Advocate",
        };
        f.write_str(s)
    }
}

impl SessionConfig {
    /// Read a `SessionConfig` from JSON at `path`.
    pub fn load(path: &str) -> Result<Self, Box<dyn Error>> {
        let file = File::open(path)?;
        let reader = BufReader::new(file);
        let mut cfg: SessionConfig = serde_json::from_reader(reader)?;
        if cfg.prompt_mode == PromptMode::Research
            && cfg
                .arena_model
                .as_deref()
                .is_some_and(arena_model_is_code_only)
        {
            let old = cfg.arena_model.as_deref().unwrap_or_default();
            eprintln!(
                "Research mode cannot use code-only Arena model '{old}'. Falling back to '{ARENA_DEFAULT_MODEL}'."
            );
            cfg.arena_model = Some(ARENA_DEFAULT_MODEL.to_string());
        }
        Ok(cfg)
    }

    /// Write this `SessionConfig` to JSON at `path`. Creates the parent directory.
    pub fn save(&self, path: &str) -> Result<(), Box<dyn Error>> {
        if let Some(dir) = Path::new(path).parent() {
            fs::create_dir_all(dir)?;
        }
        let file = File::create(path)?;
        let writer = BufWriter::new(file);
        serde_json::to_writer_pretty(writer, self)?;
        Ok(())
    }
}
