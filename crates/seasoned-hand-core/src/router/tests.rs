use super::*;

const FULL_YAML: &str = r#"
slots:
  main:
    provider: anthropic
    model: claude-sonnet-4-6
    base_url: http://localhost:4000/v1
    api_key_env: SH_TEST_MAIN_KEY
  planner:
    provider: auto
  verifier:
    provider: openai
    model: gpt-4o
    base_url: http://localhost:4000/v1
  vision:
    provider: auto
  web_extract:
    provider: auto
  screenshot:
    provider: auto
  compression:
    provider: auto
  session_title:
    provider: auto
  session_search:
    provider: auto
  classifier:
    provider: auto
  embedding:
    provider: openai
    model: text-embedding-3-small
    base_url: http://localhost:4000/v1
  reasoning:
    provider: main
"#;

#[test]
fn parse_full_yaml_config() {
    let r = SlotRouter::from_yaml_str(FULL_YAML).expect("parse");
    let main = r.resolve(SlotName::Main);
    assert_eq!(main.model, "claude-sonnet-4-6");
    assert_eq!(main.base_url, "http://localhost:4000/v1");
    let verifier = r.resolve(SlotName::Verifier);
    assert_eq!(verifier.model, "gpt-4o");
    assert_eq!(verifier.provider, "openai");
    let embedding = r.resolve(SlotName::Embedding);
    assert_eq!(embedding.model, "text-embedding-3-small");
}

#[test]
fn parse_minimal_only_main() {
    let yaml = r#"
slots:
  main:
    provider: anthropic
    model: claude-sonnet-4-6
"#;
    let r = SlotRouter::from_yaml_str(yaml).expect("parse minimal");
    // unset slots inherit main
    let planner = r.resolve(SlotName::Planner);
    assert_eq!(planner.model, "claude-sonnet-4-6");
    assert_eq!(planner.provider, "anthropic");
}

#[test]
fn missing_main_errors() {
    let yaml = r#"
slots:
  planner:
    provider: openai
    model: gpt-4o
"#;
    let err = SlotRouter::from_yaml_str(yaml).unwrap_err();
    matches!(err, RouterError::MissingMain);
}

#[test]
fn auto_inherits_from_main() {
    let r = SlotRouter::from_yaml_str(FULL_YAML).unwrap();
    let main = r.resolve(SlotName::Main).clone();
    let planner = r.resolve(SlotName::Planner);
    assert_eq!(planner.model, main.model);
    assert_eq!(planner.provider, main.provider);
}

#[test]
fn main_provider_inherits_like_auto() {
    let r = SlotRouter::from_yaml_str(FULL_YAML).unwrap();
    let main = r.resolve(SlotName::Main).clone();
    let reasoning = r.resolve(SlotName::Reasoning);
    assert_eq!(reasoning.model, main.model);
}

#[test]
fn base_url_override_wins() {
    let yaml = r#"
slots:
  main:
    provider: anthropic
    model: claude-sonnet-4-6
    base_url: http://main.example/v1
  verifier:
    provider: openai
    model: gpt-4o
    base_url: http://verifier.example/v1
"#;
    let r = SlotRouter::from_yaml_str(yaml).unwrap();
    let verifier = r.resolve(SlotName::Verifier);
    assert_eq!(verifier.base_url, "http://verifier.example/v1");
}

#[test]
fn api_key_env_resolves_from_env() {
    // SAFETY: tests live in single-threaded test runner only insofar as
    // tokio::test allocates a runtime; this is a pure sync test using
    // std::env which is process-global. We pick a unique env name to
    // avoid cross-test interference.
    unsafe {
        std::env::set_var("SH_TEST_KEY_XYZ", "secret");
    }
    let yaml = r#"
slots:
  main:
    provider: anthropic
    model: claude-sonnet-4-6
    api_key_env: SH_TEST_KEY_XYZ
"#;
    let r = SlotRouter::from_yaml_str(yaml).unwrap();
    let main = r.resolve(SlotName::Main);
    assert_eq!(main.api_key.as_deref(), Some("secret"));
    unsafe {
        std::env::remove_var("SH_TEST_KEY_XYZ");
    }
}

#[test]
fn from_yaml_reads_file() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("slots.yaml");
    std::fs::write(&path, FULL_YAML).unwrap();
    let r = SlotRouter::from_yaml(&path).unwrap();
    assert_eq!(r.resolve(SlotName::Main).model, "claude-sonnet-4-6");
}

#[test]
fn checked_in_example_presets_parse() {
    // Keep the repo's example slot configs loadable — they are the documented
    // "copy to config/slots.yaml and go" path.
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for name in ["config/slots.example.yaml", "config/slots.cn.example.yaml"] {
        let r = SlotRouter::from_yaml(root.join(name))
            .unwrap_or_else(|e| panic!("{name} failed to load: {e}"));
        for slot in SlotName::ALL.iter().copied() {
            let resolved = r.resolve(slot);
            assert!(!resolved.model.is_empty(), "{name}: {slot:?} has no model");
            assert!(
                !resolved.base_url.is_empty(),
                "{name}: {slot:?} has no base_url"
            );
        }
    }
    // The cn preset must keep verifier ≠ main (story 1.8 startup gate).
    let cn = SlotRouter::from_yaml(root.join("config/slots.cn.example.yaml")).unwrap();
    assert_ne!(
        cn.resolve(SlotName::Verifier).model,
        cn.resolve(SlotName::Main).model
    );
}

