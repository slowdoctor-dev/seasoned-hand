//! Model capability detection for startup checks.
//! refs: /specs/01-architecture/ARCHITECTURE.md §3, §4
//! refs: /specs/phase-0/architecture.md §4.4

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::llm::{LlmClient, LlmError};
use crate::router::{SlotName, SlotRouter};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Capability {
    ToolCalling,
    Vision,
    JsonMode,
    LongContext,
    Embedding,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModelCapabilities {
    pub model_id: String,
    pub capabilities: HashSet<Capability>,
}

impl ModelCapabilities {
    pub fn has(&self, capability: Capability) -> bool {
        self.capabilities.contains(&capability)
    }
}

#[derive(Debug, Error)]
pub enum CapabilityError {
    #[error("llm: {0}")]
    Llm(#[from] LlmError),
    #[error(
        "main slot model '{model}' does not support tool-calling — architecture §4 hard constraint"
    )]
    MainLacksToolCalling { model: String },
}

pub struct CapabilityProbe {
    client: LlmClient,
}

impl CapabilityProbe {
    pub fn new(client: LlmClient) -> Self {
        Self { client }
    }

    pub async fn probe_models(
        &self,
    ) -> Result<HashMap<String, ModelCapabilities>, CapabilityError> {
        let models = self.client.list_models().await?;
        let mut probed = HashMap::new();
        for model in models {
            probed.insert(model.id.clone(), built_in_capabilities(&model.id));
        }
        Ok(probed)
    }
}

pub fn assert_main_supports_tool_calling(
    router: &SlotRouter,
    probed: &HashMap<String, ModelCapabilities>,
) -> Result<(), CapabilityError> {
    let main = router.resolve(SlotName::Main);
    let caps = probed
        .get(&main.model)
        .cloned()
        .unwrap_or_else(|| built_in_capabilities(&main.model));

    if caps.has(Capability::ToolCalling) {
        Ok(())
    } else {
        Err(CapabilityError::MainLacksToolCalling {
            model: main.model.clone(),
        })
    }
}

pub fn warn_implied_slot_capability_mismatches(
    router: &SlotRouter,
    probed: &HashMap<String, ModelCapabilities>,
) {
    for slot in SlotName::ALL {
        let required = match slot {
            SlotName::Vision | SlotName::Screenshot => Some(Capability::Vision),
            SlotName::Embedding => Some(Capability::Embedding),
            _ => None,
        };
        let Some(required) = required else {
            continue;
        };
        let resolved = router.resolve(*slot);
        let caps = probed
            .get(&resolved.model)
            .cloned()
            .unwrap_or_else(|| built_in_capabilities(&resolved.model));
        if !caps.has(required) {
            tracing::warn!(
                ?slot,
                model = %resolved.model,
                ?required,
                "slot model may not support implied capability"
            );
        }
    }
}

pub fn built_in_capabilities(model_id: &str) -> ModelCapabilities {
    let mut capabilities = HashSet::new();
    let id = model_id.to_lowercase();

    if id == "agent-primary" || id == "agent-fallback" || id.starts_with("claude-") {
        capabilities.extend([
            Capability::ToolCalling,
            Capability::Vision,
            Capability::LongContext,
        ]);
    } else if id.starts_with("gpt-4") {
        capabilities.extend([
            Capability::ToolCalling,
            Capability::Vision,
            Capability::JsonMode,
            Capability::LongContext,
        ]);
    } else if id.starts_with("qwen") || id.starts_with("llama3.1") {
        capabilities.insert(Capability::ToolCalling);
    } else if let Some(size) = id.strip_prefix("llama3.2:").and_then(parse_size_b)
        && size >= 8
    {
        capabilities.insert(Capability::ToolCalling);
    } else if id.starts_with("text-embedding-") {
        capabilities.insert(Capability::Embedding);
    }

    ModelCapabilities {
        model_id: model_id.to_string(),
        capabilities,
    }
}

fn parse_size_b(value: &str) -> Option<u32> {
    let size = value
        .split(['-', '_'])
        .next()
        .unwrap_or(value)
        .strip_suffix('b')?;
    size.parse().ok()
}

#[cfg(test)]
mod tests;
