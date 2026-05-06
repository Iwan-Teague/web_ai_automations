//! Top-level execution loop.
//!
//! Replaces the old linear "submit → wait → copy" flow with the FSM-driven
//! pipeline:
//!   - one `WebUiAdapter` chosen from `cfg.target_webui`
//!   - one `RecoveryDispatcher` of recovery actions
//!   - one `Vec<Box<dyn Invariant>>` of environment invariants
//!   - one `StateLog` recording every transition
//!   - one `RuntimeState` persisted after every transition for resume
//!
//! After each iteration the response is appended to:
//!   1. `outputs/runs/<run>/results/task_{n}/iter_{i}_raw.md`        (per-iteration)
//!   2. `outputs/runs/<run>/transcript.md`                    (chronological, all iters)
//!   3. `outputs/runs/<run>/state_log.jsonl`                  (state machine timeline)
//!
//! When the run completes cleanly, `session/runtime.json` is removed so the
//! next launch does not offer a stale resume.

use std::collections::VecDeque;
use std::error::Error;
use std::io::{self, BufRead, Write};
use std::sync::Arc;

use crate::session::WebUiTarget;

/// DeepSeek's context window saturates after many long prompts. Every Nth
/// main prompt the runner calls `adapter.rotate_for_long_session` and
/// prepends the new prompt with the last few AI outputs as context.
const DEEPSEEK_ROTATE_EVERY: usize = 10;
const DEEPSEEK_CONTEXT_OUTPUTS: usize = 3;

use crate::output::summary::{IterationOutcome, write_summary};
use crate::output::transcript::{TranscriptEntry, append as append_transcript};
use crate::output::writer::{ensure_run_dirs, save_raw, save_self_review, task_dir};
use crate::prompt::builder::{
    build_research_followup_prompt_with_previous_response, build_self_review_prompt,
    build_standard_prompt, wrap_prompt_with_previous_response,
};
use crate::runner::constraint_check::check_constraint_violations;
use crate::runner::debate_runner::run_debate;
use crate::runner::site_workflow::prepare_site;
use crate::session::{PromptMode, ReviewMode, SessionConfig};
use crate::webui::fsm::{FsmConfig, FsmContext, FsmError, run_iteration_with_context};
use crate::webui::recovery::{RecoveryDispatcher, default_actions};
use crate::webui::runtime::RuntimeState;
use crate::webui::state_log::StateLog;
use crate::webui::watchdog::default_invariants;

