//! Bull → Bear → Judge orchestration. Drives every debate prompt through
//! the same FSM the task runner uses, so debate prompts get identical
//! resilience guarantees (recovery, watchdog, transcript, resume).

use std::error::Error;
use std::sync::Arc;

use crate::output::writer::{DebateRole, save_debate_round};
use crate::prompt::debate::{build_bear_prompt, build_bull_prompt, build_judge_prompt};
use crate::session::SessionConfig;
use crate::webui::adapter::WebUiAdapter;
use crate::webui::fsm::{FsmConfig, FsmContext, run_iteration};
use crate::webui::recovery::{RecoveryDispatcher, SiteHints, default_actions};
use crate::webui::state_log::StateLog;
use crate::webui::watchdog::default_invariants;

/// What the runner extracts from one debate round.
pub struct DebateOutcome {
    pub bull_response: String,
    pub bear_response: String,
    pub judge_response: String,
    pub verdict: Option<String>,
    pub score: Option<f32>,
}

/// Run all `cfg.debate_rounds` of bull → bear → judge for one task iteration.
///
/// Returns the outcome of the **final** round.
/// Per-round files are saved inside `outputs/runs/<run>/results/task_n/iter_i_debate/`.
///
/// Note: this top-level signature stays compatible with the old call site
/// in `task_runner` — it builds its own adapter internally so debates work
/// even when the caller doesn't have a live adapter handle.
pub fn run_debate(
    cfg: &SessionConfig,
    task_index: usize,
    iteration: usize,
    task_output: &str,
) -> Result<DebateOutcome, Box<dyn Error>> {
    let (adapter, site) =
        crate::runner::site_workflow::build_debate_adapter(cfg.target_webui, cfg)?;
    run_debate_with(&adapter, &site, cfg, task_index, iteration, task_output)
}

/// Same as `run_debate` but lets the caller share an existing adapter.
/// Cheaper when several debates run in the same session — only one
/// `WebUiAdapter` instance is constructed.
pub fn run_debate_with(
    adapter: &Arc<dyn WebUiAdapter>,
    site: &SiteHints,
    cfg: &SessionConfig,
    task_index: usize,
    iteration: usize,
    task_output: &str,
) -> Result<DebateOutcome, Box<dyn Error>> {
    let mut last: Option<DebateOutcome> = None;
    let fsm_cfg = FsmConfig::default();

    for round in 0..cfg.debate_rounds {
        // Bull
        let bull_prompt = build_bull_prompt(cfg, task_output);
        let bull_response = run_one(adapter, site, task_index, iteration, &fsm_cfg, &bull_prompt)?;
        save_debate_round(
            task_index,
            iteration,
            round,
            DebateRole::Bull,
            &bull_response,
        )?;

        // Bear
        let bear_prompt = build_bear_prompt(cfg, task_output);
        let bear_response = run_one(adapter, site, task_index, iteration, &fsm_cfg, &bear_prompt)?;
        save_debate_round(
            task_index,
            iteration,
            round,
            DebateRole::Bear,
            &bear_response,
        )?;

        // Judge
        let judge_prompt =
            build_judge_prompt(cfg, round, task_output, &bull_response, &bear_response);
        let judge_response = run_one(
            adapter,
            site,
            task_index,
            iteration,
            &fsm_cfg,
            &judge_prompt,
        )?;
        save_debate_round(
            task_index,
            iteration,
            round,
            DebateRole::Judge,
            &judge_response,
        )?;

        last = Some(DebateOutcome {
            verdict: parse_verdict(&judge_response),
            score: parse_score(&judge_response),
            bull_response,
            bear_response,
            judge_response,
        });
    }

    last.ok_or_else(|| "debate_rounds was 0 — no rounds executed".into())
}

fn run_one(
    adapter: &Arc<dyn WebUiAdapter>,
    site: &SiteHints,
    task_index: usize,
    iteration: usize,
    fsm_cfg: &FsmConfig,
    prompt: &str,
) -> Result<String, Box<dyn Error>> {
    let ctx = FsmContext {
        adapter: Arc::clone(adapter),
        invariants: default_invariants(
            site.browser_title_substr,
            site.site_url,
            site.site_title_substr,
        ),
        recovery: RecoveryDispatcher::new(default_actions()),
        site: site.clone(),
        state_log: StateLog::open_default()?,
        task_index,
        iteration,
        config: fsm_cfg.clone(),
    };
    let resp = run_iteration(&ctx, prompt)?;
    Ok(resp)
}

/// Look for ACCEPT / REJECT / REVISE in the judge text. Case-insensitive.
fn parse_verdict(judge: &str) -> Option<String> {
    let upper = judge.to_uppercase();
    for v in ["ACCEPT", "REJECT", "REVISE"] {
        if upper.contains(v) {
            return Some(v.to_string());
        }
    }
    None
}

/// Look for "score: X/10" or "X out of 10" patterns. Returns the first match.
fn parse_score(judge: &str) -> Option<f32> {
    let lower = judge.to_lowercase();

    if let Some(idx) = lower.find("/10") {
        let head = &lower[..idx];
        let num: String = head
            .chars()
            .rev()
            .take_while(|c| c.is_ascii_digit() || *c == '.')
            .collect::<String>()
            .chars()
            .rev()
            .collect();
        if let Ok(v) = num.parse::<f32>() {
            return Some(v);
        }
    }

    if let Some(start) = lower.find("score:") {
        let tail = &lower[start + "score:".len()..];
        let num: String = tail
            .chars()
            .skip_while(|c| !c.is_ascii_digit() && *c != '.')
            .take_while(|c| c.is_ascii_digit() || *c == '.')
            .collect();
        if let Ok(v) = num.parse::<f32>() {
            return Some(v);
        }
    }

    None
}
