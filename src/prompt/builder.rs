use crate::session::{CodingSpecialty, PromptMode, SessionConfig};

pub const SOURCE_FETCH_CONSTRAINT: &str = "If repo/code/source evidence is needed, fetch/read real source with available tools before claiming. Do not pretend. If blocked, say what evidence is missing.";
pub const TOOL_EFFICIENCY_CONSTRAINT: &str =
    "Use high-yield tool calls: broad map/manifests/config/search first, targeted files second.";
pub const REPO_CLONE_CONSTRAINT: &str = "For named repos/sources, inspect real repo context and branch/commit/head before conclusions. Read tools are fine; literal clone is not required if source can be read accurately.";
pub const REPO_COMMIT_RECHECK_CONSTRAINT: &str =
    "On source-relevant follow-ups, re-check branch/commit/head before assuming unchanged.";

/// Constraint replacement used when the runner has attached a project
/// map. It tells the AI to treat the attachments as authoritative and to
/// stop trying to fetch the repo over the network — which is now both
/// redundant (we provide every file verbatim) and counterproductive (the
/// AI burns tool calls re-fetching code we've already given it).
pub const ATTACHED_MAP_CONSTRAINT: &str = "Attached project-map .txt files are authoritative. They contain verbatim files with numbered lines and `===== /repo/path =====` headers plus git state. Do not fetch the repo online. If needed code is missing, say so.";

pub fn format_constraints_with_global(cfg: &SessionConfig) -> String {
    let mut constraints = cfg.hard_constraints.clone();

    let has_map = !cfg.all_project_map_paths().is_empty();
    if has_map {
        // Map is attached: replace the four "go fetch the repo" constraints
        // with one that points the AI at the attachments and tells it not
        // to chase the source over the network.
        if !constraints.iter().any(|c| c == ATTACHED_MAP_CONSTRAINT) {
            constraints.push(ATTACHED_MAP_CONSTRAINT.to_string());
        }
        // Strip any of the legacy fetch-the-repo constraints if they
        // happened to be added by an older session.json.
        constraints.retain(|c| {
            c != SOURCE_FETCH_CONSTRAINT
                && c != REPO_CLONE_CONSTRAINT
                && c != REPO_COMMIT_RECHECK_CONSTRAINT
        });
        // Tool-efficiency advice is redundant once attachments are the only source.
    } else {
        for global in [
            SOURCE_FETCH_CONSTRAINT,
            TOOL_EFFICIENCY_CONSTRAINT,
            REPO_CLONE_CONSTRAINT,
            REPO_COMMIT_RECHECK_CONSTRAINT,
        ] {
            if !constraints.iter().any(|c| c == global) {
                constraints.push(global.to_string());
            }
        }
    }

    if constraints.is_empty() {
        "none".to_string()
    } else {
        constraints.join("\n")
    }
}

/// Assemble the standard task prompt — used for every task iteration.
///
/// `task_index` and `iteration` are 0-based; the rendered prompt shows them
/// as 1-based ordinals for human readability.
pub fn build_standard_prompt(
    cfg: &SessionConfig,
    task_index: usize,
    iteration: usize,
    total_iterations: usize,
) -> String {
    let total_tasks = cfg.tasks.len();
    let current_task = cfg
        .tasks
        .get(task_index)
        .map(String::as_str)
        .unwrap_or("(no task at this index)");

    let constraints = format_constraints_with_global(cfg);
    let branch = cfg.branch.as_deref().unwrap_or("default");

    let map_block = project_map_block(cfg);

    format!(
        "END GOAL:
{end_goal}

Success looks like: {success_criteria}

{map_block}CONSTRAINTS:
{constraints}

Tech constraints: {tech_constraints}

CONTEXT:
{background_context}

Source: {repo_url} ({branch})

TASK  [{ti} of {total_tasks}] — Iteration {it} of {total_iterations}:
{current_task}
",
        end_goal = cfg.end_goal,
        success_criteria = cfg.success_criteria,
        map_block = map_block,
        constraints = constraints,
        tech_constraints = cfg.tech_constraints,
        background_context = cfg.background_context,
        repo_url = cfg.repo_url,
        branch = branch,
        ti = task_index + 1,
        total_tasks = total_tasks,
        it = iteration + 1,
        total_iterations = total_iterations,
        current_task = current_task,
    )
}