/// Top-level dispatch loop — one entry per task, one inner pass per iteration.
pub fn run_session(cfg: &SessionConfig) -> Result<(), Box<dyn Error>> {
    let mut outcomes: Vec<IterationOutcome> = Vec::new();
    let run_root = ensure_run_dirs()?;
    println!("[output] run folder → {}", run_root.display());

    // ── Resume prompt FIRST (before any browser-touching setup) ─────────────
    //
    // Why first: the prompt blocks on stdin while Terminal is the
    // foreground app. If we run setup first, setup focuses Chrome and
    // then the Y/N prompt drags focus back to Terminal, leaving the
    // FSM's first invariant pass to fight the focus right after start.
    // Asking up-front lets setup fire AFTER we already have stdin
    // input, so its final action (Direct mode click) leaves Chrome
    // foreground for the FSM.
    let expected_adapter_name = cfg.target_webui.adapter_name();
    let resume = RuntimeState::load();
    let (start_task, start_iter) = match resume {
        Some(r) if r.adapter == expected_adapter_name => {
            println!(
                "Found prior runtime state at task {} iter {} (last state: {}).",
                r.task_index + 1,
                r.iteration + 1,
                r.last_state,
            );
            if prompt_yes_no("Resume from there?", true)? {
                (r.task_index, r.iteration)
            } else {
                (0, 0)
            }
        }
        Some(r) => {
            println!(
                "Found runtime state for adapter '{}' but session targets '{}' — ignoring.",
                r.adapter, expected_adapter_name,
            );
            (0, 0)
        }
        None => (0, 0),
    };

    // ── One-time site setup shared across all iterations ────────────────────
    let prepared = prepare_site(cfg)?;
    let adapter = prepared.adapter;
    let site = prepared.hints;

    // ── Optional project map upload (one or more files, e.g. per-crate split)
    //
    // Wrapped in a restart loop: when the adapter signals "server busy"
    // (DeepSeek backend overloaded → page reloaded), the entire batch
    // starts over, since reload wipes every previously-uploaded chip.
    // Files that DeepSeek persistently rejects (3 retries × 2 batch
    // attempts) get added to `deferred_uploads` and retried at the start
    // of every follow-up iteration via `retry_deferred_uploads`.
    let mut deferred_uploads: Vec<String> = Vec::new();
    upload_project_maps_with_restart(cfg, &adapter, &mut deferred_uploads)?;
    if !deferred_uploads.is_empty() {
        println!(
            "[upload] initial batch deferred {} file(s); will retry before each follow-up prompt",
            deferred_uploads.len()
        );
    }

    let invariants = default_invariants(
        site.browser_title_substr,
        site.site_url,
        site.site_title_substr,
    );
    let recovery = RecoveryDispatcher::new(default_actions());
    let state_log = StateLog::open_default()?;
    let fsm_cfg = FsmConfig::default();

    // ── Long-session bookkeeping (DeepSeek chat rotation) ───────────────────
    //
    // `prompt_count` tracks main prompts submitted across the whole run
    // (self-review prompts excluded). `last_responses` is a ring of the
    // most recent AI outputs that gets prepended to the prompt right
    // after a rotation so the new chat starts with continuity.
    let mut prompt_count: usize = 0;
    let mut last_responses: VecDeque<String> = VecDeque::with_capacity(DEEPSEEK_CONTEXT_OUTPUTS);

    // ── Iteration loop ──────────────────────────────────────────────────────
    'tasks: for task_index in 0..cfg.tasks.len() {
        if task_index < start_task {
            continue;
        }
        for iteration in 0..cfg.iterations_per_task {
            if task_index == start_task && iteration < start_iter {
                continue;
            }

            println!(
                "\n── Task {}/{}  ·  Iter {}/{} ─────────────────────────────",
                task_index + 1,
                cfg.tasks.len(),
                iteration + 1,
                cfg.iterations_per_task,
            );

            // Load previous iteration's response (if any) so error
            // recovery in a new chat can include it as context. Research
            // follow-ups also paste this directly into the prompt.
            let prev_context = if iteration > 0 {
                let prev_path = task_dir(task_index).join(format!("iter_{}_raw.md", iteration)); // iteration is 0-based, file is 1-based
                match std::fs::read_to_string(&prev_path) {
                    Ok(s) => Some(s),
                    Err(_) => {
                        eprintln!(
                            "  ✘ missing previous response {}; refusing to submit follow-up",
                            prev_path.display()
                        );
                        outcomes.push(IterationOutcome {
                            task_index,
                            iteration,
                            verdict: Some("FAILED".to_string()),
                            score: None,
                            violations: vec![format!(
                                "missing_previous_response: {}",
                                prev_path.display()
                            )],
                        });
                        break 'tasks;
                    }
                }
            } else {
                None
            };

            // ── DeepSeek long-session rotation ─────────────────────────────
            //
            // Every Nth main prompt: start a fresh chat, re-attach the
            // project map, and prepend the prompt with the last few AI
            // outputs as context. Skipped on prompt 0 (initial setup
            // already produced a fresh chat).
            let rotating = matches!(cfg.target_webui, WebUiTarget::DeepSeek)
                && prompt_count > 0
                && prompt_count % DEEPSEEK_ROTATE_EVERY == 0;
            if rotating {
                println!(
                    "[rotate] {} prompts sent — rotating to fresh chat",
                    prompt_count
                );
                match adapter.rotate_for_long_session(cfg) {
                    Ok(true) => println!("[rotate] fresh chat ready"),
                    Ok(false) => eprintln!("[rotate] adapter does not support rotation"),
                    Err(e) => eprintln!("[rotate] failed: {e} — proceeding anyway"),
                }
                // Fresh chat → every file gets a fresh shot at uploading,
                // so reset the deferred list before re-running the batch.
                deferred_uploads.clear();
                if let Err(e) =
                    upload_project_maps_with_restart(cfg, &adapter, &mut deferred_uploads)
                {
                    eprintln!("[rotate] re-upload failed: {e}");
                }
            } else if prompt_count > 0 {
                // Non-rotation follow-up iteration. Try uploading any
                // files that earlier got deferred — succeeds attaches
                // them to the active conversation; failures stay
                // deferred for the next iteration.
                retry_deferred_uploads(&adapter, &mut deferred_uploads);
            }

            // 1. Build prompt after upload/retry, so the prompt's map
            // list reflects which docs are actually attached right now.
            let prompt_cfg = cfg_with_uploaded_project_maps(cfg, &deferred_uploads);
            let use_simplified = cfg.success_criteria.is_empty();
            let prompt = if !use_simplified {
                let base = build_standard_prompt(
                    &prompt_cfg,
                    task_index,
                    iteration,
                    prompt_cfg.iterations_per_task,
                );
                if prompt_cfg.prompt_mode == PromptMode::Research && iteration > 0 {
                    wrap_prompt_with_previous_response(
                        prev_context.as_deref().unwrap_or_default(),
                        &base,
                    )
                } else {
                    base
                }
            } else if iteration == 0 {
                crate::prompt::builder::build_initial_prompt(&prompt_cfg)
            } else if prompt_cfg.prompt_mode == PromptMode::Research {
                build_research_followup_prompt_with_previous_response(
                    &prompt_cfg,
                    iteration,
                    prev_context.as_deref().unwrap_or_default(),
                )
            } else {
                crate::prompt::builder::build_followup_prompt(&prompt_cfg, iteration)
            };
            let mut prompt = prepend_project_map_upload_note(prompt, cfg, &deferred_uploads);
            if rotating {
                prompt = prepend_recent_context(&prompt, &last_responses);
            }
            let retry_context = if cfg.prompt_mode == PromptMode::Research {
                None
            } else {
                prev_context.as_deref()
            };

            let ctx = FsmContext {
                adapter: Arc::clone(&adapter),
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
            let _ = (&invariants, &recovery, &state_log); // kept for clarity

            let response = match run_iteration_with_context(&ctx, &prompt, retry_context) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("  ✘ FSM halted iteration: {e}");
                    let abort_run = should_abort_after_fsm_error(&e);
                    if abort_run {
                        eprintln!(
                            "    Halting remaining iterations; next prompt would risk overwriting uncopied output."
                        );
                    } else {
                        eprintln!("    Continuing to next iteration; runtime.json preserved.");
                    }
                    outcomes.push(IterationOutcome {
                        task_index,
                        iteration,
                        verdict: Some("FAILED".to_string()),
                        score: None,
                        violations: vec![format!("fsm_error: {e}")],
                    });
                    if abort_run {
                        break 'tasks;
                    }
                    continue;
                }
            };

            let raw_path = save_raw(task_index, iteration, &response)?;
            println!("  raw saved → {}", raw_path.display());

            // Bookkeeping for the long-session rotation: count this main
            // prompt and push the response into the rolling context ring.
            prompt_count += 1;
            if last_responses.len() == DEEPSEEK_CONTEXT_OUTPUTS {
                last_responses.pop_front();
            }
            last_responses.push_back(response.clone());

            // Download code artifacts (zip) if the adapter has a code panel.
            let artifact_dir =
                task_dir(task_index).join(format!("iter_{}_artifacts", iteration + 1));
            match adapter.download_artifacts_to(&artifact_dir) {
                Ok(true) => println!("  artifacts downloaded"),
                Ok(false) => {}
                Err(e) => eprintln!("  artifact download failed: {e}"),
            }

            // 2. Constraint scan.
            let violations = check_constraint_violations(&response, &cfg.hard_constraints);
            if !violations.is_empty() {
                println!("  ⚠  flagged constraints:");
                for v in &violations {
                    println!("    - {v}");
                }
            }

            // 3. Review pass.
            let mut verdict: Option<String> = None;
            let mut score: Option<f32> = None;
            match cfg.review_mode {
                ReviewMode::Standard => {
                    let rprompt = build_self_review_prompt(cfg, &response);
                    // Self-review uses the same adapter and FSM.
                    let rctx = FsmContext {
                        adapter: Arc::clone(&adapter),
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
                    let rresponse = run_iteration_with_context(&rctx, &rprompt, None)
                        .unwrap_or_else(|e| format!("[self-review failed: {e}]"));
                    let path = save_self_review(task_index, iteration, &rresponse)?;
                    println!("  self-review → {}", path.display());
                }
                ReviewMode::Debate => {
                    let outcome = run_debate(cfg, task_index, iteration, &response)?;
                    verdict = outcome.verdict;
                    score = outcome.score;
                    if let Some(v) = &verdict {
                        println!("  judge verdict: {v}");
                    }
                }
                ReviewMode::None => {}
            }

            // 4. Append transcript.
            let model = adapter.detect_model().ok().flatten();
            let note = if violations.is_empty() {
                None
            } else {
                Some(format!("constraint flags: {}", violations.join("; ")))
            };
            let entry = TranscriptEntry {
                task_index,
                iteration,
                adapter: adapter.name(),
                model: model.as_deref(),
                prompt: &prompt,
                response: &response,
                note: note.as_deref(),
            };
            if let Err(e) = append_transcript(&entry) {
                eprintln!("  transcript append failed: {e}");
            }

            outcomes.push(IterationOutcome {
                task_index,
                iteration,
                verdict,
                score,
                violations,
            });

            // 5. Pause unless auto-continue.
            if !cfg.auto_continue && !is_last(task_index, iteration, cfg) {
                pause_for_user()?;
            }
        }
    }

    // ── Wrap-up ─────────────────────────────────────────────────────────────
    let path = write_summary(cfg, &outcomes)?;
    println!("\nSummary written → {}", path.display());
    if let Err(e) = RuntimeState::clear() {
        eprintln!("(could not clear runtime.json: {e})");
    }
    Ok(())
}

