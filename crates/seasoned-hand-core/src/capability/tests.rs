use wiremock::MockServer;

use super::*;

fn has(model: &str, capability: Capability) -> bool {
    built_in_capabilities(model).has(capability)
}

#[test]
fn claude_sonnet_supports_tool_calling() {
    let caps = built_in_capabilities("claude-sonnet-4-6");
    assert!(caps.has(Capability::ToolCalling));
    assert!(caps.has(Capability::Vision));
    assert!(caps.has(Capability::LongContext));
}

#[test]
fn gpt_4o_supports_tool_calling_and_vision() {
    let caps = built_in_capabilities("gpt-4o");
    assert!(caps.has(Capability::ToolCalling));
    assert!(caps.has(Capability::Vision));
    assert!(caps.has(Capability::JsonMode));
    assert!(caps.has(Capability::LongContext));
}

#[test]
fn llama_3_2_3b_does_not_claim_tool_calling() {
    assert!(!has("llama3.2:3b", Capability::ToolCalling));
}

#[test]
fn unknown_model_has_empty_capabilities() {
    assert!(
        built_in_capabilities("mystery-model")
            .capabilities
            .is_empty()
    );
}

#[tokio::test]
async fn probe_returns_empty_when_models_endpoint_unreachable() {
    let client = LlmClient::new("http://127.0.0.1:9/v1", None);
    let probe = CapabilityProbe::new(client);
    assert!(matches!(
        probe.probe_models().await,
        Err(CapabilityError::Llm(_))
    ));
}

#[test]
fn assert_main_passes_with_claude_sonnet() {
    let router = SlotRouter::from_yaml_str(
        r#"
slots:
  main:
    provider: anthropic
    model: claude-sonnet-4-6
"#,
    )
    .unwrap();

    assert!(assert_main_supports_tool_calling(&router, &HashMap::new()).is_ok());
}

#[test]
fn assert_main_fails_with_llama_3_2_3b() {
    let router = SlotRouter::from_yaml_str(
        r#"
slots:
  main:
    provider: ollama
    model: llama3.2:3b
"#,
    )
    .unwrap();

    let err = assert_main_supports_tool_calling(&router, &HashMap::new()).unwrap_err();
    assert!(matches!(
        err,
        CapabilityError::MainLacksToolCalling { model } if model == "llama3.2:3b"
    ));
}

#[test]
fn default_agent_primary_alias_passes_tool_calling_gate() {
    let router = SlotRouter::default_for_bifrost();
    assert!(assert_main_supports_tool_calling(&router, &HashMap::new()).is_ok());
}

#[tokio::test]
async fn probe_models_merges_model_list_with_built_in_table() {
    let mock = MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("GET"))
        .and(wiremock::matchers::path("/models"))
        .respond_with(
            wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [
                    { "id": "gpt-4o" },
                    { "id": "text-embedding-3-small" }
                ]
            })),
        )
        .mount(&mock)
        .await;

    let probe = CapabilityProbe::new(LlmClient::new(mock.uri(), None));
    let probed = probe.probe_models().await.unwrap();

    assert!(probed["gpt-4o"].has(Capability::ToolCalling));
    assert!(probed["text-embedding-3-small"].has(Capability::Embedding));
}
