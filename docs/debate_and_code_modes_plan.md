# Debate Mode and Coding Submodes Plan

## Goal

Extend the simplified setup flow with:

- Coding submodes that tailor only the generated prompts.
- Optional debate mode that runs two review chats for every main output:
  - Good cop: why the output is strong.
  - Bad cop: why the output is weak or risky.
- Debate summaries feed into the next main prompt.

## Coding Submodes

Only active when `PromptMode::Coding` is selected. These should not change Arena navigation, model selection, artifact download, or browser automation. They only add prompt guidance.

Initial set:

1. Web Design
2. Backend
3. Full Stack Application
4. Networking
5. Bug Finding
6. Testing and QA
7. Refactor and Architecture
8. DevOps and Tooling

Each submode provides:

- expected output style
- risks to prioritize
- extra review checklist for follow-up prompts

## Research Mode Submodes

Research mode uses the same focus categories as coding mode, but every prompt must request a document/report instead of a project:

1. General
2. Web Design
3. Backend
4. Full Stack Application
5. Networking
6. Bug Finding
7. Testing and QA
8. Refactor and Architecture
9. DevOps and Tooling

Research prompts must include guardrails:

- Return a document/report, not a project.
- Verify recommended directories/files against the real project structure.
- Mark assumptions when something has not been inspected.
- Provide sample code/commands where useful.
- Do not invent dependencies, services, APIs, or filesystem paths.

## Debate Mode

Setup option:

- `Debate mode? [y/N]`

Debate mode is only available in Research mode. It is intentionally limited to report-style outputs that can be copied/pasted into Good Cop and Bad Cop chats. It should not be available for big coding/project-output runs where copying the whole generated project is unreliable.

If enabled, every main research output gets a debate. With 2 follow-up turns, there are 3 main outputs and 3 debates.

Per main output:

1. Save/download main output artifacts.
2. Start a new chat for Good Cop.
3. Send Good Cop prompt with:
   - end goal
   - constraints
   - coding submode guidance if coding
   - main output text or artifact summary
4. Save Good Cop response.
5. Start a new chat for Bad Cop.
6. Send Bad Cop prompt with same context.
7. Save Bad Cop response.
8. Combine both into `debate_feedback.md`.
9. Inject combined feedback into the next main prompt.

## Prompt Carry-Forward

Next main prompt should include:

```text
═══ PRIOR GOOD-COP REVIEW ═══
...

═══ PRIOR BAD-COP REVIEW ═══
...

Use this feedback to improve the next iteration. Do not restart from scratch unless the critique proves the direction is unsalvageable.
```

## Arena View Modes

Arena has two materially different Direct-mode views:

- Coding mode must use `/code/direct`.
- Research mode must use `/text/direct`.

The current matrix comparisons are calibrated for the Code view. Keep those
regions and thresholds owned by the Code surface. Text view gets a separate
region/template scaffold so screenshots can be captured and calibrated without
weakening the working Code automation.

Text-view TODOs:

- Capture fresh send/stop button states.
- Capture in-chat send/stop button states.
- Capture any response-copy or report-export affordances if present.
- Capture model picker and Direct-mode header in Text filter.
- Add text-specific completion checks once the screenshots are available.

## Navigation Rules

Main chat and debate chats must be separate.

Required adapter capabilities:

- Start fresh chat from active chat.
- Return to the main chat after debate, or keep a recorded main chat URL and reopen it.
- Avoid sending debate prompts into the main chat.
- Avoid sending next main prompt into a debate chat.

Recommended implementation path:

1. Add session fields for coding submode and debate enablement.
2. Add prompt tailoring for coding submodes.
3. Add output files for debate feedback.
4. Add adapter support for preserving and restoring the main chat URL.
5. Implement Good Cop new-chat run.
6. Implement Bad Cop new-chat run.
7. Combine feedback and inject into next main prompt.
8. Add resume/runtime metadata so interrupted debate runs can resume safely.

## Safety Checks

Before sending any prompt:

- Verify current URL belongs to expected chat role: main, good-cop, or bad-cop.
- Verify input box is ready.
- Verify send button matrix location.

Before proceeding to next main prompt:

- Verify main output completed.
- Attempt artifact download if active Download button is detected.
- Save debate feedback file if debate mode is enabled.
