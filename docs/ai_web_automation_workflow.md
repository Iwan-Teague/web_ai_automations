# AI Web Automation — Workflow Design
**Project:** Automated browser-driven AI prompting system  
**Language:** Rust  
**Status:** Design / Specification

---

## Overview

This system automates the process of submitting structured prompts to an AI via a web browser, collecting responses, and running configurable review loops — including a **Debate Mode** where one AI argues for a change, another argues against it, and a third acts as an independent judge.

The user is guided through a staged intake flow at startup. Every subsequent AI interaction is anchored back to the user's stated end goal and constraints, so the AI stays on track across many iterations.

---

## Part 1 — Startup Prompt (What the User Sees)

When the project starts, the user is presented with a staged intake sequence. Each stage must be completed before the next is shown. Partially completed stages are saved to a local `session.json` so the user can resume.

---

### Stage 1 — Project Source

```
╔══════════════════════════════════════════════════════════════════╗
║  STAGE 1 OF 5 — PROJECT SOURCE                                  ║
╠══════════════════════════════════════════════════════════════════╣
║                                                                  ║
║  Paste your GitHub repository URL (or local file path):         ║
║  ┌────────────────────────────────────────────────────────────┐ ║
║  │                                                            │ ║
║  └────────────────────────────────────────────────────────────┘ ║
║                                                                  ║
║  Specific branch or commit (leave blank for default branch):    ║
║  ┌────────────────────────────────────────────────────────────┐ ║
║  │                                                            │ ║
║  └────────────────────────────────────────────────────────────┘ ║
║                                                                  ║
║  What type of content is this?                                  ║
║    ( ) Rust codebase      ( ) Other code                        ║
║    ( ) Document / spec    ( ) Mixed                             ║
║                                                                  ║
╚══════════════════════════════════════════════════════════════════╝
```

**Fields collected:**
- `repo_url` — GitHub link or local path
- `branch` — optional, defaults to `main`
- `content_type` — enum used to tune prompt phrasing downstream

---

### Stage 2 — End Goal

```
╔══════════════════════════════════════════════════════════════════╗
║  STAGE 2 OF 5 — END GOAL                              ★ KEY ★  ║
╠══════════════════════════════════════════════════════════════════╣
║                                                                  ║
║  State your end goal clearly and specifically.                  ║
║  This will be shown to the AI at the start of every prompt.    ║
║                                                                  ║
║  ┌────────────────────────────────────────────────────────────┐ ║
║  │                                                            │ ║
║  │  e.g. "Produce a production-ready Rust image comparison   │ ║
║  │  module with full test coverage and no unsafe blocks."    │ ║
║  │                                                            │ ║
║  └────────────────────────────────────────────────────────────┘ ║
║                                                                  ║
║  How will you know the goal is achieved? (success criteria)    ║
║  ┌────────────────────────────────────────────────────────────┐ ║
║  │                                                            │ ║
║  └────────────────────────────────────────────────────────────┘ ║
║                                                                  ║
╚══════════════════════════════════════════════════════════════════╝
```

**Fields collected:**
- `end_goal` — the north-star statement injected into every prompt
- `success_criteria` — plain-language definition of done

> The end goal is the single most important input. It is bolded and placed at the top of every AI prompt in every stage and iteration, so the AI is always oriented toward the same target regardless of how many loops have run.

---

### Stage 3 — Constraints and Context

```
╔══════════════════════════════════════════════════════════════════╗
║  STAGE 3 OF 5 — CONSTRAINTS & CONTEXT                          ║
╠══════════════════════════════════════════════════════════════════╣
║                                                                  ║
║  What must the AI never do or suggest? (hard constraints)       ║
║  ┌────────────────────────────────────────────────────────────┐ ║
║  │  e.g. "Do not change the public API. No nightly features.  │ ║
║  │  Do not introduce tokio. Target Windows only."             │ ║
║  └────────────────────────────────────────────────────────────┘ ║
║                                                                  ║
║  Background context the AI needs to understand your project:   ║
║  ┌────────────────────────────────────────────────────────────┐ ║
║  │  e.g. "This runs on a Windows automation bot. Screen       │ ║
║  │  captures are 1080p. All comparison functions must be      │ ║
║  │  deterministic across threads."                            │ ║
║  └────────────────────────────────────────────────────────────┘ ║
║                                                                  ║
║  Tech stack / language constraints:                             ║
║  ┌────────────────────────────────────────────────────────────┐ ║
║  │                                                            │ ║
║  └────────────────────────────────────────────────────────────┘ ║
║                                                                  ║
╚══════════════════════════════════════════════════════════════════╝
```

