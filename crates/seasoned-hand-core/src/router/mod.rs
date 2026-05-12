//! 12-slot model router.
//! Each slot resolves to (provider, model, base_url, api_key).
//! refs: /specs/01-architecture/ARCHITECTURE.md §3
//! refs: /specs/01-architecture/decisions/ADR-003-12-slot-model-routing.md

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub mod capability;

/// Bifrost default (used when neither the main slot nor the slot itself
/// supplies a base_url).
const DEFAULT_BIFROST_BASE_URL: &str = "http://localhost:4000/v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SlotName {
    Main,
    Planner,
    Verifier,
    Vision,
    WebExtract,
    Screenshot,
    Compression,
    SessionTitle,
    SessionSearch,
    Classifier,
    Embedding,
    Reasoning,
}

impl SlotName {
    /// All 12 slots in canonical order.
    pub const ALL: &'static [SlotName] = &[
        SlotName::Main,
        SlotName::Planner,
        SlotName::Verifier,
        SlotName::Vision,
        SlotName::WebExtract,
        SlotName::Screenshot,
        SlotName::Compression,
        SlotName::SessionTitle,
        SlotName::SessionSearch,
        SlotName::Classifier,
        SlotName::Embedding,
        SlotName::Reasoning,
    ];
}

#[derive(Debug, Clone, Deserialize)]
pub struct SlotConfig {
    pub provider: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub api_key_env: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RouterConfig {
    pub slots: HashMap<SlotName, SlotConfig>,
}

#[derive(Debug, Clone)]
pub struct ResolvedSlot {
    pub provider: String,
    pub model: String,
    pub base_url: String,
    pub api_key: Option<String>,
}

#[derive(Debug, Error)]
pub enum RouterError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("yaml parse error: {0}")]
    Yaml(#[from] serde_yaml::Error),
    #[error("config missing required 'main' slot")]
    MissingMain,
    #[error(
        "auxiliary slot {slot:?} uses provider 'auto'/'main' but has no model and main has none either"
    )]
    UnresolvableInheritance { slot: SlotName },
    // ---- Story 1.7: Bifrost alias resolution (capability submodule) ----
    #[error("slot {0:?} not configured for capability resolution")]
    SlotNotConfigured(SlotName),
    #[error("slot {slot:?} alias {alias} not found at Bifrost (HTTP {status})")]
    AliasNotFound {
        slot: SlotName,
        alias: String,
        status: reqwest::StatusCode,
    },
    #[error("slot {slot:?} resolution: {message}")]
    Resolution { slot: SlotName, message: String },
    #[error("main slot unavailable: {0}")]
    MainSlotUnavailable(Box<RouterError>),
}

#[derive(Debug)]
pub struct SlotRouter {
    resolved: HashMap<SlotName, ResolvedSlot>,
    /// Returned by `resolve` if a slot is missing from `resolved`. Both
    /// `from_config` and `default_for_bifrost` populate every SlotName::ALL,
    /// so this should be unreachable — keeping it lets `resolve` stay
    /// panic-free per AGENTS.md §7.
    main_fallback: ResolvedSlot,
    /// Story 1.7: optional Bifrost alias resolver. Populated by
    /// [`SlotRouter::with_resolver`] at server startup; consumed by story
    /// 1.8 (`verifier ≠ main` gate) and the runtime for resolved-provider
    /// logging. None when the server boots without a real Bifrost
    /// (default-for-bifrost ergonomics).
    resolver: Option<Arc<capability::Resolver>>,
    /// Story 1.7: capability-side resolutions keyed by slot. Empty when no
    /// resolver is attached; populated by [`SlotRouter::with_capability_resolutions`].
    capability_resolved: HashMap<SlotName, capability::ResolvedSlot>,
}

