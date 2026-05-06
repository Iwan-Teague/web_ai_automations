# Site Workflows

The runner is site-agnostic. It asks `runner::site_workflow` for a prepared
site and then drives the returned `WebUiAdapter` through the shared FSM.

## Current Sites

| Site | Status | Notes |
|---|---|---|
| `arena.ai` | implemented | Owns Arena model selection, text/code surface choice, setup verification, and Arena-specific matrix comparisons. |
| `Null` | implemented | Local smoke-test adapter. No browser automation. |
| `ChatGPT` | placeholder | Listed in the config enum only. Needs a `webui` adapter and workflow before intake can select it. |
| `Claude` | placeholder | Listed in the config enum only. Needs a `webui` adapter and workflow before intake can select it. |
| `DeepSeek` | placeholder | Listed in the config enum only. Needs a `webui` adapter and workflow before intake can select it. |

## Adding a Site

1. Add `src/webui/<site>/` with the same shape as Arena where useful:
   `adapter.rs`, `regions.rs`, `templates.rs`, and optional setup code.
2. Implement `WebUiAdapter` for the site adapter.
3. Add a `SiteWorkflow` implementation in `src/runner/site_workflow.rs`.
4. Return the workflow from `workflow_for`.
5. Enable it in `intake::simplified::read_site`.

Keep model selection inside the site workflow. Arena model selection is not a
generic runner concern, and future sites should have their own model-picker
logic and asset templates.