**Fields collected:**
- `hard_constraints` — lines injected as a "YOU MUST NOT" block in every prompt
- `background_context` — paragraph injected as a "Context" block
- `tech_constraints` — language/framework restrictions

> Constraints are shown to the AI as a clearly labelled block every time. If a response violates a constraint, the system can flag it and optionally re-prompt automatically.

---

### Stage 4 — Tasks

```
╔══════════════════════════════════════════════════════════════════╗
║  STAGE 4 OF 5 — TASKS                                          ║
╠══════════════════════════════════════════════════════════════════╣
║                                                                  ║
║  List the specific tasks you want the AI to work through.      ║
║  Add one task per line. They will be run in order.             ║
║                                                                  ║
║  ┌────────────────────────────────────────────────────────────┐ ║
║  │  1. Review the image comparison functions for correctness  │ ║
║  │  2. Suggest optimisations to the sliding-window search     │ ║
║  │  3. Identify any race conditions in the Rayon usage        │ ║
║  │  4.                                                        │ ║
║  └────────────────────────────────────────────────────────────┘ ║
║                                                                  ║
║  How many prompt iterations per task?  [ 1 ]                   ║
║                                                                  ║
║  After each iteration:                                          ║
║    (●) Show output and wait for confirmation before continuing  ║
║    ( ) Run all iterations automatically                         ║
║                                                                  ║
╚══════════════════════════════════════════════════════════════════╝
```

**Fields collected:**
- `tasks[]` — ordered list of task strings
- `iterations_per_task` — integer, how many times each task is prompted
- `auto_continue` — bool, whether to pause between iterations

---

### Stage 5 — Review Mode

```
╔══════════════════════════════════════════════════════════════════╗
║  STAGE 5 OF 5 — REVIEW MODE                                    ║
╠══════════════════════════════════════════════════════════════════╣
║                                                                  ║
║  Choose how output is reviewed:                                 ║
║                                                                  ║
║    (●) Standard — AI reviews its own output once               ║
║    ( ) Debate Mode — see below                                  ║
║    ( ) No review — raw output only                              ║
║                                                                  ║
║  ┌─ DEBATE MODE ──────────────────────────────────────────────┐ ║
║  │                                                            │ ║
║  │  Bull AI    — argues FOR the proposed changes             │ ║
║  │  Bear AI    — argues AGAINST the proposed changes         │ ║
║  │  Judge AI   — reviews both positions independently        │ ║
║  │                                                            │ ║
║  │  Debate rounds:                          [ 1 ]            │ ║
║  │                                                            │ ║
║  │  After each Judge ruling:                                  │ ║
║  │    (●) Show ruling and wait                               │ ║
║  │    ( ) Auto-apply Judge recommendation                    │ ║
║  │                                                            │ ║
║  │  Bull stance strength:  [ Balanced ▼ ]                    │ ║
║  │  Bear stance strength:  [ Balanced ▼ ]                    │ ║
║  │    Options: Mild / Balanced / Strong / Devil's Advocate   │ ║
║  └────────────────────────────────────────────────────────────┘ ║
║                                                                  ║
║  [ CONFIRM AND START ]                                          ║
╚══════════════════════════════════════════════════════════════════╝
```

**Fields collected:**
- `review_mode` — enum: `Standard | Debate | None`
- `debate_rounds` — integer
- `bull_strength` / `bear_strength` — enum: `Mild | Balanced | Strong | DevilsAdvocate`
- `auto_apply_judge` — bool

---

## Part 2 — Session Config Schema

All intake data is written to `session.json` in the project directory before any prompting begins.

```rust
#[derive(Serialize, Deserialize)]
pub struct SessionConfig {
    // Stage 1
    pub repo_url:       String,
    pub branch:         Option<String>,
    pub content_type:   ContentType,

    // Stage 2
    pub end_goal:         String,
    pub success_criteria: String,

    // Stage 3
    pub hard_constraints:  Vec<String>,
    pub background_context: String,
    pub tech_constraints:  String,

    // Stage 4
    pub tasks:               Vec<String>,
    pub iterations_per_task: usize,
    pub auto_continue:       bool,

    // Stage 5
    pub review_mode:      ReviewMode,
    pub debate_rounds:    usize,
    pub bull_strength:    DebateStrength,
    pub bear_strength:    DebateStrength,
    pub auto_apply_judge: bool,
}

#[derive(Serialize, Deserialize)]
pub enum ContentType { RustCode, OtherCode, Document, Mixed }

#[derive(Serialize, Deserialize)]
pub enum ReviewMode { Standard, Debate, None }

#[derive(Serialize, Deserialize)]
pub enum DebateStrength { Mild, Balanced, Strong, DevilsAdvocate }
```