impl SlotRouter {
    pub fn from_config(cfg: RouterConfig) -> Result<Self, RouterError> {
        let main = cfg
            .slots
            .get(&SlotName::Main)
            .ok_or(RouterError::MissingMain)?
            .clone();

        let main_resolved = resolve_concrete(SlotName::Main, &main, None)?;
        let mut resolved = HashMap::new();
        resolved.insert(SlotName::Main, main_resolved.clone());

        for slot in SlotName::ALL.iter().copied() {
            if slot == SlotName::Main {
                continue;
            }
            match cfg.slots.get(&slot) {
                None => {
                    // unset slot → inherit main
                    resolved.insert(slot, main_resolved.clone());
                }
                Some(slot_cfg) => {
                    let r = if slot_cfg.provider == "auto" || slot_cfg.provider == "main" {
                        // inherit from main; allow per-slot overrides for model/base_url
                        let mut inherited = main_resolved.clone();
                        if let Some(m) = &slot_cfg.model {
                            inherited.model = m.clone();
                        }
                        if let Some(u) = &slot_cfg.base_url {
                            inherited.base_url = u.clone();
                        }
                        inherited
                    } else {
                        resolve_concrete(slot, slot_cfg, Some(&main_resolved))?
                    };
                    resolved.insert(slot, r);
                }
            }
        }

        Ok(Self {
            resolved,
            main_fallback: main_resolved,
            resolver: None,
            capability_resolved: HashMap::new(),
        })
    }

    pub fn from_yaml_str(yaml: &str) -> Result<Self, RouterError> {
        let cfg: RouterConfig = serde_yaml::from_str(yaml)?;
        Self::from_config(cfg)
    }

    pub fn from_yaml(path: impl AsRef<Path>) -> Result<Self, RouterError> {
        let s = std::fs::read_to_string(path)?;
        Self::from_yaml_str(&s)
    }

    /// Minimal default: `main` slot pointing at Bifrost's `agent-primary`
    /// alias. Used when no config file is present (Phase 0 ergonomics).
    pub fn default_for_bifrost() -> Self {
        let main = ResolvedSlot {
            provider: "bifrost".into(),
            model: "agent-primary".into(),
            base_url: DEFAULT_BIFROST_BASE_URL.into(),
            api_key: None,
        };
        let mut resolved = HashMap::new();
        for slot in SlotName::ALL {
            resolved.insert(*slot, main.clone());
        }
        Self {
            resolved,
            main_fallback: main,
            resolver: None,
            capability_resolved: HashMap::new(),
        }
    }

    pub fn resolve(&self, slot: SlotName) -> &ResolvedSlot {
        self.resolved.get(&slot).unwrap_or(&self.main_fallback)
    }

    /// Story 1.7: Phase-0 `resolve(Main)` alias; kept around so story 1.8
    /// and other Phase 1 callers can reach for "the main slot routing
    /// target" without re-typing `SlotName::Main`.
    pub fn resolve_main(&self) -> &ResolvedSlot {
        self.resolve(SlotName::Main)
    }

    /// Story 1.7: attach a capability resolver + its [`capability::ResolveAllReport`]
    /// to the router. Server startup calls this after a successful
    /// `Resolver::resolve_all_or_main()`.
    pub fn with_resolver(
        mut self,
        resolver: Arc<capability::Resolver>,
        report: capability::ResolveAllReport,
    ) -> Self {
        self.resolver = Some(resolver);
        self.capability_resolved = report.resolved;
        self
    }

    /// Story 1.7: expose the resolver so story 1.8 can re-resolve at
    /// runtime if a slot's alias changes.
    pub fn resolver(&self) -> Option<&Arc<capability::Resolver>> {
        self.resolver.as_ref()
    }

    /// Story 1.7: returns the capability-side resolution for a slot if it
    /// was successfully resolved at startup. `None` when the slot was
    /// recorded as `unavailable` (non-main slots only) or no resolver was
    /// attached.
    pub fn resolve_optional(&self, slot: SlotName) -> Option<&capability::ResolvedSlot> {
        self.capability_resolved.get(&slot)
    }
}

fn resolve_concrete(
    slot: SlotName,
    cfg: &SlotConfig,
    main: Option<&ResolvedSlot>,
) -> Result<ResolvedSlot, RouterError> {
    let model = cfg
        .model
        .clone()
        .or_else(|| main.map(|m| m.model.clone()))
        .ok_or(RouterError::UnresolvableInheritance { slot })?;
    let base_url = cfg
        .base_url
        .clone()
        .or_else(|| main.map(|m| m.base_url.clone()))
        .unwrap_or_else(|| DEFAULT_BIFROST_BASE_URL.to_string());
    let api_key = cfg
        .api_key_env
        .as_ref()
        .and_then(|env| std::env::var(env).ok().filter(|s| !s.is_empty()));
    Ok(ResolvedSlot {
        provider: cfg.provider.clone(),
        model,
        base_url,
        api_key,
    })
}

#[cfg(test)]
mod tests;
