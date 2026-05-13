//! NarratorHook — Phase 1 story 1.15.
//!
//! PreToolUse hook that emits a `Message{role:"assistant", ui:"narrate",
//! call_id}` event for every tool dispatch. Two paths:
//!
//! - **Templated** (0 tokens) for cheap tools: looked up in
//!   [`templates::template_for`] and a generic `"Invoking {tool}"`
//!   fallback for unlisted tools.
//! - **Classifier-slot LLM** (~50 tokens) for action-changing tools
//!   matched by [`NarrationConfig::llm_path`]. Configurable timeout
//!   (default 2 s); on timeout or LLM error the hook emits
//!   `Misc{kind:"narration_skipped"}` and returns — the agent loop
//!   never blocks on narration.
//!
//! Narration is UI signal only — the sticky-context builder
//! ([`crate::agent::prompt::build_messages`]) skips Message events
//! with `ui == "narrate"` so they never re-enter the agent's own
//! context (architecture §12 q2).
//!
//! refs: /specs/phase-1/architecture.md §2.8, §4.2, §7, §8
//! refs: /specs/phase-1/stories/story-1.15.md

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{Value, json};

use crate::dispatch::hooks::Hook;
use crate::events::{EventStore, EventType, NewEvent, sqlite::SqliteEventStore};
use crate::llm::LlmClient;
use crate::tools::{ToolContext, ToolError, ToolOutput};

mod classifier;
mod templates;

#[cfg(test)]
mod tests;

pub use classifier::ClassifierSlot;

pub const NARRATE_UI_TAG: &str = "narrate";
pub const DEFAULT_TIMEOUT_MS: u64 = 2000;

/// Compile-time default for which tool names route through the LLM
/// classifier path. Each entry is either an exact tool name or a glob
/// `<prefix>_*` matched by [`NarrationConfig::uses_llm_path`].
pub const DEFAULT_LLM_PATH: &[&str] = &[
    "file_write",
    "file_str_replace",
    "shell_*",
    "browser_*",
    "info_search_web",
    "deploy_*",
    "message_*",
];

#[derive(Debug, Clone)]
pub struct NarrationConfig {
    pub enabled: bool,
    /// Tool names (or `prefix_*` globs) that route through the LLM
    /// classifier. Everything else uses the templated path.
    pub llm_path: Vec<String>,
    pub timeout: Duration,
}

impl Default for NarrationConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            llm_path: DEFAULT_LLM_PATH.iter().map(|s| (*s).to_string()).collect(),
            timeout: Duration::from_millis(DEFAULT_TIMEOUT_MS),
        }
    }
}

impl NarrationConfig {
    /// Returns `true` when `tool_name` matches any entry in `llm_path`,
    /// where an entry of the form `prefix_*` matches by prefix.
    pub fn uses_llm_path(&self, tool_name: &str) -> bool {
        self.llm_path.iter().any(|pat| {
            if let Some(prefix) = pat.strip_suffix("_*") {
                tool_name.starts_with(prefix) && tool_name.len() > prefix.len()
            } else {
                pat == tool_name
            }
        })
    }
}

pub struct NarratorHook {
    config: NarrationConfig,
    events: Arc<SqliteEventStore>,
    /// `OnceLock` because the classifier is attached either at
    /// construction time (via `with_classifier`, for tests) or at
    /// boot time (via `attach_classifier` from `AppState::with_narrator_classifier`,
    /// for production). After it's set the value is read-only — no
    /// per-dispatch lock cost, no `RwLock` churn.
    classifier: OnceLock<ClassifierSlot>,
}

impl NarratorHook {
    pub fn new(events: Arc<SqliteEventStore>) -> Self {
        Self {
            config: NarrationConfig::default(),
            events,
            classifier: OnceLock::new(),
        }
    }

    pub fn with_config(mut self, config: NarrationConfig) -> Self {
        self.config = config;
        self
    }

    /// Construction-time classifier attachment (consumed `self`).
    /// Used by tests + any caller that knows the classifier at
    /// instantiation. Silently no-ops if a classifier is already
    /// present (matches `OnceLock` semantics).
    pub fn with_classifier(
        self,
        llm: Arc<LlmClient>,
        slot_alias: impl Into<String>,
        system_prompt: Arc<String>,
    ) -> Self {
        let _ = self.classifier.set(ClassifierSlot {
            llm,
            model: slot_alias.into(),
            system_prompt,
        });
        self
    }

    /// Story 2.20: runtime classifier attachment via `&self`. Called by
    /// `AppState::with_narrator_classifier` after the templated-only
    /// `NarratorHook` is already inside the dispatcher's hook chain.
    /// Returns `Err(slot)` if a classifier was already attached (set-once
    /// invariant); production callers ignore the Err since the hook is
    /// attached exactly once at boot.
    pub fn attach_classifier(
        &self,
        llm: Arc<LlmClient>,
        slot_alias: impl Into<String>,
        system_prompt: Arc<String>,
    ) -> Result<(), ClassifierSlot> {
        self.classifier.set(ClassifierSlot {
            llm,
            model: slot_alias.into(),
            system_prompt,
        })
    }

    /// Test-only / read-only accessor for the classifier state.
    pub fn classifier_is_attached(&self) -> bool {
        self.classifier.get().is_some()
    }

    async fn emit_narration(&self, ctx: &ToolContext, call_id: &str, text: &str) {
        let event = NewEvent {
            session_id: ctx.session_id.clone(),
            event_type: EventType::Message,
            source: "hook:narrator".into(),
            data: json!({
                "role": "assistant",
                "content": text,
                "ui": NARRATE_UI_TAG,
                "call_id": call_id,
            }),
        };
        if let Err(error) = self.events.append(event).await {
            tracing::warn!(%error, "narration emit failed");
        }
    }

    async fn emit_skipped(&self, ctx: &ToolContext, tool: &str, reason: &str) {
        let event = NewEvent {
            session_id: ctx.session_id.clone(),
            event_type: EventType::Misc,
            source: "hook:narrator".into(),
            data: json!({
                "kind": "narration_skipped",
                "tool": tool,
                "reason": reason,
            }),
        };
        if let Err(error) = self.events.append(event).await {
            tracing::warn!(%error, "narration_skipped emit failed");
        }
    }
}

#[async_trait]
impl Hook for NarratorHook {
    async fn pre(&self, call_id: &str, name: &str, args: &Value, ctx: &ToolContext) {
        if !self.config.enabled {
            return;
        }
        let text = if self.config.uses_llm_path(name) {
            match self.classifier.get() {
                Some(slot) => match slot.classify(name, args, self.config.timeout).await {
                    Ok(text) => text,
                    Err(reason) => {
                        self.emit_skipped(ctx, name, &reason).await;
                        return;
                    }
                },
                // No classifier wired (yet — e.g., before
                // `AppState::with_narrator_classifier` runs, or test
                // setup that never attaches one). Fall through to the
                // generic templated sentence rather than silently
                // dropping the narration.
                None => templates::template_for(name, args),
            }
        } else {
            templates::template_for(name, args)
        };
        self.emit_narration(ctx, call_id, &text).await;
    }

    async fn post(&self, _: &str, _: &str, _: &Value, _: &ToolOutput, _: &ToolContext) {}

    async fn failure(&self, _: &str, _: &str, _: &Value, _: &ToolError, _: &ToolContext) {}
}
