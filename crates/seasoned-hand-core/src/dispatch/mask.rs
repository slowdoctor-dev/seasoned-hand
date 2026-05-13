use crate::llm::ToolSpec;

pub const UNAVAILABLE_PREFIX: &str = "[UNAVAILABLE in current iteration] ";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentMode {
    Initializer,
    Worker,
    Verifier,
    /// Story 1.13b: trusted-internal caller (admin HTTP endpoint,
    /// Verifier-driven opt-in rollback handler). Tools that are
    /// LLM-masked (e.g. `checkpoint_rollback`) are AVAILABLE in this
    /// mode. Never set when an LLM is in the loop.
    Internal,
}

pub trait ToolMaskPolicy: Send + Sync {
    fn is_available(&self, tool_name: &str, mode: AgentMode) -> bool;
}

pub struct DefaultMaskPolicy;

impl ToolMaskPolicy for DefaultMaskPolicy {
    fn is_available(&self, tool_name: &str, mode: AgentMode) -> bool {
        match (tool_name, mode) {
            // `plan_create` is Initializer-only.
            ("plan_create", AgentMode::Worker | AgentMode::Verifier) => false,
            // Story 1.13b: `checkpoint_rollback` is masked from every
            // LLM-facing mode. The trusted-internal `Internal` mode
            // (admin endpoint + verifier-driven opt-in handler) can
            // dispatch it.
            ("checkpoint_rollback", AgentMode::Internal) => true,
            ("checkpoint_rollback", _) => false,
            _ => true,
        }
    }
}

pub fn apply_mask(specs: &mut [ToolSpec], policy: &dyn ToolMaskPolicy, mode: AgentMode) {
    for spec in specs.iter_mut() {
        if !policy.is_available(&spec.function.name, mode)
            && !spec.function.description.starts_with(UNAVAILABLE_PREFIX)
        {
            spec.function.description =
                format!("{UNAVAILABLE_PREFIX}{}", spec.function.description);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_specs() -> Vec<ToolSpec> {
        vec![
            ToolSpec::function("a_tool", "a", serde_json::json!({"type":"object"})),
            ToolSpec::function(
                "plan_create",
                "create plan",
                serde_json::json!({"type":"object"}),
            ),
            ToolSpec::function("z_tool", "z", serde_json::json!({"type":"object"})),
        ]
    }

    #[test]
    fn mask_descriptions_prefix_unavailable() {
        let mut specs = sample_specs();
        apply_mask(&mut specs, &DefaultMaskPolicy, AgentMode::Worker);
        let plan_create = specs
            .iter()
            .find(|spec| spec.function.name == "plan_create")
            .expect("plan_create spec");
        assert!(
            plan_create
                .function
                .description
                .starts_with(UNAVAILABLE_PREFIX)
        );
    }

    #[test]
    fn mask_blocks_checkpoint_rollback_from_every_llm_facing_mode() {
        // Story 1.13b regression: `checkpoint_rollback` must NEVER be
        // visible to an LLM. The trusted-internal `AgentMode::Internal`
        // (admin endpoint + verifier-driven opt-in handler) CAN
        // dispatch it. Pin both halves of the invariant against
        // accidental future change.
        let policy = DefaultMaskPolicy;
        for mode in [
            AgentMode::Initializer,
            AgentMode::Worker,
            AgentMode::Verifier,
        ] {
            assert!(
                !policy.is_available("checkpoint_rollback", mode),
                "checkpoint_rollback must be masked from AgentMode::{mode:?}"
            );
        }
        assert!(
            policy.is_available("checkpoint_rollback", AgentMode::Internal),
            "checkpoint_rollback must be AVAILABLE in AgentMode::Internal"
        );
    }

    #[test]
    fn tool_catalog_order_is_stable() {
        let baseline: Vec<String> = sample_specs()
            .iter()
            .map(|spec| spec.function.name.clone())
            .collect();
        for _ in 0..10 {
            let current: Vec<String> = sample_specs()
                .iter()
                .map(|spec| spec.function.name.clone())
                .collect();
            assert_eq!(baseline, current);
        }
    }
}
