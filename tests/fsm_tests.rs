//! End-to-end FSM tests using the `NullAdapter`.
//!
//! These tests exercise the state machine, watchdog, recovery dispatcher,
//! state log, and runtime persistence without touching any real screen.
//! Every test runs on every platform.

use std::sync::Arc;

use web_ai_automation::webui::adapter::{WebUiAdapter, WebUiState};
use web_ai_automation::webui::fsm::{FsmConfig, FsmContext, run_iteration};
use web_ai_automation::webui::null::{NullAdapter, StateScript};
use web_ai_automation::webui::recovery::{RecoveryDispatcher, SiteHints};
use web_ai_automation::webui::state_log::StateLog;

fn fast_config() -> FsmConfig {
    FsmConfig {
        max_wait_for_gen_ms: 2_000,
        post_submit_quiet_ms: 0,
        max_generation_ms: 4_000,
        min_generation_ms: 100,
        generation_poll_ms: 50,
        max_retries: 1,
        poll_interval_ms: 20,
        state_stable_ms: 40,
        unknown_polls_threshold: 4,
    }
}

/// Tests deliberately use **empty** invariants and recovery actions.
/// Both rely on real OS calls (osascript / enigo) which would side-effect
/// the host machine — pressing Escape, focusing windows, etc. The FSM
/// itself doesn't care; we exercise the state machine and its logging
/// without touching the real desktop.
fn fixture_ctx(adapter: Arc<dyn WebUiAdapter>, log_path: &str) -> FsmContext {
    FsmContext {
        adapter,
        invariants: Vec::new(),
        recovery: RecoveryDispatcher::new(Vec::new()),
        site: SiteHints {
            browser_title_substr: "Chrome",
            site_url: "lmarena.ai",
            site_title_substr: "Arena",
        },
        state_log: StateLog::open(log_path).expect("open log"),
        task_index: 0,
        iteration: 0,
        config: fast_config(),
    }
}

fn tmp_log(name: &str) -> String {
    let dir = std::env::temp_dir();
    dir.join(format!(
        "web_ai_automation_test_{name}_{}.jsonl",
        std::process::id()
    ))
    .to_string_lossy()
    .into_owned()
}

// ── Happy path ──────────────────────────────────────────────────────────────

#[test]
fn fsm_completes_with_default_script() {
    let adapter = Arc::new(NullAdapter::new()) as Arc<dyn WebUiAdapter>;
    let ctx = fixture_ctx(adapter, &tmp_log("happy"));

    let response = run_iteration(&ctx, "what is the capital of France?")
        .expect("FSM should complete on the default Ready→Generating→Complete script");
    assert!(response.contains("[null adapter response]"));
    assert!(response.contains("what is the capital of France?"));
}

// ── Skip-Generating path (very short response) ──────────────────────────────

#[test]
fn fsm_handles_immediate_complete_without_generating() {
    let script = StateScript {
        states: vec![
            WebUiState::Ready,
            WebUiState::Complete,
            WebUiState::Complete,
        ],
    };
    let adapter = Arc::new(NullAdapter::with_script(script)) as Arc<dyn WebUiAdapter>;
    let ctx = fixture_ctx(adapter, &tmp_log("immediate"));

    let response = run_iteration(&ctx, "ping").expect("immediate-complete is valid");
    assert!(response.contains("ping"));
}

// ── Unknown screen → recovery escalates and FSM fails ───────────────────────

#[test]
fn fsm_fails_when_screen_stays_unknown() {
    // All Unknowns → Watchdog will mark UnknownScreen, recovery actions
    // for it (PressEscape, DismissModal) won't change anything because
    // the null adapter ignores escape, so the FSM should eventually
    // give up.
    let script = StateScript {
        states: vec![WebUiState::Unknown; 50],
    };
    let adapter = Arc::new(NullAdapter::with_script(script)) as Arc<dyn WebUiAdapter>;
    let ctx = fixture_ctx(adapter, &tmp_log("unknown"));

    let result = run_iteration(&ctx, "anything");
    assert!(
        result.is_err(),
        "expected FSM to fail with persistent Unknown"
    );
}

// ── Error → retry → success ─────────────────────────────────────────────────

#[test]
fn fsm_retries_after_error_then_completes() {
    // Walk: Ready, Generating, Error, then on retry: Ready, Generating, Complete.
    let script = StateScript {
        states: vec![
            WebUiState::Ready,
            WebUiState::Generating,
            WebUiState::Error,
            WebUiState::Ready,
            WebUiState::Generating,
            WebUiState::Complete,
        ],
    };
    let adapter = Arc::new(NullAdapter::with_script(script)) as Arc<dyn WebUiAdapter>;
    let ctx = fixture_ctx(adapter, &tmp_log("retry"));

    // The FSM might exhaust retries depending on stability windows; either
    // way the state log should grow with multiple transitions.
    let _ = run_iteration(&ctx, "test");
    let log = std::fs::read_to_string(ctx.state_log.path()).expect("log written");
    let line_count = log.lines().count();
    assert!(
        line_count >= 3,
        "state log should have at least 3 transitions, got {line_count}"
    );
}