---

## Part 3 — Prompt Assembly

Every prompt sent to the AI is assembled from the same template. The end goal and constraints appear at the top of every single prompt without exception.

### 3.1 Standard Prompt Template

```
═══════════════════════════════════════
END GOAL (keep this in mind at all times)
═══════════════════════════════════════
{end_goal}

Success looks like: {success_criteria}

═══════════════════════════════════════
CONSTRAINTS — you must not violate these
═══════════════════════════════════════
{hard_constraints joined by newline}

Tech constraints: {tech_constraints}

═══════════════════════════════════════
CONTEXT
═══════════════════════════════════════
{background_context}

Source: {repo_url} ({branch})

═══════════════════════════════════════
TASK  [{task_index + 1} of {total_tasks}]  —  Iteration {iteration} of {total_iterations}
═══════════════════════════════════════
{current_task}
```

---

### 3.2 Debate Mode — Bull Prompt

Sent to the AI after the standard task prompt has produced output.

```
═══════════════════════════════════════
END GOAL
═══════════════════════════════════════
{end_goal}

═══════════════════════════════════════
CONSTRAINTS
═══════════════════════════════════════
{hard_constraints}

═══════════════════════════════════════
ROLE: BULL — argue FOR these changes
═══════════════════════════════════════
Stance strength: {bull_strength}

Below is a proposed change or review output. Your job is to build the
strongest possible case FOR why this change is correct, beneficial, and
should be accepted. Be specific. Reference the code or document directly.

{task_output}
```

---

### 3.3 Debate Mode — Bear Prompt

```
═══════════════════════════════════════
END GOAL
═══════════════════════════════════════
{end_goal}

═══════════════════════════════════════
CONSTRAINTS
═══════════════════════════════════════
{hard_constraints}

═══════════════════════════════════════
ROLE: BEAR — argue AGAINST these changes
═══════════════════════════════════════
Stance strength: {bear_strength}

Below is a proposed change or review output. Your job is to build the
strongest possible case AGAINST this change — identify risks, errors,
missed edge cases, violations of the constraints above, or ways the
end goal is NOT served. Be specific and critical.

{task_output}
```

---

### 3.4 Debate Mode — Judge Prompt

The judge receives the original output, the Bull case, and the Bear case.
It does not know which is which until it has formed its own view.

```
═══════════════════════════════════════
END GOAL
═══════════════════════════════════════
{end_goal}

Success looks like: {success_criteria}

═══════════════════════════════════════
CONSTRAINTS
═══════════════════════════════════════
{hard_constraints}

═══════════════════════════════════════
ROLE: JUDGE — independent review
═══════════════════════════════════════
Round {debate_round} of {debate_rounds}

You will be given three pieces of text:
  A — the original proposed change or review
  B — a case arguing FOR it
  C — a case arguing AGAINST it

Read all three independently and then provide:
  1. Your verdict: ACCEPT / REJECT / REVISE
  2. Your reasoning (cite A, B, and C specifically)
  3. If REVISE: a concrete list of what must change before acceptance
  4. A score out of 10 for how well the original serves the end goal

───────────────────────────────────────
[A] ORIGINAL OUTPUT
───────────────────────────────────────
{task_output}

───────────────────────────────────────
[B] CASE FOR
───────────────────────────────────────
{bull_response}

───────────────────────────────────────
[C] CASE AGAINST
───────────────────────────────────────
{bear_response}
```

---

## Part 4 — Execution Flow

```
START
  │
  ├─ Load session.json if it exists (resume)
  │   └─ else: run staged intake (Stages 1–5)
  │
  └─ For each task in tasks[]:
       │
       ├─ For iteration 1..=iterations_per_task:
       │    │
       │    ├─ Assemble standard prompt
       │    ├─ Submit to AI via browser automation
       │    ├─ Capture response
       │    │
       │    ├─ [if review_mode == Standard]
       │    │    └─ Submit self-review prompt → capture → save
       │    │
       │    ├─ [if review_mode == Debate]
       │    │    ├─ For round 1..=debate_rounds:
       │    │    │    ├─ Submit Bull prompt   → capture bull_response
       │    │    │    ├─ Submit Bear prompt   → capture bear_response
       │    │    │    ├─ Submit Judge prompt  → capture judge_response
       │    │    │    ├─ Parse verdict (ACCEPT / REJECT / REVISE)
       │    │    │    ├─ Save all three to outputs/task_{n}/round_{r}/
       │    │    │    └─ if auto_apply_judge: feed judge revision
       │    │    │         notes into next iteration's task context
       │    │    └─ end rounds
       │    │
       │    ├─ Save raw output to outputs/task_{n}/iter_{i}.md
       │    │
       │    └─ [if not auto_continue]: pause and wait for user input
       │
       └─ end iterations

  └─ Final summary: print judge verdicts, scores, and pass/fail
       against success_criteria
END
```