#[test]
fn default_for_bifrost_resolves_all_slots() {
    let r = SlotRouter::default_for_bifrost();
    for slot in SlotName::ALL {
        let res = r.resolve(*slot);
        assert_eq!(res.model, "agent-primary");
        assert_eq!(res.base_url, "http://localhost:4000/v1");
    }
}

// ============================================================================
// Story 1.8 — verifier ≠ main resolved-model-ID startup gate.
// Tests build a synthetic `capability::ResolveAllReport` (skipping the real
// Bifrost HTTP path) and exercise `SlotRouter::with_resolver`'s gate.
// ============================================================================

use crate::router::capability::{CapabilityFlags, ResolveAllReport, Resolver};
use std::collections::HashMap;
use std::sync::Arc;

fn resolved_for(slot: SlotName, alias: &str, provider_model_id: &str) -> capability::ResolvedSlot {
    capability::ResolvedSlot {
        slot,
        alias: alias.to_string(),
        provider_model_id: provider_model_id.to_string(),
        capabilities: CapabilityFlags::unknown(),
    }
}

fn build_resolver_with_aliases(aliases: &[(SlotName, &str)]) -> Arc<Resolver> {
    let mut map = HashMap::new();
    for (slot, alias) in aliases {
        map.insert(*slot, alias.to_string());
    }
    Arc::new(Resolver::new("http://unused.example/v1", map))
}

#[test]
fn verifier_gate_passes_when_models_differ() {
    let router = SlotRouter::default_for_bifrost();
    let resolver = build_resolver_with_aliases(&[
        (SlotName::Main, "agent-primary"),
        (SlotName::Verifier, "verifier-primary"),
    ]);
    let mut report = ResolveAllReport::default();
    report.resolved.insert(
        SlotName::Main,
        resolved_for(SlotName::Main, "agent-primary", "claude-sonnet-4-6"),
    );
    report.resolved.insert(
        SlotName::Verifier,
        resolved_for(SlotName::Verifier, "verifier-primary", "gpt-5.1"),
    );
    let router = router
        .with_resolver(resolver, report)
        .expect("differ → gate passes");
    assert!(router.verifier_enabled());
    let v = router
        .resolve_optional(SlotName::Verifier)
        .expect("verifier resolved");
    assert_eq!(v.provider_model_id, "gpt-5.1");
}

#[test]
fn verifier_gate_fails_when_models_equal() {
    let router = SlotRouter::default_for_bifrost();
    let resolver = build_resolver_with_aliases(&[
        (SlotName::Main, "agent-primary"),
        (SlotName::Verifier, "verifier-primary"),
    ]);
    let mut report = ResolveAllReport::default();
    report.resolved.insert(
        SlotName::Main,
        resolved_for(SlotName::Main, "agent-primary", "claude-sonnet-4-6"),
    );
    report.resolved.insert(
        SlotName::Verifier,
        resolved_for(SlotName::Verifier, "verifier-primary", "claude-sonnet-4-6"),
    );
    let err = router
        .with_resolver(resolver, report)
        .expect_err("same model id → gate fails");
    match err {
        RouterError::VerifierSameAsMain { model_id } => {
            assert_eq!(model_id, "claude-sonnet-4-6");
        }
        other => panic!("expected VerifierSameAsMain, got {other:?}"),
    }
}

#[test]
fn verifier_gate_fails_when_aliases_differ_but_models_equal() {
    let router = SlotRouter::default_for_bifrost();
    // agent-primary and agent-fallback are distinct aliases, but Bifrost
    // resolves both to the same upstream model — gate must still fail.
    let resolver = build_resolver_with_aliases(&[
        (SlotName::Main, "agent-primary"),
        (SlotName::Verifier, "agent-fallback"),
    ]);
    let mut report = ResolveAllReport::default();
    report.resolved.insert(
        SlotName::Main,
        resolved_for(SlotName::Main, "agent-primary", "claude-sonnet-4-6"),
    );
    report.resolved.insert(
        SlotName::Verifier,
        resolved_for(SlotName::Verifier, "agent-fallback", "claude-sonnet-4-6"),
    );
    let err = router
        .with_resolver(resolver, report)
        .expect_err("alias-distinct but model-same → gate fails");
    assert!(matches!(err, RouterError::VerifierSameAsMain { .. }));
}

#[test]
fn verifier_gate_skipped_when_verifier_not_configured() {
    let router = SlotRouter::default_for_bifrost();
    let resolver = build_resolver_with_aliases(&[(SlotName::Main, "agent-primary")]);
    let mut report = ResolveAllReport::default();
    report.resolved.insert(
        SlotName::Main,
        resolved_for(SlotName::Main, "agent-primary", "claude-sonnet-4-6"),
    );
    let router = router
        .with_resolver(resolver, report)
        .expect("verifier absent → build succeeds");
    assert!(
        !router.verifier_enabled(),
        "no verifier configured ⇒ verifier_enabled stays false"
    );
    assert!(router.resolve_optional(SlotName::Verifier).is_none());
}

#[test]
fn verifier_gate_error_message_names_both_models() {
    let err = RouterError::VerifierSameAsMain {
        model_id: "claude-sonnet-4-6".to_string(),
    };
    let s = err.to_string();
    assert!(s.contains("claude-sonnet-4-6"), "msg was: {s}");
    assert!(s.contains("verifier"), "msg was: {s}");
    assert!(s.contains("main"), "msg was: {s}");
}