/// Assemble the self-review prompt for `ReviewMode::Standard`.
///
/// Anchored back to the goal and constraints, then asks the AI to critique
/// its own previous output.
pub fn build_self_review_prompt(cfg: &SessionConfig, task_output: &str) -> String {
    let constraints = format_constraints_with_global(cfg);
    let has_map = !cfg.all_project_map_paths().is_empty();

    // Verification instruction — only mention the attached files when one
    // is actually attached. When no map is present, "verify against the
    // attached project files" is a contradiction the AI gets stuck on.
    let verification = if has_map {
        "Verify concrete claims against attached maps; cite path+line; flag missing evidence."
    } else {
        "Verify concrete claims with evidence; flag unsupported claims."
    };

    format!(
        "END GOAL:
{end_goal}

Success looks like: {success_criteria}

{map_block}CONSTRAINTS:
{constraints}

ROLE: SELF-REVIEW
Critique previous output against goal/constraints. {verification} Identify mistakes, gaps, constraint violations. Suggest smallest fixes.

{task_output}
",
        end_goal = cfg.end_goal,
        success_criteria = cfg.success_criteria,
        map_block = project_map_block(cfg),
        constraints = constraints,
        verification = verification,
        task_output = task_output,
    )
}

// ── Mode-aware prompt builders (simplified flow) ───────────────────────────

fn format_constraints(cfg: &SessionConfig) -> String {
    format_constraints_with_global(cfg)
}

fn coding_focus_instruction(focus: CodingSpecialty) -> &'static str {
    match focus {
        CodingSpecialty::General => "Correct, simple, maintainable implementation.",
        CodingSpecialty::WebDesign => {
            "Frontend polish: hierarchy, responsive layout, accessibility, states, stable styling."
        }
        CodingSpecialty::Backend => {
            "Backend: API contracts, data, persistence, validation, errors, observability, security."
        }
        CodingSpecialty::FullStackApplication => {
            "Full-stack: frontend/backend contracts, data flow, auth/session, user workflows."
        }
        CodingSpecialty::Networking => {
            "Networking: protocols, connectivity, timeouts, retries, DNS/routing, socket lifecycle."
        }
        CodingSpecialty::BugFinding => {
            "Debugging: reproduce, isolate root cause, assess regressions, fix minimally."
        }
        CodingSpecialty::TestingQa => {
            "Testing: coverage gaps, determinism, fixtures, integration boundaries, diagnostics."
        }
        CodingSpecialty::RefactorArchitecture => {
            "Architecture: boundaries, useful abstractions, compatibility, migration safety, low churn."
        }
        CodingSpecialty::DevOpsTooling => {
            "DevOps: builds, scripts, CI, environment assumptions, reproducibility, deploy safety."
        }
    }
}

fn coding_focus_review_checklist(focus: CodingSpecialty) -> &'static str {
    match focus {
        CodingSpecialty::General => "Check correctness, scope, edge cases, maintainability.",
        CodingSpecialty::WebDesign => "Check responsive layout, polish, a11y, overflow, states.",
        CodingSpecialty::Backend => {
            "Check APIs, validation, persistence, errors, security, logging."
        }
        CodingSpecialty::FullStackApplication => {
            "Check data flow, contract drift, auth/session, workflows, visible failures."
        }
        CodingSpecialty::Networking => {
            "Check protocols, timeouts, retries, partial failures, routing, cleanup."
        }
        CodingSpecialty::BugFinding => "Check root cause, minimal fix, regressions, adjacent bugs.",
        CodingSpecialty::TestingQa => {
            "Check coverage, determinism, fixtures, failure output, integration."
        }
        CodingSpecialty::RefactorArchitecture => {
            "Check boundaries, abstraction cost, migration safety, compatibility."
        }
        CodingSpecialty::DevOpsTooling => {
            "Check CI/builds, scripts, reproducibility, env assumptions, deploy safety."
        }
    }
}

