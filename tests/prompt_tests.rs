use web_ai_automation::prompt::builder::{
    build_followup_prompt, build_initial_prompt,
    build_research_followup_prompt_with_previous_response, build_self_review_prompt,
    build_standard_prompt,
};
use web_ai_automation::prompt::debate::{build_bear_prompt, build_bull_prompt, build_judge_prompt};
use web_ai_automation::session::{
    CodingSpecialty, ContentType, DebateStrength, DeepSeekModelMode, PromptMode, ReviewMode,
    SessionConfig, WebUiTarget,
};
use web_ai_automation::webui::arena::regions::{ArenaSurface, Regions};

fn fixture() -> SessionConfig {
    SessionConfig {
        repo_url: "https://github.com/example/repo".into(),
        branch: Some("main".into()),
        content_type: ContentType::RustCode,

        end_goal: "Ship a production-ready image comparison module.".into(),
        success_criteria: "Tests pass, no unsafe blocks, runs on Windows.".into(),

        hard_constraints: vec![
            "Do not change the public API.".into(),
            "No nightly features.".into(),
            "Do not introduce tokio.".into(),
        ],
        background_context: "Runs on a Windows automation bot.".into(),
        tech_constraints: "Rust 2024 edition, stable toolchain.".into(),

        tasks: vec![
            "Review compare.rs for correctness".into(),
            "Identify race conditions".into(),
        ],
        iterations_per_task: 2,
        auto_continue: false,

        review_mode: ReviewMode::Debate,
        debate_rounds: 1,
        bull_strength: DebateStrength::Strong,
        bear_strength: DebateStrength::DevilsAdvocate,
        auto_apply_judge: false,
        debate_mode_enabled: false,

        prompt_mode: PromptMode::Research,
        coding_specialty: CodingSpecialty::General,
        research_specialty: CodingSpecialty::General,
        target_webui: WebUiTarget::Null,
        arena_model: None,
        deepseek_model: DeepSeekModelMode::Instant,
        deepseek_deep_thinking: false,
        deepseek_smart_search: false,
        project_map_path: None,
        project_map_extra_paths: Vec::new(),
    }
}

// ── Standard prompt ──────────────────────────────────────────────────────────

#[test]
fn standard_prompt_injects_end_goal_first() {
    let cfg = fixture();
    let p = build_standard_prompt(&cfg, 0, 0, cfg.iterations_per_task);

    let goal_pos = p.find(&cfg.end_goal).expect("end_goal must appear");
    let constraints_pos = p
        .find("CONSTRAINTS")
        .expect("constraints block must appear");
    let task_pos = p.find("TASK  [").expect("task header must appear");

    assert!(goal_pos < constraints_pos);
    assert!(constraints_pos < task_pos);
}

#[test]
fn standard_prompt_lists_every_hard_constraint() {
    let cfg = fixture();
    let p = build_standard_prompt(&cfg, 0, 0, cfg.iterations_per_task);
    for c in &cfg.hard_constraints {
        assert!(p.contains(c), "constraint missing: {c}");
    }
}

#[test]
fn standard_prompt_renders_one_based_indices() {
    let cfg = fixture();
    let p = build_standard_prompt(&cfg, 1, 1, cfg.iterations_per_task);
    assert!(p.contains("TASK  [2 of 2]"));
    assert!(p.contains("Iteration 2 of 2"));
}

#[test]
fn standard_prompt_uses_default_branch_when_missing() {
    let mut cfg = fixture();
    cfg.branch = None;
    let p = build_standard_prompt(&cfg, 0, 0, cfg.iterations_per_task);
    assert!(p.contains("(default)"));
}

#[test]
fn standard_prompt_includes_current_task_text() {
    let cfg = fixture();
    let p = build_standard_prompt(&cfg, 1, 0, cfg.iterations_per_task);
    assert!(p.contains("Identify race conditions"));
}

// ── Self-review ──────────────────────────────────────────────────────────────

#[test]
fn self_review_prompt_includes_task_output() {
    let cfg = fixture();
    let p = build_self_review_prompt(&cfg, "<<<previous AI output>>>");
    assert!(p.contains("<<<previous AI output>>>"));
    assert!(p.contains(&cfg.end_goal));
}

// ── Debate prompts ───────────────────────────────────────────────────────────

#[test]
fn bull_prompt_carries_role_and_strength() {
    let cfg = fixture();
    let p = build_bull_prompt(&cfg, "proposed change");
    assert!(p.contains("ROLE: BULL"));
    assert!(p.contains("Stance strength: Strong"));
    assert!(p.contains(&cfg.end_goal));
}

