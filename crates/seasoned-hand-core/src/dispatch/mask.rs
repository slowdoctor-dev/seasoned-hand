use crate::llm::ToolSpec;

pub const UNAVAILABLE_PREFIX: &str = "[UNAVAILABLE in current iteration] ";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentMode {
    Initializer,
    Worker,
    Verifier,
}

#[derive(Debug, Clone)]
pub struct MaskContext {
    pub session_id: String,
    pub iteration: u32,
    pub mode: AgentMode,
}

impl Default for MaskContext {
    fn default() -> Self {
        Self {
            session_id: String::new(),
            iteration: 0,
            mode: AgentMode::Worker,
        }
    }
}

pub trait ToolMaskPolicy: Send + Sync {
    fn is_available(&self, tool_name: &str, ctx: &MaskContext) -> bool;
}

pub struct DefaultMaskPolicy;

impl ToolMaskPolicy for DefaultMaskPolicy {
    fn is_available(&self, tool_name: &str, ctx: &MaskContext) -> bool {
        !matches!(
            (tool_name, ctx.mode),
            ("plan_create", AgentMode::Worker | AgentMode::Verifier) | ("checkpoint_rollback", _)
        )
    }
}

pub fn apply_mask(specs: &mut [ToolSpec], policy: &dyn ToolMaskPolicy, ctx: &MaskContext) {
    for spec in specs.iter_mut() {
        if !policy.is_available(&spec.function.name, ctx)
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

    struct ToggleMaskPolicy;
    impl ToolMaskPolicy for ToggleMaskPolicy {
        fn is_available(&self, tool_name: &str, ctx: &MaskContext) -> bool {
            if tool_name == "plan_create" {
                return (ctx.iteration & 1) == 0;
            }
            true
        }
    }

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
        let ctx = MaskContext {
            session_id: "s1".into(),
            iteration: 1,
            mode: AgentMode::Worker,
        };
        apply_mask(&mut specs, &DefaultMaskPolicy, &ctx);
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
    fn mask_does_not_change_order() {
        let baseline: Vec<(String, String)> = sample_specs()
            .iter()
            .map(|spec| {
                let hash = serde_json::to_string(&spec.function.parameters).expect("schema json");
                (spec.function.name.clone(), hash)
            })
            .collect();

        for iteration in [0_u32, 1_u32, 50_u32] {
            let mut specs = sample_specs();
            let ctx = MaskContext {
                session_id: "s1".into(),
                iteration,
                mode: AgentMode::Worker,
            };
            apply_mask(&mut specs, &ToggleMaskPolicy, &ctx);
            let current: Vec<(String, String)> = specs
                .iter()
                .map(|spec| {
                    let hash =
                        serde_json::to_string(&spec.function.parameters).expect("schema json");
                    (spec.function.name.clone(), hash)
                })
                .collect();
            assert_eq!(baseline, current);
        }
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
