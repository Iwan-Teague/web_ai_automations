//! Execution finite state machine.
//!
//! Drives one prompt submission from `Idle` to `Done(response)`, with
//! explicit handling for every failure path. Replaces the linear
//! "submit → wait → copy" sequence in the original runner.
//!
//! ```text
//!   Idle ─► Submitting ─► WaitingForGen ─► Generating ─► Complete ─► Copying ─► Done
//!                  ▲ ▲              │             │           │
//!                  │ │              ▼             ▼           ▼
//!                  │ └──────── Retrying ◄───── Error ────────┐
//!                  └──────── StartingNewChat ◄── ChatFull ◄──┘
//!                                  ▼
//!                               Failed
//! ```
//!
//! Every transition is logged. State must be **stable** for `state_stable_ms`
//! before the FSM accepts it — this kills render-flicker false positives.

use std::error::Error as StdError;
use std::fmt;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use crate::webui::adapter::{AdapterError, WebUiAdapter, WebUiState};
use crate::webui::recovery::{
    DispatchResult, Problem, RecoveryContext, RecoveryDispatcher, SiteHints, UnrecoveredProblem,
};
use crate::webui::runtime::RuntimeState;
use crate::webui::state_log::{StateLog, now_transition};
use crate::webui::watchdog::{Invariant, check_all};

// ── Tunables ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct FsmConfig {
    /// How long to wait after `submit` before deciding generation never started.
    pub max_wait_for_gen_ms: u64,
    /// Mandatory no-poll period after submitting. Prevents false "complete"
    /// detections from stale page/input variance right after send.
    pub post_submit_quiet_ms: u64,
    /// Hard cap on total generation time before we declare timeout.
    pub max_generation_ms: u64,
    /// Minimum time to wait before accepting Complete in the Generating
    /// state. Prevents false positives from variance-based detection on
    /// pages that haven't started rendering yet.
    pub min_generation_ms: u64,
    /// Polling interval during the generation phase (slower than the
    /// default poll to avoid thrashing on long-running generations).
    pub generation_poll_ms: u64,
    /// Maximum retries on `Error` before failing the iteration.
    pub max_retries: u8,
    /// Polling cadence — how often to ask the adapter what state we're in.
    pub poll_interval_ms: u64,
    /// A state must stay the same for this long before we accept it.
    /// Filters render-flicker between e.g. Generating and Complete.
    pub state_stable_ms: u64,
    /// Maximum number of Unknown polls before treating it as a Problem.
    pub unknown_polls_threshold: u32,
}

impl Default for FsmConfig {
    fn default() -> Self {
        Self {
            max_wait_for_gen_ms: 60_000,
            post_submit_quiet_ms: 60_000,
            max_generation_ms: 600_000,
            min_generation_ms: 0,
            generation_poll_ms: 15_000,
            max_retries: 3,
            poll_interval_ms: 500,
            state_stable_ms: 700,
            unknown_polls_threshold: 6,
        }
    }
}

// ── Execution states ────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum ExecutionState {
    Idle,
    Submitting,
    WaitingForGen,
    Generating,
    Complete,
    Copying,
    Done(String),
    Error(String),
    Retrying(u8),
    StartingNewChat,
    Failed(String),
}

impl ExecutionState {
    pub fn name(&self) -> &'static str {
        match self {
            ExecutionState::Idle => "Idle",
            ExecutionState::Submitting => "Submitting",
            ExecutionState::WaitingForGen => "WaitingForGen",
            ExecutionState::Generating => "Generating",
            ExecutionState::Complete => "Complete",
            ExecutionState::Copying => "Copying",
            ExecutionState::Done(_) => "Done",
            ExecutionState::Error(_) => "Error",
            ExecutionState::Retrying(_) => "Retrying",
            ExecutionState::StartingNewChat => "StartingNewChat",
            ExecutionState::Failed(_) => "Failed",
        }
    }
}

// ── Public error type ───────────────────────────────────────────────────────

