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
fn default_for_bifrost_resolves_all_slots() {
    let r = SlotRouter::default_for_bifrost();
    for slot in SlotName::ALL {
        let res = r.resolve(*slot);
        assert_eq!(res.model, "agent-primary");
        assert_eq!(res.base_url, "http://localhost:4000/v1");
    }
}
