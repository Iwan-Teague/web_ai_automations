//! Runner — execution layer that drives prompts through the FSM and saves
//! their results.
//!
//! Public submodules:
//! - [`task_runner`]      — outer task × iteration loop, picks the adapter.
//! - [`debate_runner`]    — bull → bear → judge orchestration.
//! - [`constraint_check`] — post-response keyword/pattern scan.

pub mod constraint_check;
pub mod debate_runner;
pub mod site_workflow;
pub mod task_runner;