#[derive(Debug)]
pub enum FsmError {
    /// Recovery exhausted all options.
    Unrecovered(UnrecoveredProblem),
    /// Hit the retry ceiling on `Error`.
    RetriesExhausted,
    /// A timeout fired with no state change.
    Timeout(&'static str),
    /// Adapter reported an unrecoverable error.
    Adapter(AdapterError),
    /// Disk write or other infrastructure error.
    Io(String),
}

impl fmt::Display for FsmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FsmError::Unrecovered(u) => write!(f, "{u}"),
            FsmError::RetriesExhausted => write!(f, "FSM retries exhausted"),
            FsmError::Timeout(stage) => write!(f, "FSM timeout in {stage}"),
            FsmError::Adapter(e) => write!(f, "adapter: {e}"),
            FsmError::Io(s) => write!(f, "io: {s}"),
        }
    }
}

impl StdError for FsmError {}

// ── Execution context ───────────────────────────────────────────────────────

/// Runtime context the FSM needs alongside the adapter.
pub struct FsmContext {
    pub adapter: Arc<dyn WebUiAdapter>,
    pub invariants: Vec<Box<dyn Invariant>>,
    pub recovery: RecoveryDispatcher,
    pub site: SiteHints,
    pub state_log: StateLog,
    pub task_index: usize,
    pub iteration: usize,
    pub config: FsmConfig,
}

// ── Public entry point ──────────────────────────────────────────────────────

/// Run one full prompt submission through the FSM and return the response.
///
/// The function is intentionally long — every transition is explicit so the
/// failure paths are visible. Splitting into per-state helpers would hide
/// the control flow that is the whole point of the FSM.
///
/// `previous_context`: optional text from a prior iteration's response.
/// When non-empty and the FSM retries in a new chat, this context is
/// prepended to the prompt so the AI has the previous work available.
pub fn run_iteration(ctx: &FsmContext, prompt: &str) -> Result<String, FsmError> {
    run_iteration_with_context(ctx, prompt, None)
}