fn cfg_with_uploaded_project_maps(
    cfg: &SessionConfig,
    deferred_uploads: &[String],
) -> SessionConfig {
    let all_paths = cfg.all_project_map_paths();
    if all_paths.is_empty() {
        return cfg.clone();
    }

    let uploaded: Vec<String> = all_paths
        .into_iter()
        .filter(|p| !deferred_uploads.iter().any(|d| d.as_str() == *p))
        .map(str::to_string)
        .collect();

    let mut prompt_cfg = cfg.clone();
    if let Some((primary, extras)) = uploaded.split_first() {
        prompt_cfg.project_map_path = Some(primary.clone());
        prompt_cfg.project_map_extra_paths = extras.to_vec();
    } else {
        prompt_cfg.project_map_path = None;
        prompt_cfg.project_map_extra_paths.clear();
    }
    prompt_cfg
}

fn prepend_project_map_upload_note(
    prompt: String,
    cfg: &SessionConfig,
    deferred_uploads: &[String],
) -> String {
    let all_paths = cfg.all_project_map_paths();
    if all_paths.is_empty() || deferred_uploads.is_empty() {
        return prompt;
    }

    let missing: Vec<String> = all_paths
        .iter()
        .filter(|p| deferred_uploads.iter().any(|d| d.as_str() == **p))
        .map(|p| short_filename(p))
        .collect();
    if missing.is_empty() {
        return prompt;
    }

    let total = all_paths.len();
    let uploaded = total.saturating_sub(missing.len());
    format!(
        "Heads up: only {uploaded}/{total} project-map docs are attached right now. Not all docs uploaded yet; automation will retry missing docs before later prompts.\nMissing now: {}.\nUse attached docs only. If evidence lives in a missing doc, say it is not attached yet.\n\n{prompt}",
        missing.join(", ")
    )
}