#[test]
fn bear_prompt_uses_devils_advocate_strength() {
    let cfg = fixture();
    let p = build_bear_prompt(&cfg, "proposed change");
    assert!(p.contains("ROLE: BEAR"));
    assert!(p.contains("Stance strength: Devil's Advocate"));
}

#[test]
fn judge_prompt_carries_all_three_inputs() {
    let cfg = fixture();
    let p = build_judge_prompt(&cfg, 0, "ORIG-OUT", "BULL-OUT", "BEAR-OUT");
    assert!(p.contains("ROLE: JUDGE"));
    assert!(p.contains("Round 1 of 1"));
    assert!(p.contains("ORIG-OUT"));
    assert!(p.contains("BULL-OUT"));
    assert!(p.contains("BEAR-OUT"));
    assert!(p.contains("ACCEPT / REJECT / REVISE"));
}

#[test]
fn every_prompt_starts_with_end_goal_block() {
    let cfg = fixture();
    let prompts = [
        build_standard_prompt(&cfg, 0, 0, cfg.iterations_per_task),
        build_self_review_prompt(&cfg, "x"),
        build_bull_prompt(&cfg, "x"),
        build_bear_prompt(&cfg, "x"),
        build_judge_prompt(&cfg, 0, "a", "b", "c"),
    ];
    for p in &prompts {
        let goal_pos = p.find("END GOAL").expect("END GOAL header missing");
        let role_pos = p
            .find("ROLE")
            .or_else(|| p.find("TASK  ["))
            .unwrap_or(usize::MAX);
        // END GOAL must precede the role/task block in every template.
        assert!(goal_pos < role_pos, "END GOAL not first:\n{p}");
    }
}

#[test]
fn coding_focus_tailors_initial_and_followup_prompts() {
    let mut cfg = fixture();
    cfg.prompt_mode = PromptMode::Coding;
    cfg.coding_specialty = CodingSpecialty::Networking;

    let initial = build_initial_prompt(&cfg);
    assert!(initial.contains("CODING FOCUS: Networking"));
    assert!(initial.contains("protocols"));
    assert!(initial.contains("timeouts"));

    let followup = build_followup_prompt(&cfg, 1);
    assert!(followup.contains("CODING FOCUS"));
    assert!(followup.contains("protocols"));
    assert!(followup.contains("Iteration 2/2"));
}

#[test]
fn research_focus_requires_document_and_guardrails() {
    let mut cfg = fixture();
    cfg.prompt_mode = PromptMode::Research;
    cfg.research_specialty = CodingSpecialty::Backend;

    let initial = build_initial_prompt(&cfg);
    assert!(initial.contains("RESEARCH FOCUS: Backend"));
    assert!(initial.contains("evidence-led report"));
    assert!(!initial.contains(".md file"));
    assert!(!initial.contains("easy-to-copy Markdown"));
    assert!(initial.contains("Verify paths"));
    assert!(initial.contains("Do not invent"));

    cfg.iterations_per_task = 4;
    let followup = build_followup_prompt(&cfg, 3);
    assert!(followup.contains("revised report"));
    assert!(followup.contains("hallucinated paths"));
}

#[test]
fn global_tool_rules_are_injected_into_prompts() {
    let cfg = fixture();
    let initial = build_initial_prompt(&cfg);
    let followup = build_followup_prompt(&cfg, 1);
    let standard = build_standard_prompt(&cfg, 0, 0, cfg.iterations_per_task);

    for prompt in [initial, followup, standard] {
        assert!(prompt.contains("Use high-yield tool calls"));
        assert!(prompt.contains("fetch/read real source"));
        assert!(!prompt.contains("clone or otherwise"));
        assert!(prompt.contains("re-check branch/commit/head"));
    }
}

#[test]
fn prompts_include_context_as_source_of_truth() {
    let cfg = fixture();

    let initial = build_initial_prompt(&cfg);
    let followup = build_followup_prompt(&cfg, 1);

    for prompt in [initial, followup] {
        assert!(prompt.contains("CONTEXT:"));
        assert!(prompt.contains("Runs on a Windows automation bot."));
    }
}

