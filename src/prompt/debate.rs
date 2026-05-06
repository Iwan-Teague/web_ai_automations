use crate::session::{DebateStrength, SessionConfig};

use super::builder::format_constraints_with_global;

/// Bull prompt — argue FOR the proposed change.
pub fn build_bull_prompt(cfg: &SessionConfig, task_output: &str) -> String {
    let constraints = format_constraints_with_global(cfg);

    format!(
        "END GOAL:
{end_goal}

CONSTRAINTS:
{constraints}

ROLE: BULL — argue FOR these changes
Stance strength: {strength}
{strength_note}

Build strongest evidence-backed case FOR accepting this output/change. Be specific; cite code/docs where possible.

{task_output}
",
        end_goal = cfg.end_goal,
        constraints = constraints,
        strength = cfg.bull_strength,
        strength_note = strength_note(cfg.bull_strength, true),
        task_output = task_output,
    )
}

/// Bear prompt — argue AGAINST the proposed change.
pub fn build_bear_prompt(cfg: &SessionConfig, task_output: &str) -> String {
    let constraints = format_constraints_with_global(cfg);

    format!(
        "END GOAL:
{end_goal}

CONSTRAINTS:
{constraints}

ROLE: BEAR — argue AGAINST these changes
Stance strength: {strength}
{strength_note}

Build strongest evidence-backed case AGAINST this output/change: risks, errors, edge cases, constraint violations, missed goal. Be specific.

{task_output}
",
        end_goal = cfg.end_goal,
        constraints = constraints,
        strength = cfg.bear_strength,
        strength_note = strength_note(cfg.bear_strength, false),
        task_output = task_output,
    )
}

/// Judge prompt — independent verdict, given the original output plus both cases.
pub fn build_judge_prompt(
    cfg: &SessionConfig,
    debate_round: usize,
    task_output: &str,
    bull_response: &str,
    bear_response: &str,
) -> String {
    let constraints = format_constraints_with_global(cfg);

    format!(
        "END GOAL:
{end_goal}

Success looks like: {success_criteria}

CONSTRAINTS:
{constraints}

ROLE: JUDGE — independent review
Round {round} of {total_rounds}

Read A original, B pro, C con. Return: verdict ACCEPT / REJECT / REVISE; concise reasoning citing A/B/C; revise list if needed; score /10.

[A] ORIGINAL
{task_output}

[B] FOR
{bull_response}

[C] AGAINST
{bear_response}
",
        end_goal = cfg.end_goal,
        success_criteria = cfg.success_criteria,
        constraints = constraints,
        round = debate_round + 1,
        total_rounds = cfg.debate_rounds,
        task_output = task_output,
        bull_response = bull_response,
        bear_response = bear_response,
    )
}

/// Per-strength phrasing that nudges the AI's tone without redefining the role.
fn strength_note(strength: DebateStrength, bull: bool) -> &'static str {
    match (strength, bull) {
        (DebateStrength::Mild, true) => "Lean supportive. Note minor concerns honestly.",
        (DebateStrength::Mild, false) => "Raise concerns clearly while acknowledging merit.",
        (DebateStrength::Balanced, true) => "Argue clearly for the change; acknowledge tradeoffs.",
        (DebateStrength::Balanced, false) => "Argue clearly against the change; acknowledge merit.",
        (DebateStrength::Strong, true) => {
            "Forcefully advocate. Press hard against weak counter-arguments."
        }
        (DebateStrength::Strong, false) => "Forcefully oppose. Hunt every weakness.",
        (DebateStrength::DevilsAdvocate, true) => {
            "Argue FOR a position you may personally find weaker. Surface the strongest possible pro arguments anyway."
        }
        (DebateStrength::DevilsAdvocate, false) => {
            "Argue AGAINST something you may personally find reasonable. Surface the strongest possible objections anyway."
        }
    }
}
