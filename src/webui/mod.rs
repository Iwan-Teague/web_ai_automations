//! Web-UI automation layer.
//!
//! Five concerns, one per submodule:
//!
//! - [`adapter`]   — `WebUiAdapter` trait, perceived screen state, anchors.
//! - [`fsm`]       — execution finite-state-machine that drives one prompt
//!                   submission to completion.
//! - [`watchdog`]  — environment invariants (browser foreground, URL, no
//!                   unexpected modal, etc.).
//! - [`recovery`]  — recovery action library + dispatcher.
//! - [`runtime`]   — crash-resilient resume state (`session/runtime.json`).
//! - [`state_log`] — append-only JSONL log of every FSM transition.
//! - [`null`]      — `NullAdapter` used when no real UI is targeted.
//!
//! Per-site implementations live in submodules: [`arena`] (Arena.ai) is the
//! first one. New sites add a sibling module; the runner consumes them only
//! through the trait.

pub mod adapter;
pub mod arena;
pub mod browser_state;
pub mod deepseek;
pub mod fsm;
pub mod null;
pub mod recovery;
pub mod runtime;
pub mod state_log;
pub mod watchdog;