fn short_filename(path: &str) -> String {
    std::path::Path::new(path)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(path)
        .to_string()
}

/// Upload every project-map file in the configured order.
///
/// Two failure paths the adapter can surface:
/// - `SERVER_BUSY_RESTART_SENTINEL` — DeepSeek backend overloaded, page
///   was reloaded. We rotate the chat (re-apply model + toggles) and
///   restart the batch from file 1, capped at `MAX_BATCH_RESTARTS`.
/// - `FILE_REJECTED_DEFER_SENTINEL` — this one file got rejected three
///   times in a row but the page wasn't reloaded; the chip's already
///   been deleted locally. The file goes into a "tried-once" list and
///   we continue to the next file. After the pass, every "tried-once"
///   file gets ONE more attempt against the active chat. Anything still
///   failing gets carried into `deferred_uploads` for retry on every
///   subsequent iteration.
fn upload_project_maps_with_restart(
    cfg: &SessionConfig,
    adapter: &Arc<dyn crate::webui::adapter::WebUiAdapter>,
    deferred_uploads: &mut Vec<String>,
) -> Result<(), Box<dyn Error>> {
    use crate::webui::deepseek::adapter::{
        FILE_REJECTED_DEFER_SENTINEL, NEW_CHAT_TRIGGERED_SENTINEL, SERVER_BUSY_RESTART_SENTINEL,
    };
    const MAX_BATCH_RESTARTS: usize = 4;
    // Circuit breaker: once this many files have failed in a single
    // batch pass, stop trying the rest — DeepSeek's context is clearly
    // full. Defer everything left and submit the prompt with whatever
    // landed; the runner retries deferred files at the start of every
    // follow-up iteration.
    const MAX_FAILURES_BEFORE_ABORT: usize = 3;

    let map_paths: Vec<String> = cfg
        .all_project_map_paths()
        .iter()
        .map(|s| s.to_string())
        .collect();
    if map_paths.is_empty() {
        return Ok(());
    }
    let total_maps = map_paths.len();
    // Files that triggered DeepSeek's "New chat" button and ended up
    // attached to the fresh chat. Skipped on subsequent batch passes
    // (no point re-uploading something that's already there). Lives at
    // the function scope so it persists across batch restarts.
    let mut new_chat_carried_over: Vec<String> = Vec::new();

    for restart in 0..=MAX_BATCH_RESTARTS {
        if restart > 0 {
            println!(
                "[upload] batch restart {}/{} after server-busy reload — re-applying model + toggles",
                restart, MAX_BATCH_RESTARTS
            );
            if let Err(e) = adapter.rotate_for_long_session(cfg) {
                eprintln!("[upload] post-busy rotate failed: {e}");
            }
        }

        // Files that returned the file-rejected sentinel during this
        // batch pass. Retried once at the end of the pass — UNLESS the
        // failure count hit the circuit breaker.
        let mut tried_once: Vec<String> = Vec::new();
        let mut server_busy_triggered = false;
        let mut breaker_tripped = false;

        for (i, map_path) in map_paths.iter().enumerate() {
            // Files we already gave up on (carried from a previous
            // iteration via `deferred_uploads`) — skip cleanly.
            if deferred_uploads.iter().any(|d| d == map_path) {
                println!(
                    "[upload] {}/{} SKIP (deferred from earlier iteration, retried before each prompt): {map_path}",
                    i + 1,
                    total_maps
                );
                continue;
            }
            // Files that already landed in the active chat via the
            // "New chat" carry-over — re-uploading would just create
            // a duplicate chip.
            if new_chat_carried_over.iter().any(|d| d == map_path) {
                println!(
                    "[upload] {}/{} SKIP (already attached via 'New chat' carry-over): {map_path}",
                    i + 1,
                    total_maps
                );
                continue;
            }
            let path = std::path::Path::new(map_path);
            match adapter.upload_file(path) {
                Ok(true) => println!(
                    "[upload] project map {}/{} attached: {map_path}",
                    i + 1,
                    total_maps
                ),
                Ok(false) => {
                    println!(
                        "[upload] adapter does not support file upload — skipping project map"
                    );
                    return Ok(());
                }
                Err(e) => {
                    let msg = format!("{e}");
                    if msg.contains(NEW_CHAT_TRIGGERED_SENTINEL) {
                        // The adapter clicked DeepSeek's "New chat"
                        // button. We're now in a fresh chat with just
                        // `map_path` attached; every other file is
                        // gone. Clear deferred (irrelevant in the new
                        // chat — the new chat starts empty) and
                        // restart the batch. The in-flight file is
                        // already attached so the next pass skips it
                        // via `new_chat_carried_over`.
                        println!(
                            "[upload] 'New chat' triggered after file {}/{}; restarting batch in fresh chat (file just-attached and carried over: {map_path})",
                            i + 1,
                            total_maps
                        );
                        deferred_uploads.clear();
                        if !new_chat_carried_over.iter().any(|p| p == map_path) {
                            new_chat_carried_over.push(map_path.clone());
                        }
                        server_busy_triggered = true;
                        break;
                    }
                    if msg.contains(SERVER_BUSY_RESTART_SENTINEL) {
                        println!(
                            "[upload] server-busy detected after file {}/{} — restarting batch",
                            i + 1,
                            total_maps
                        );
                        server_busy_triggered = true;
                        break;
                    }
                    if msg.contains(FILE_REJECTED_DEFER_SENTINEL) {
                        println!(
                            "[upload] {}/{} REJECTED — chip deleted, deferring: {map_path}",
                            i + 1,
                            total_maps,
                        );
                        tried_once.push(map_path.clone());
                        // Circuit breaker: if N distinct files have
                        // failed, give up on this batch. DeepSeek is
                        // clearly out of room. Defer everything still
                        // pending and submit with what we have.
                        if tried_once.len() >= MAX_FAILURES_BEFORE_ABORT {
                            println!(
                                "[upload] circuit breaker: {} files rejected — aborting batch, deferring all remaining files",
                                tried_once.len()
                            );
                            breaker_tripped = true;
                            // Defer the failures we collected so far.
                            for f in tried_once.drain(..) {
                                deferred_uploads.push(f);
                            }
                            // Plus every file we haven't even tried yet.
                            for not_yet_tried in &map_paths[i + 1..] {
                                if !deferred_uploads.contains(not_yet_tried) {
                                    println!("[upload] deferring untried file: {not_yet_tried}");
                                    deferred_uploads.push(not_yet_tried.clone());
                                }
                            }
                            break;
                        }
                    } else {
                        eprintln!("[upload] project map upload failed: {e}");
                        tried_once.push(map_path.clone());
                        if tried_once.len() >= MAX_FAILURES_BEFORE_ABORT {
                            println!(
                                "[upload] circuit breaker: {} files failed — aborting batch, deferring all remaining files",
                                tried_once.len()
                            );
                            breaker_tripped = true;
                            for f in tried_once.drain(..) {
                                if !deferred_uploads.contains(&f) {
                                    deferred_uploads.push(f);
                                }
                            }
                            for not_yet_tried in &map_paths[i + 1..] {
                                if !deferred_uploads.contains(not_yet_tried) {
                                    println!("[upload] deferring untried file: {not_yet_tried}");
                                    deferred_uploads.push(not_yet_tried.clone());
                                }
                            }
                            break;
                        }
                    }
                }
            }
            if i + 1 < total_maps {
                std::thread::sleep(std::time::Duration::from_millis(800));
            }
        }

        if server_busy_triggered {
            continue; // page reloaded → restart the batch from scratch
        }

        if breaker_tripped {
            // Don't run the end-of-batch retry — the circuit breaker
            // says give up. Submit the prompt with what we have; the
            // deferred list will be retried at the start of every
            // follow-up iteration.
            return Ok(());
        }

        // Pass-1 done normally. Retry the tried-once files ONCE against
        // the active chat. Anything still failing gets deferred.
        if !tried_once.is_empty() {
            println!(
                "[upload] batch pass-1 done; retrying {} rejected file(s) once before submitting prompt",
                tried_once.len()
            );
            for path_str in tried_once {
                let path = std::path::Path::new(&path_str);
                match adapter.try_upload_preserving_chat(path) {
                    Ok(true) => println!("[upload] retry attached: {path_str}"),
                    _ => {
                        println!(
                            "[upload] retry still failing — deferring to follow-up prompts: {path_str}"
                        );
                        deferred_uploads.push(path_str);
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(800));
            }
        }

        return Ok(());
    }
    Err(format!(
        "upload batch exhausted {} server-busy restarts — DeepSeek backend is unstable",
        MAX_BATCH_RESTARTS
    )
    .into())
}

/// Retry every file in `deferred` against the active chat. Uses the
/// adapter's `try_upload_preserving_chat` so a still-failing file just
/// has its chip deleted locally — no page reload that would wipe the
/// conversation. Files that succeed are removed from the list; files
/// that fail again stay deferred for the next iteration's retry.
fn retry_deferred_uploads(
    adapter: &Arc<dyn crate::webui::adapter::WebUiAdapter>,
    deferred: &mut Vec<String>,
) {
    if deferred.is_empty() {
        return;
    }
    let pending: Vec<String> = std::mem::take(deferred);
    println!(
        "[upload] retrying {} deferred file(s) before this prompt: {:?}",
        pending.len(),
        pending
    );
    let mut still_failing: Vec<String> = Vec::new();
    for path_str in pending {
        let path = std::path::Path::new(&path_str);
        match adapter.try_upload_preserving_chat(path) {
            Ok(true) => println!("[upload] deferred file finally attached: {path_str}"),
            Ok(false) => {
                eprintln!(
                    "[upload] deferred retry not supported by adapter — keeping deferred: {path_str}"
                );
                still_failing.push(path_str);
            }
            Err(e) => {
                eprintln!(
                    "[upload] deferred retry still failing — keeping deferred: {path_str}: {e}"
                );
                still_failing.push(path_str);
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(800));
    }
    *deferred = still_failing;
    if !deferred.is_empty() {
        println!(
            "[upload] {} file(s) still deferred after retry; will try again next iteration",
            deferred.len()
        );
    }
}

/// Prepend the recent AI outputs to a prompt so a freshly rotated chat
/// starts with continuity. New prompt sits at the top — the model reads
/// the instruction first, then the historical context below.
fn prepend_recent_context(new_prompt: &str, recent: &VecDeque<String>) -> String {
    if recent.is_empty() {
        return new_prompt.to_string();
    }
    let mut out = String::with_capacity(new_prompt.len() + 4096);
    out.push_str(new_prompt);
    out.push_str(
        "\n\n---\nContext from the previous chat session (most recent AI outputs, oldest first):\n---\n",
    );
    for (i, r) in recent.iter().enumerate() {
        out.push_str(&format!("\n=== Previous Output {} ===\n{}\n", i + 1, r));
    }
    out
}

fn is_last(task_index: usize, iteration: usize, cfg: &SessionConfig) -> bool {
    task_index + 1 == cfg.tasks.len() && iteration + 1 == cfg.iterations_per_task
}

fn should_abort_after_fsm_error(_e: &FsmError) -> bool {
    // Any FSM failure may leave Arena mid-generation, on a completed but
    // uncopied response, or with stale text in the input box. Continuing to
    // the next configured prompt can overwrite the result we were supposed
    // to save. Stop instead; the preserved runtime.json + browser state are
    // useful for manual inspection/resume.
    true
}

fn pause_for_user() -> Result<(), Box<dyn Error>> {
    print!("Press Enter to continue (or Ctrl-C to stop)... ");
    io::stdout().flush()?;
    let mut buf = String::new();
    io::stdin().lock().read_line(&mut buf)?;
    Ok(())
}

fn prompt_yes_no(q: &str, default: bool) -> Result<bool, Box<dyn Error>> {
    let hint = if default { "[Y/n]" } else { "[y/N]" };
    print!("{q} {hint}: ");
    io::stdout().flush()?;
    let mut buf = String::new();
    io::stdin().lock().read_line(&mut buf)?;
    let ans = buf.trim().to_lowercase();
    Ok(match ans.as_str() {
        "" => default,
        "y" | "yes" => true,
        "n" | "no" => false,
        _ => default,
    })
}