---

## Part 5 — Output Structure

```
project_root/
  session.json                    — saved intake config
  outputs/
    task_1/
      iter_1_raw.md               — raw AI output
      iter_1_self_review.md       — standard review (if enabled)
      iter_1_debate/
        round_1_bull.md
        round_1_bear.md
        round_1_judge.md          — includes verdict + score
      iter_2_raw.md
      ...
    task_2/
      ...
  summary.md                      — final report: verdicts, scores, goal assessment
```

---

## Part 6 — Constraint Violation Detection

After every AI response is captured, the system runs a lightweight check before saving:

```rust
pub fn check_constraint_violations(
    response:    &str,
    constraints: &[String],
) -> Vec<String> {
    // Returns a list of constraints that the response appears to violate.
    // Uses simple keyword heuristics — not a guarantee, just a flag.
    //
    // Examples:
    //   constraint: "do not use tokio"
    //   trigger:    response contains "tokio::" or "use tokio"
    //
    //   constraint: "no nightly features"
    //   trigger:    response contains "#![feature(" 
    //
    // Violations are logged and shown to the user before continuing.
    constraints
        .iter()
        .filter(|c| response_violates(response, c))
        .cloned()
        .collect()
}
```

If violations are found, the user is shown a warning and given three options:
- **Ignore and continue** — log it but proceed
- **Re-prompt** — resubmit the task with the violation explicitly flagged in the prompt
- **Stop** — halt the session and review manually

---

## Part 7 — Debate Strength Behaviour

| Strength | Bull behaviour | Bear behaviour |
|---|---|---|
| `Mild` | Supports the change, notes a few minor concerns | Raises concerns but acknowledges merit |
| `Balanced` | Argues clearly for, acknowledges tradeoffs | Argues clearly against, acknowledges merit |
| `Strong` | Forcefully advocates, dismisses concerns | Forcefully opposes, seeks every weakness |
| `DevilsAdvocate` | Argues for a position it likely disagrees with | Argues against something it likely agrees with |

The stance strength is injected into the prompt phrasing. At `DevilsAdvocate` level the AI is explicitly told to argue a position it may personally find weaker, to surface arguments the other side would have to answer.

---

## Part 8 — What Gets Built (Rust Module Structure)

```
src/
  main.rs                  — entry point, runs intake then dispatch loop
  session.rs               — SessionConfig, load/save session.json
  intake/
    mod.rs
    stage_1_source.rs      — repo URL input
    stage_2_goal.rs        — end goal + success criteria
    stage_3_constraints.rs — hard constraints + context
    stage_4_tasks.rs       — task list + iteration config
    stage_5_review.rs      — review mode + debate config
  prompt/
    mod.rs
    builder.rs             — assemble prompts from SessionConfig
    debate.rs              — bull / bear / judge prompt variants
  runner/
    mod.rs
    task_runner.rs         — outer task + iteration loop
    debate_runner.rs       — bull → bear → judge orchestration
    constraint_check.rs    — post-response violation detection
  browser/
    mod.rs                 — browser automation (existing code)
  output/
    mod.rs
    writer.rs              — save responses to outputs/
    summary.rs             — generate summary.md
```

---

## Part 9 — Things Still To Decide

- **Which AI endpoint** — does the browser target Claude, ChatGPT, Gemini, or is this configurable per stage? Likely a Stage 1 field.
- **Code injection** — does the system paste file contents into the prompt automatically, or rely on the AI having repo access (e.g. GitHub Copilot, Cursor)?
- **Judge auto-apply** — if the judge says REVISE, how are its revision notes merged into the next iteration's context? Appended to the task string, or passed as a new preamble block?
- **Multi-model debate** — could Bull and Bear be on different AI endpoints to get genuinely independent perspectives?
- **Session versioning** — if the user re-runs with a different goal, does the old session archive or overwrite?