#[test]
fn research_iterations_are_split_into_four_stages() {
    let mut cfg = fixture();
    cfg.prompt_mode = PromptMode::Research;
    cfg.research_specialty = CodingSpecialty::BugFinding;
    cfg.iterations_per_task = 8;

    let prompts = [
        build_initial_prompt(&cfg),
        build_followup_prompt(&cfg, 1),
        build_followup_prompt(&cfg, 2),
        build_followup_prompt(&cfg, 3),
        build_followup_prompt(&cfg, 4),
        build_followup_prompt(&cfg, 5),
        build_followup_prompt(&cfg, 6),
        build_followup_prompt(&cfg, 7),
    ];

    assert!(prompts[0].contains("RESEARCH STAGE: FETCH"));
    assert!(prompts[1].contains("RESEARCH STAGE: FETCH"));
    assert!(prompts[2].contains("RESEARCH STAGE: DISCOVERY"));
    assert!(prompts[3].contains("RESEARCH STAGE: DISCOVERY"));
    assert!(prompts[4].contains("RESEARCH STAGE: SELECTED ANALYSIS"));
    assert!(prompts[5].contains("RESEARCH STAGE: SELECTED ANALYSIS"));
    assert!(prompts[6].contains("RESEARCH STAGE: VERIFY"));
    assert!(prompts[7].contains("RESEARCH STAGE: VERIFY"));
    assert!(prompts[4].contains("reachable failure paths"));
}

#[test]
fn research_stage_remainder_prefers_later_stages() {
    let mut cfg = fixture();
    cfg.prompt_mode = PromptMode::Research;

    cfg.iterations_per_task = 5;
    assert!(build_initial_prompt(&cfg).contains("RESEARCH STAGE: FETCH"));
    assert!(build_followup_prompt(&cfg, 1).contains("RESEARCH STAGE: DISCOVERY"));
    assert!(build_followup_prompt(&cfg, 2).contains("RESEARCH STAGE: SELECTED ANALYSIS"));
    assert!(build_followup_prompt(&cfg, 3).contains("RESEARCH STAGE: VERIFY"));
    assert!(build_followup_prompt(&cfg, 4).contains("RESEARCH STAGE: VERIFY"));

    cfg.iterations_per_task = 6;
    assert!(build_followup_prompt(&cfg, 2).contains("RESEARCH STAGE: SELECTED ANALYSIS"));
    assert!(build_followup_prompt(&cfg, 3).contains("RESEARCH STAGE: SELECTED ANALYSIS"));
    assert!(build_followup_prompt(&cfg, 4).contains("RESEARCH STAGE: VERIFY"));
    assert!(build_followup_prompt(&cfg, 5).contains("RESEARCH STAGE: VERIFY"));

    cfg.iterations_per_task = 7;
    assert!(build_followup_prompt(&cfg, 1).contains("RESEARCH STAGE: DISCOVERY"));
    assert!(build_followup_prompt(&cfg, 2).contains("RESEARCH STAGE: DISCOVERY"));
    assert!(build_followup_prompt(&cfg, 3).contains("RESEARCH STAGE: SELECTED ANALYSIS"));
    assert!(build_followup_prompt(&cfg, 4).contains("RESEARCH STAGE: SELECTED ANALYSIS"));
    assert!(build_followup_prompt(&cfg, 5).contains("RESEARCH STAGE: VERIFY"));
    assert!(build_followup_prompt(&cfg, 6).contains("RESEARCH STAGE: VERIFY"));
}

#[test]
fn research_followup_can_include_previous_response_context() {
    let mut cfg = fixture();
    cfg.prompt_mode = PromptMode::Research;
    cfg.iterations_per_task = 4;

    let p = build_research_followup_prompt_with_previous_response(
        &cfg,
        1,
        "PREVIOUS FINDING: copy button coordinates wrong",
    );

    assert!(p.contains("PREVIOUS RESPONSE"));
    assert!(p.contains("PREVIOUS FINDING: copy button coordinates wrong"));
    assert!(p.contains("CURRENT PROMPT"));
    assert!(p.contains("Iteration 2/4"));
}

#[test]
fn followup_prompt_reports_total_iterations() {
    let mut cfg = fixture();
    cfg.iterations_per_task = 40;

    let second_prompt = build_followup_prompt(&cfg, 1);
    assert!(second_prompt.contains("Iteration 2/40"));
    assert!(second_prompt.contains("follow-up 1"));
}

#[test]
fn arena_surface_tracks_prompt_mode_without_sharing_identity() {
    assert_eq!(
        ArenaSurface::from_prompt_mode(PromptMode::Coding),
        ArenaSurface::Code
    );
    assert_eq!(
        ArenaSurface::from_prompt_mode(PromptMode::Research),
        ArenaSurface::Text
    );

    let code = Regions::for_screen_and_surface(1512, 982, ArenaSurface::Code);
    let text = Regions::for_screen_and_surface(1512, 982, ArenaSurface::Text);

    assert_eq!(code.surface, ArenaSurface::Code);
    assert_eq!(text.surface, ArenaSurface::Text);
}