fn research_focus_instruction(focus: CodingSpecialty) -> &'static str {
    match focus {
        CodingSpecialty::General => {
            "Build an evidence-led technical report. Separate facts, assumptions, and recommendations."
        }
        CodingSpecialty::WebDesign => {
            "Inspect screens/components/styles; judge UX, a11y, responsiveness, frontend risks."
        }
        CodingSpecialty::Backend => {
            "Map APIs, data, persistence, jobs/queues, auth, reliability, observability, migrations."
        }
        CodingSpecialty::FullStackApplication => {
            "Trace workflows across frontend/backend contracts, auth/session, persistence, deploy."
        }
        CodingSpecialty::Networking => {
            "Map protocols, trust, topology, DNS/routing, sockets, timeouts, NAT/relay, faults."
        }
        CodingSpecialty::BugFinding => {
            "Find concrete defects: root cause, reachable failure paths, repro, severity, fixes."
        }
        CodingSpecialty::TestingQa => {
            "Inventory tests; find coverage gaps, fixture/determinism issues, missing regressions."
        }
        CodingSpecialty::RefactorArchitecture => {
            "Map deps/modules; find boundary issues, migration risk, duplication, useful abstractions."
        }
        CodingSpecialty::DevOpsTooling => {
            "Inspect manifests/scripts/CI/config; assess builds, env assumptions, deploy safety."
        }
    }
}

fn anti_hallucination_guardrails_with_map() -> &'static str {
    "Report only. Ground project claims in attached maps; cite path + line. Missing evidence → say missing. Do not invent or browse repo online. Include examples only when useful."
}

fn anti_hallucination_guardrails_no_map() -> &'static str {
    "Report only. Use tools for real evidence. Fetch/read source when needed. Verify paths/APIs/commands/deps or mark assumptions. Missing evidence → say what to inspect. Do not invent."
}

fn anti_hallucination_guardrails(cfg: &SessionConfig) -> &'static str {
    if cfg.all_project_map_paths().is_empty() {
        anti_hallucination_guardrails_no_map()
    } else {
        anti_hallucination_guardrails_with_map()
    }
}

fn context_block(cfg: &SessionConfig) -> String {
    let context = cfg.background_context.trim();
    if context.is_empty() {
        String::new()
    } else {
        format!(
            "CONTEXT:
{context}
"
        )
    }
}

/// What an attached project-map file represents. Derived from the filename
/// the generator wrote: `rust_project_map_<scope>.txt` → `<scope>` is the
/// crate name (or `_workspace` for root-level docs).
#[derive(Debug, Clone)]
struct AttachedMapFile {
    /// Bare filename — e.g. `rust_project_map_rustynetd.txt`.
    filename: String,
    /// Human-readable description of what's inside.
    description: String,
}

/// Inspect a single attached path and describe what it summarises. The
/// filename is the only signal — it's deterministic for files written by
/// `project_map::generate`. Anything that doesn't match the expected
/// pattern falls back to a generic "full project map" label so a hand-
/// edited or legacy session.json still produces a sensible prompt.
fn describe_attached_file(path: &str) -> AttachedMapFile {
    let filename = std::path::Path::new(path)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(path)
        .to_string();

    // Pattern from project_map::generate: `<stem>_<scope>.<ext>`.
    // For our default `rust_project_map.txt` base, that's
    // `rust_project_map_<scope>.txt` where `<scope>` is either a crate
    // name or `_workspace`.
    let scope = filename
        .strip_prefix("rust_project_map_")
        .and_then(|rest| rest.strip_suffix(".txt"));

    let description = match scope {
        Some("_workspace") => {
            "workspace/root docs, manifests, configs, policy, uncategorised source".to_string()
        }
        Some(crate_name) => format!("crate `{crate_name}` source + Cargo.toml"),
        None => "project source dump".to_string(),
    };

    AttachedMapFile {
        filename,
        description,
    }
}

