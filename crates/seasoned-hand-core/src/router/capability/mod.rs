//! Bifrost alias → provider model-ID resolver + capability flags.
//!
//! Replaces Phase 0's hardcoded "Bifrost cloud aliases support tool calling"
//! assumption (Phase 0 DEBT #22). At startup, for each slot's configured
//! alias we query Bifrost `GET /v1/models/<alias>` to learn the underlying
//! provider model id, then look up tool-calling / json-mode / vision flags
//! in the static [`table::capabilities_for`].
//!
//! Story 1.8 consumes `provider_model_id` to enforce `verifier ≠ main` at
//! startup.
//!
//! refs: /specs/phase-1/stories/story-1.7.md
//! refs: /specs/phase-1/architecture.md §4.4, §6
//! refs: /specs/phase-0/DEBT.md #22

use std::collections::HashMap;

use serde::Deserialize;

use super::{RouterError, SlotName};

pub mod table;

pub use table::capabilities_for;

/// Tri-state capability flags. `None` = unknown (model not in the table).
/// `Some(false)` is explicitly recorded support absence — never confuse
/// with unknown.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CapabilityFlags {
    pub tool_calling: Option<bool>,
    pub json_mode: Option<bool>,
    pub vision: Option<bool>,
}

impl CapabilityFlags {
    /// All flags `None` — used for model IDs not in the static table.
    pub const fn unknown() -> Self {
        Self {
            tool_calling: None,
            json_mode: None,
            vision: None,
        }
    }
}

/// Outcome of resolving one slot's alias against Bifrost.
#[derive(Debug, Clone)]
pub struct ResolvedSlot {
    pub slot: SlotName,
    pub alias: String,
    pub provider_model_id: String,
    pub capabilities: CapabilityFlags,
}

/// Outcome of [`Resolver::resolve_all_or_main`]. The `main` slot is
/// always present (otherwise the function would have returned `Err`).
/// Non-main slots that failed to resolve are listed in `unavailable`.
#[derive(Debug, Default)]
pub struct ResolveAllReport {
    pub resolved: HashMap<SlotName, ResolvedSlot>,
    pub unavailable: Vec<SlotName>,
}

/// Resolves slot aliases via Bifrost's OpenAI-compatible `/v1/models`
/// surface. Built once at server startup; consumed by [`crate::router::SlotRouter`]
/// (story 1.8) and the agent runtime for runtime model-ID-aware logging.
#[derive(Debug, Clone)]
pub struct Resolver {
    bifrost_base_url: String,
    http: reqwest::Client,
    slot_aliases: HashMap<SlotName, String>,
}

impl Resolver {
    /// `bifrost_base_url` is the same `base_url` value that the LLM client
    /// uses (e.g. `http://localhost:4000/v1`). Trailing `/v1` is tolerated;
    /// the `/v1/models/<alias>` path is appended canonically.
    pub fn new(
        bifrost_base_url: impl Into<String>,
        slot_aliases: HashMap<SlotName, String>,
    ) -> Self {
        Self {
            bifrost_base_url: bifrost_base_url.into(),
            http: reqwest::Client::new(),
            slot_aliases,
        }
    }

    pub fn slot_aliases(&self) -> &HashMap<SlotName, String> {
        &self.slot_aliases
    }

    /// Resolve one slot's alias against Bifrost. The capability flags come
    /// from the static [`table::capabilities_for`] keyed on the returned
    /// provider model id.
    pub async fn resolve_slot(&self, slot: SlotName) -> Result<ResolvedSlot, RouterError> {
        let alias = self
            .slot_aliases
            .get(&slot)
            .cloned()
            .ok_or(RouterError::SlotNotConfigured(slot))?;
        let url = format!(
            "{}/models/{alias}",
            self.bifrost_base_url.trim_end_matches('/')
        );
        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| RouterError::Resolution {
                slot,
                message: e.to_string(),
            })?;
        let status = resp.status();
        if !status.is_success() {
            return Err(RouterError::AliasNotFound {
                slot,
                alias,
                status,
            });
        }
        let info: ModelInfo = resp.json().await.map_err(|e| RouterError::Resolution {
            slot,
            message: e.to_string(),
        })?;
        let capabilities = table::capabilities_for(&info.id);
        Ok(ResolvedSlot {
            slot,
            alias,
            provider_model_id: info.id,
            capabilities,
        })
    }

    /// Resolve every configured slot. The `main` slot is hard-required —
    /// its failure becomes [`RouterError::MainSlotUnavailable`]. Any other
    /// slot whose resolution fails is logged at WARN and recorded in
    /// `unavailable` so startup can continue (architecture §6: "non-main
    /// slots that fail to resolve log a warning and are recorded as
    /// unavailable").
    pub async fn resolve_all_or_main(&self) -> Result<ResolveAllReport, RouterError> {
        let main = self
            .resolve_slot(SlotName::Main)
            .await
            .map_err(|e| RouterError::MainSlotUnavailable(Box::new(e)))?;
        let mut report = ResolveAllReport::default();
        report.resolved.insert(SlotName::Main, main);
        for slot in self.slot_aliases.keys().copied() {
            if slot == SlotName::Main {
                continue;
            }
            match self.resolve_slot(slot).await {
                Ok(r) => {
                    report.resolved.insert(slot, r);
                }
                Err(e) => {
                    tracing::warn!(?slot, error = %e, "slot alias unresolvable; marking unavailable");
                    report.unavailable.push(slot);
                }
            }
        }
        Ok(report)
    }
}

#[derive(Debug, Deserialize)]
struct ModelInfo {
    id: String,
}

#[cfg(test)]
mod tests;
