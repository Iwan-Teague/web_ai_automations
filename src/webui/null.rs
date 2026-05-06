//! `NullAdapter` — answers every method without touching a real screen.
//!
//! Used when:
//! - The session targets `WebUiTarget::Null` (CI / smoke tests).
//! - The host is non-Windows and no real adapter exists.
//! - FSM unit tests need a deterministic adapter.
//!
//! The adapter walks a scripted state sequence so the FSM can run end-to-end
//! without any real UI. Each state in the script is held for
//! `polls_per_state` consecutive `detect_state` calls so the FSM's
//! stability filter has something to lock onto. When the script is
//! exhausted, the final state repeats forever.

use std::sync::Mutex;

use crate::webui::adapter::{AdapterError, WebUiAdapter, WebUiState};

#[derive(Debug, Clone)]
pub struct StateScript {
    pub states: Vec<WebUiState>,
}

impl Default for StateScript {
    fn default() -> Self {
        Self {
            states: vec![
                WebUiState::Ready,
                WebUiState::Generating,
                WebUiState::Complete,
            ],
        }
    }
}

pub struct NullAdapter {
    script: Mutex<StateScript>,
    /// Total `detect_state` calls since the last script reset.
    calls: Mutex<usize>,
    last_text: Mutex<Option<String>>,
    polls_per_state: usize,
    model: Option<String>,
}

impl NullAdapter {
    pub fn new() -> Self {
        Self::with_script(StateScript::default())
    }

    pub fn with_script(script: StateScript) -> Self {
        Self::with_script_and_polls(script, 5)
    }

    pub fn with_script_and_polls(script: StateScript, polls_per_state: usize) -> Self {
        Self {
            script: Mutex::new(script),
            calls: Mutex::new(0),
            last_text: Mutex::new(None),
            polls_per_state: polls_per_state.max(1),
            model: Some("null-model".to_string()),
        }
    }

    /// Replace the script mid-run and reset the call counter.
    pub fn replace_script(&self, script: StateScript) {
        if let Ok(mut s) = self.script.lock() {
            *s = script;
        }
        if let Ok(mut c) = self.calls.lock() {
            *c = 0;
        }
    }
}

impl Default for NullAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl WebUiAdapter for NullAdapter {
    fn name(&self) -> &'static str {
        "null"
    }

    fn detect_state(&self) -> Result<WebUiState, AdapterError> {
        let script = self
            .script
            .lock()
            .map_err(|_| AdapterError::Other("script lock poisoned".into()))?;
        let mut calls = self
            .calls
            .lock()
            .map_err(|_| AdapterError::Other("calls lock poisoned".into()))?;

        if script.states.is_empty() {
            return Ok(WebUiState::Unknown);
        }

        let bucket = (*calls) / self.polls_per_state;
        let idx = bucket.min(script.states.len() - 1);
        let s = script.states[idx];
        *calls += 1;
        Ok(s)
    }

    fn detect_model(&self) -> Result<Option<String>, AdapterError> {
        Ok(self.model.clone())
    }

    fn submit_prompt(&self, text: &str) -> Result<(), AdapterError> {
        if let Ok(mut g) = self.last_text.lock() {
            *g = Some(text.to_string());
        }
        if let Ok(mut c) = self.calls.lock() {
            *c = 0;
        }
        Ok(())
    }

    fn copy_response(&self) -> Result<String, AdapterError> {
        let g = self
            .last_text
            .lock()
            .map_err(|_| AdapterError::Other("last_text lock poisoned".into()))?;
        Ok(format!(
            "[null adapter response]\nLast prompt was:\n{}",
            g.as_deref().unwrap_or("<no prompt submitted>"),
        ))
    }

    fn start_new_chat(&self) -> Result<(), AdapterError> {
        if let Ok(mut c) = self.calls.lock() {
            *c = 0;
        }
        Ok(())
    }

    fn dismiss_modal(&self) -> Result<bool, AdapterError> {
        Ok(false)
    }
}