/// Build the labelled `filename → description` listing that goes in the
/// prompt. Public-within-module so other prompt builders can reuse the
/// same labelling without duplicating string templates.
fn render_attached_map_listing(paths: &[&str]) -> String {
    let mut s = String::new();
    for (i, p) in paths.iter().enumerate() {
        let entry = describe_attached_file(p);
        s.push_str(&format!(
            "{}. {} — {}\n",
            i + 1,
            entry.filename,
            entry.description
        ));
    }
    s.trim_end().to_string()
}

/// Header that points the model at the attached project-map files. Emitted
/// only when at least one map is configured. Each file is listed by real
/// filename alongside what it summarises (workspace overview vs. specific
/// crate), so the model can route claims about a particular crate to the
/// correct attachment.
fn project_map_block(cfg: &SessionConfig) -> String {
    let paths = cfg.all_project_map_paths();
    if paths.is_empty() {
        return String::new();
    }
    let listing = render_attached_map_listing(&paths);
    format!(
        "ATTACHED MAPS = source of truth. Files are verbatim with numbered lines and `===== /path =====` headers. Cite path+line. Do not fetch repo online. If missing, say missing.
{listing}
",
        listing = listing,
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResearchStage {
    Fetch,
    Discovery,
    Selected,
    Verify,
}

impl ResearchStage {
    fn for_iteration(iteration: usize, total_iterations: usize) -> Self {
        if total_iterations == 0 {
            return Self::Fetch;
        }

        let stage_counts = research_stage_counts(total_iterations);
        let mut end = 0usize;
        for (idx, count) in stage_counts.iter().enumerate() {
            end += count;
            if iteration.min(total_iterations - 1) < end {
                return match idx {
                    0 => Self::Fetch,
                    1 => Self::Discovery,
                    2 => Self::Selected,
                    _ => Self::Verify,
                };
            }
        }

        Self::Verify
    }

    fn title(self) -> &'static str {
        match self {
            Self::Fetch => "FETCH",
            Self::Discovery => "DISCOVERY",
            Self::Selected => "SELECTED ANALYSIS",
            Self::Verify => "VERIFY",
        }
    }

    fn instruction(self, focus: CodingSpecialty) -> String {
        match self {
            Self::Fetch => format!(
                "Gather evidence first. If repo/source named, read real source; confirm branch/commit when possible. Map layout/manifests/config/tests/high-value source. No final findings unless blocker.\nFocus: {}",
                research_focus_instruction(focus)
            ),
            Self::Discovery => format!(
                "Map project shape from evidence: modules, data flow, trust boundaries, deps, runtime assumptions, key files. Mark verified vs unknown.\nFocus: {}",
                research_focus_instruction(focus)
            ),
            Self::Selected => format!(
                "Do selected analysis. Produce concrete findings with evidence. For bug/security: reachable conditions, severity, impact, repro path, file/line refs.\nSelected focus: {}\n{}",
                focus,
                research_focus_instruction(focus)
            ),
            Self::Verify => {
                "Verify/harden findings. Re-check claims, remove weak/duplicate items, test assumptions, seek counterexamples, group by severity with evidence/uncertainty.".to_string()
            }
        }
    }
}

fn research_stage_counts(total_iterations: usize) -> [usize; 4] {
    if total_iterations == 0 {
        return [0, 0, 0, 0];
    }

    let base = total_iterations / 4;
    let mut counts = [base; 4];
    for idx in (0..4).rev().take(total_iterations % 4) {
        counts[idx] += 1;
    }
    counts
}

fn research_stage_block(cfg: &SessionConfig, iteration: usize) -> String {
    let stage = ResearchStage::for_iteration(iteration, cfg.iterations_per_task);
    format!(
        "═══ RESEARCH STAGE: {} ═══
{}",
        stage.title(),
        stage.instruction(cfg.research_specialty)
    )
}

pub fn build_initial_prompt(cfg: &SessionConfig) -> String {
    let constraints = format_constraints(cfg);
    match cfg.prompt_mode {
        PromptMode::Research => format!(
            "You are a technical research analyst. Produce concise evidence-led report.

RESEARCH FOCUS: {focus}
{focus_instruction}

{map_block}{context_block}

STAGE: {stage_block}

GUARDRAILS: {guardrails}

GOAL:
{goal}

CONSTRAINTS:
{constraints}

Output: clear sections; findings, evidence, uncertainty, recommendations. Be precise, not verbose.",
            focus = cfg.research_specialty,
            focus_instruction = research_focus_instruction(cfg.research_specialty),
            stage_block = research_stage_block(cfg, 0),
            map_block = project_map_block(cfg),
            context_block = context_block(cfg),
            guardrails = anti_hallucination_guardrails(cfg),
            goal = cfg.end_goal,
            constraints = constraints,
        ),
        PromptMode::Coding => format!(
            "You are an expert software engineer. Produce high-quality code/plan.

CODING FOCUS: {focus}
{focus_instruction}

GOAL:
{goal}

{map_block}{context_block}

CONSTRAINTS:
{constraints}

Keep scoped. Follow existing patterns. Include errors/tests where relevant. Explain briefly.",
            focus = cfg.coding_specialty,
            focus_instruction = coding_focus_instruction(cfg.coding_specialty),
            goal = cfg.end_goal,
            map_block = project_map_block(cfg),
            context_block = context_block(cfg),
            constraints = constraints,
        ),
    }
}

pub fn build_followup_prompt(cfg: &SessionConfig, turn: usize) -> String {
    let constraints = format_constraints(cfg);
    let prompt_number = turn + 1;
    let followup_number = turn;
    let total_prompts = cfg.iterations_per_task;
    let focus_section = if cfg.prompt_mode == PromptMode::Coding {
        format!(
            "CODING FOCUS:
{}
",
            coding_focus_instruction(cfg.coding_specialty)
        )
    } else if cfg.prompt_mode == PromptMode::Research {
        format!(
            "RESEARCH FOCUS:
{}

{}

GUARDRAILS: {}
",
            research_focus_instruction(cfg.research_specialty),
            research_stage_block(cfg, turn),
            anti_hallucination_guardrails(cfg),
        )
    } else {
        String::new()
    };
    let review_instruction =
        match cfg.prompt_mode {
            PromptMode::Research => {
                match ResearchStage::for_iteration(turn, cfg.iterations_per_task) {
                ResearchStage::Fetch => "Continue source gathering. Fill missing repo/source context before deep analysis. State what was read, blocked, and now evidenced.".to_string(),
                ResearchStage::Discovery => "Continue discovery. Turn source evidence into concise architecture/workflow map.".to_string(),
                ResearchStage::Selected => "Continue selected-focus analysis. Produce evidence-backed findings; avoid generic advice.".to_string(),
                ResearchStage::Verify => "Review previous analysis for contradictions, weak evidence, hallucinated paths/APIs/deps/commands, missed angles, and unclear actions. Return revised report only.".to_string(),
            }
            }
            PromptMode::Coding => format!(
                "Review previous code/plan for logic errors, edge cases, inconsistencies, constraints, missing validation/errors, needless complexity. Focus risks: {focus_review}. Return corrections/improvements.",
                focus_review = coding_focus_review_checklist(cfg.coding_specialty),
            ),
        };

    format!(
"{review_instruction}

{focus_section}

{map_block}GOAL:
{goal}

{context_block}

CONSTRAINTS:
{constraints}

Iteration {prompt_number}/{total_prompts}; follow-up {followup_number}. Build on previous response; do not restart.",
        review_instruction = review_instruction,
        focus_section = focus_section,
        map_block = project_map_block(cfg),
        goal = cfg.end_goal,
        context_block = context_block(cfg),
        constraints = constraints,
        prompt_number = prompt_number,
        total_prompts = total_prompts,
        followup_number = followup_number,
    )
}

pub fn build_research_followup_prompt_with_previous_response(
    cfg: &SessionConfig,
    turn: usize,
    previous_response: &str,
) -> String {
    let followup = build_followup_prompt(cfg, turn);
    wrap_prompt_with_previous_response(previous_response, &followup)
}

pub fn wrap_prompt_with_previous_response(previous_response: &str, prompt: &str) -> String {
    format!(
        "PREVIOUS RESPONSE (working context, not final truth):
{previous_response}

CURRENT PROMPT:
{prompt}",
    )
}