/// Like `run_iteration` but with optional context from previous iterations.
pub fn run_iteration_with_context(
    ctx: &FsmContext,
    prompt: &str,
    previous_context: Option<&str>,
) -> Result<String, FsmError> {
    let mut state = ExecutionState::Idle;
    log_transition(ctx, "<start>", state.name(), None);
    persist_runtime(ctx, &state);

    let mut retries: u8 = 0;
    let mut in_new_chat = false; // tracks whether we're re-submitting in a fresh chat
    let iteration_started = Instant::now();
    let _ = iteration_started;

    loop {
        // Per-tick environment check — never trust that the world is still
        // the way we left it.
        let problems = check_all(&ctx.invariants);
        if let Some(p) = problems.into_iter().next() {
            handle_problem(ctx, &p, &mut state)?;
            continue;
        }

        match &state {
            ExecutionState::Idle => {
                // Accept Ready (fresh chat) OR Complete (follow-up in an
                // active chat where the previous response is still showing).
                wait_for_any(
                    ctx,
                    &[WebUiState::Ready, WebUiState::Complete],
                    ctx.config.max_wait_for_gen_ms,
                    "Idle→Ready|Complete",
                )?;
                state = transition(ctx, state, ExecutionState::Submitting);
            }

            ExecutionState::Submitting => {
                // When re-submitting after error recovery in a new chat,
                // prepend previous iteration context so the AI has the work.
                let effective_prompt = if in_new_chat {
                    if let Some(ctx_text) = previous_context {
                        format!(
                            "═══ PREVIOUS ITERATION OUTPUT (for context) ═══
{ctx_text}

═══ CURRENT PROMPT ═══
{prompt}"
                        )
                    } else {
                        prompt.to_string()
                    }
                } else {
                    prompt.to_string()
                };
                ctx.adapter
                    .submit_prompt(&effective_prompt)
                    .map_err(FsmError::Adapter)?;
                state = transition(ctx, state, ExecutionState::WaitingForGen);
            }

            ExecutionState::WaitingForGen => {
                eprintln!("[fsm] post-submit quiet: wait 5s, check stop square, then wait 40s");
                post_submit_stop_square_check(ctx)?;
                // Either generation starts (Generating) or skips straight to
                // Complete on a very short response. Both are valid.
                match wait_for_any(
                    ctx,
                    &[WebUiState::Generating, WebUiState::Complete],
                    ctx.config.max_wait_for_gen_ms,
                    "WaitingForGen",
                )? {
                    WebUiState::Generating => {
                        state = transition(ctx, state, ExecutionState::Generating)
                    }
                    WebUiState::Complete => {
                        state = transition(ctx, state, ExecutionState::Complete)
                    }
                    _ => unreachable!("wait_for_any only returns one of the requested states"),
                }
            }

            ExecutionState::Generating => match wait_for_generation_complete(ctx)? {
                WebUiState::Complete => state = transition(ctx, state, ExecutionState::Complete),
                WebUiState::Error => {
                    state = transition(
                        ctx,
                        state,
                        ExecutionState::Error("generation reported error".into()),
                    )
                }
                WebUiState::ChatFull => {
                    state = transition(ctx, state, ExecutionState::StartingNewChat)
                }
                _ => unreachable!(),
            },

            ExecutionState::Complete => {
                state = transition(ctx, state, ExecutionState::Copying);
            }

            ExecutionState::Copying => {
                let response = ctx.adapter.copy_response().map_err(FsmError::Adapter)?;
                // Arena errors: if the copied text looks like an error
                // message rather than a real AI response, route to Error.
                if looks_like_arena_error(&response) {
                    eprintln!("[fsm] copy_response looks like Arena error — routing to Error");
                    state = transition(
                        ctx,
                        state,
                        ExecutionState::Error("Arena returned error instead of response".into()),
                    );
                } else {
                    state = transition(ctx, state, ExecutionState::Done(response));
                }
            }

            ExecutionState::Done(response) => {
                let r = response.clone();
                persist_runtime(ctx, &state);
                return Ok(r);
            }

            ExecutionState::Error(reason) => {
                if retries >= ctx.config.max_retries {
                    let msg = format!("max retries reached after error: {reason}");
                    let _ = transition(ctx, state.clone(), ExecutionState::Failed(msg));
                    return Err(FsmError::RetriesExhausted);
                }
                retries += 1;
                state = transition(ctx, state, ExecutionState::Retrying(retries));
            }

            ExecutionState::Retrying(_n) => {
                // Arena errors require a fresh chat — "Please start a new
                // chat and try again". Start new chat, wait for Ready, then
                // re-submit.
                thread::sleep(Duration::from_millis(1_000));
                state = transition(ctx, state, ExecutionState::StartingNewChat);
            }

            ExecutionState::StartingNewChat => {
                ctx.adapter.start_new_chat().map_err(FsmError::Adapter)?;
                // Wait for Ready in the new chat before we resubmit.
                wait_for_state(ctx, WebUiState::Ready, 30_000, "StartingNewChat→Ready")?;
                in_new_chat = true;
                state = transition(ctx, state, ExecutionState::Submitting);
            }

            ExecutionState::Failed(reason) => {
                return Err(FsmError::Io(reason.clone()));
            }
        }
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────────

/// Generation-aware wait: two-phase polling.
///
/// Phase 1 (quiet period, first `min_generation_ms`): poll every
///   `generation_poll_ms`, only break on Error/ChatFull. Complete is
///   IGNORED — the AI needs time to finish before we trust the signal.
///
/// Phase 2 (active polling): poll every `generation_poll_ms`, accept
///   Complete only after a confirmation poll 5s later.
///
/// Hard timeout: `max_generation_ms`.
fn wait_for_generation_complete(ctx: &FsmContext) -> Result<WebUiState, FsmError> {
    let min_gen = Duration::from_millis(ctx.config.min_generation_ms);
    let max_gen = Duration::from_millis(ctx.config.max_generation_ms);
    let poll = Duration::from_millis(ctx.config.generation_poll_ms);
    let started = Instant::now();

    eprintln!(
        "[fsm] generation: quiet period {}s, then polling every {}s (max {}s)",
        min_gen.as_secs(),
        poll.as_secs(),
        max_gen.as_secs(),
    );

    loop {
        if started.elapsed() >= max_gen {
            return Err(FsmError::Timeout("Generating"));
        }

        thread::sleep(poll);

        if let Some(p) = check_all(&ctx.invariants).into_iter().next() {
            recover_or_fail(ctx, &p)?;
            continue;
        }

        let _ = ctx.adapter.prepare_for_detection();
        let detected = ctx.adapter.detect_state().map_err(FsmError::Adapter)?;
        let elapsed = started.elapsed().as_secs();

        match detected {
            WebUiState::Error => return Ok(WebUiState::Error),
            WebUiState::ChatFull => return Ok(WebUiState::ChatFull),

            WebUiState::Complete if started.elapsed() >= min_gen => {
                eprintln!("[fsm] generation: Complete detected at {elapsed}s — confirming…");
                thread::sleep(Duration::from_secs(5));
                let confirm = ctx.adapter.detect_state().map_err(FsmError::Adapter)?;
                if confirm == WebUiState::Complete {
                    eprintln!("[fsm] generation: confirmed Complete at {elapsed}s");
                    return Ok(WebUiState::Complete);
                }
                eprintln!("[fsm] generation: unconfirmed (now {confirm}) — continuing");
            }

            WebUiState::Complete if started.elapsed() < min_gen => {
                eprintln!(
                    "[fsm] generation: Complete at {elapsed}s but min_generation_ms not reached"
                );
            }

            other => {
                let phase = if started.elapsed() < min_gen {
                    "quiet"
                } else {
                    "active"
                };
                eprintln!("[fsm] generation: {phase} {elapsed}s — state={other}");
            }
        }
    }
}

fn post_submit_stop_square_check(ctx: &FsmContext) -> Result<(), FsmError> {
    thread::sleep(Duration::from_secs(5));

    if let Some(p) = check_all(&ctx.invariants).into_iter().next() {
        recover_or_fail(ctx, &p)?;
    }

    // Ignore state classification here. This call exists so adapter-level
    // watchdog actions, such as clicking Arena's white stop square, run once
    // shortly after prompt submission.
    let _ = ctx.adapter.detect_state().map_err(FsmError::Adapter)?;

    thread::sleep(Duration::from_secs(40));
    Ok(())
}

/// Poll the adapter until `target` is observed *stably*, or `timeout_ms`
/// elapses. Recovery runs whenever an invariant breaks or the adapter
/// returns an unrecoverable error.
fn wait_for_state(
    ctx: &FsmContext,
    target: WebUiState,
    timeout_ms: u64,
    stage: &'static str,
) -> Result<(), FsmError> {
    wait_for_any(ctx, &[target], timeout_ms, stage).map(|_| ())
}

/// Poll until any of `targets` is observed stably, or timeout.
/// Returns the first target that matched.
fn wait_for_any(
    ctx: &FsmContext,
    targets: &[WebUiState],
    timeout_ms: u64,
    stage: &'static str,
) -> Result<WebUiState, FsmError> {
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    let stable_for = Duration::from_millis(ctx.config.state_stable_ms);
    let poll_period = Duration::from_millis(ctx.config.poll_interval_ms);

    let mut last_seen: Option<WebUiState> = None;
    let mut last_change: Instant = Instant::now();
    let mut unknown_streak: u32 = 0;

    while Instant::now() < deadline {
        // Run invariants on every poll — cheap, catches drift fast.
        if let Some(p) = check_all(&ctx.invariants).into_iter().next() {
            recover_or_fail(ctx, &p)?;
            // After recovery, reset stability tracking.
            last_seen = None;
            last_change = Instant::now();
            continue;
        }

        let _ = ctx.adapter.prepare_for_detection();
        let now_state = match ctx.adapter.detect_state() {
            Ok(s) => s,
            Err(e) => return Err(FsmError::Adapter(e)),
        };

        if now_state == WebUiState::Unknown {
            unknown_streak += 1;
            if unknown_streak >= ctx.config.unknown_polls_threshold {
                recover_or_fail(ctx, &Problem::UnknownScreen)?;
                unknown_streak = 0;
            }
        } else {
            unknown_streak = 0;
        }

        if last_seen.as_ref() != Some(&now_state) {
            last_seen = Some(now_state);
            last_change = Instant::now();
        }

        if let Some(seen) = last_seen {
            if targets.contains(&seen) && last_change.elapsed() >= stable_for {
                return Ok(seen);
            }
        }

        thread::sleep(poll_period);
    }

    Err(FsmError::Timeout(stage))
}

/// Run the recovery dispatcher; map outcomes into FSM-level errors.
fn recover_or_fail(ctx: &FsmContext, p: &Problem) -> Result<(), FsmError> {
    let rctx = RecoveryContext {
        adapter: Arc::clone(&ctx.adapter),
        site: ctx.site.clone(),
    };
    match ctx.recovery.handle(p, &rctx) {
        DispatchResult::Recovered { action } => {
            log_transition(
                ctx,
                "recovery",
                action,
                Some(format!("resolved problem: {p}")),
            );
            Ok(())
        }
        DispatchResult::Failed { tried } => {
            log_transition(
                ctx,
                "recovery",
                "failed",
                Some(format!(
                    "could not resolve {p}; tried [{}]",
                    tried.join(", ")
                )),
            );
            Err(FsmError::Unrecovered(UnrecoveredProblem {
                problem: p.clone(),
                tried,
            }))
        }
    }
}

/// Map an in-flight problem into an FSM transition. Used between states
/// when an invariant breaks but recovery succeeds.
fn handle_problem(
    ctx: &FsmContext,
    p: &Problem,
    state: &mut ExecutionState,
) -> Result<(), FsmError> {
    // ChatFull is special — it's a state instruction, not a recovery hand-off.
    if matches!(p, Problem::ChatFull) {
        *state = transition(ctx, state.clone(), ExecutionState::StartingNewChat);
        return Ok(());
    }
    recover_or_fail(ctx, p)
}

/// Log a transition and persist runtime state.
fn transition(ctx: &FsmContext, from: ExecutionState, to: ExecutionState) -> ExecutionState {
    log_transition(ctx, from.name(), to.name(), None);
    persist_runtime(ctx, &to);
    to
}

fn log_transition(ctx: &FsmContext, from: &str, to: &str, note: Option<String>) {
    let row = now_transition(
        ctx.adapter.name(),
        ctx.task_index,
        ctx.iteration,
        from,
        to,
        note,
        None,
    );
    if let Err(e) = ctx.state_log.append(&row) {
        // Logging failures must never block the FSM. Surface to stderr.
        eprintln!("[state_log] failed to write transition: {e}");
    }
}

fn persist_runtime(ctx: &FsmContext, state: &ExecutionState) {
    let row = RuntimeState {
        task_index: ctx.task_index,
        iteration: ctx.iteration,
        last_state: state.name().to_string(),
        updated_at: chrono::Utc::now().to_rfc3339(),
        adapter: ctx.adapter.name().to_string(),
    };
    if let Err(e) = row.save() {
        eprintln!("[runtime] failed to persist resume state: {e}");
    }
}

/// Check if copied text looks like an Arena error message rather than
/// a real AI response. Short text containing Arena error phrases → true.
fn looks_like_arena_error(text: &str) -> bool {
    let lower = text.to_lowercase();
    let short = text.len() < 500;
    short
        && (lower.contains("something went wrong")
            || lower.contains("please start a new chat")
            || lower.contains("failed to generate")
            || (lower.contains("try again") && lower.contains("error")))
}
