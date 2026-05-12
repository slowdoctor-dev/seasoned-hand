use std::collections::HashMap;

use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;
use crate::router::{RouterError, SlotName};

#[test]
fn resolver_returns_capabilities_for_claude_sonnet_4_6() {
    let caps = capabilities_for("claude-sonnet-4-6");
    assert_eq!(caps.tool_calling, Some(true));
    assert_eq!(caps.json_mode, Some(true));
    assert_eq!(caps.vision, Some(true));
}

#[test]
fn resolver_returns_unknown_for_unrecognised_model() {
    let caps = capabilities_for("definitely-not-a-real-model-2099");
    assert_eq!(caps.tool_calling, None);
    assert_eq!(caps.json_mode, None);
    assert_eq!(caps.vision, None);
    assert_eq!(caps, CapabilityFlags::unknown());
}

#[test]
fn table_covers_minimum_required_ids() {
    // Spec acceptance criterion: table covers Claude 4.x, GPT-5.x, and
    // llama3.2:3b. Guard the matrix with an assertion-per-id.
    for id in [
        "claude-sonnet-4-6",
        "claude-opus-4-7",
        "claude-haiku-4-5",
        "gpt-5.1",
        "gpt-5.3-codex",
        "llama3.2:3b",
    ] {
        let caps = capabilities_for(id);
        assert_ne!(
            caps,
            CapabilityFlags::unknown(),
            "model {id} should have a capability entry"
        );
        assert_eq!(
            caps.tool_calling,
            Some(true),
            "model {id} should advertise tool_calling"
        );
    }
}

#[tokio::test]
async fn resolve_slot_against_bifrost_mock() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models/agent-primary"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "claude-sonnet-4-6",
            "object": "model"
        })))
        .mount(&server)
        .await;

    let mut aliases = HashMap::new();
    aliases.insert(SlotName::Main, "agent-primary".to_string());
    let resolver = Resolver::new(format!("{}/v1", server.uri()), aliases);

    let resolved = resolver
        .resolve_slot(SlotName::Main)
        .await
        .expect("resolve main");
    assert_eq!(resolved.slot, SlotName::Main);
    assert_eq!(resolved.alias, "agent-primary");
    assert_eq!(resolved.provider_model_id, "claude-sonnet-4-6");
    assert_eq!(resolved.capabilities.tool_calling, Some(true));
}

#[tokio::test]
async fn non_main_slot_unavailable_is_warning_not_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models/agent-primary"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "claude-sonnet-4-6"
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/models/verifier-broken-alias"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let mut aliases = HashMap::new();
    aliases.insert(SlotName::Main, "agent-primary".to_string());
    aliases.insert(SlotName::Verifier, "verifier-broken-alias".to_string());
    let resolver = Resolver::new(format!("{}/v1", server.uri()), aliases);

    let report = resolver
        .resolve_all_or_main()
        .await
        .expect("resolve_all_or_main should NOT error on non-main 404");
    assert!(report.resolved.contains_key(&SlotName::Main));
    assert!(report.unavailable.contains(&SlotName::Verifier));
    assert!(!report.resolved.contains_key(&SlotName::Verifier));
}

#[tokio::test]
async fn main_slot_unresolvable_is_startup_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models/agent-primary"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let mut aliases = HashMap::new();
    aliases.insert(SlotName::Main, "agent-primary".to_string());
    let resolver = Resolver::new(format!("{}/v1", server.uri()), aliases);

    let err = resolver
        .resolve_all_or_main()
        .await
        .expect_err("main 404 must surface as a startup error");
    match err {
        RouterError::MainSlotUnavailable(inner) => match *inner {
            RouterError::AliasNotFound {
                slot, ref alias, ..
            } => {
                assert_eq!(slot, SlotName::Main);
                assert_eq!(alias, "agent-primary");
            }
            other => panic!("expected AliasNotFound, got {other:?}"),
        },
        other => panic!("expected MainSlotUnavailable, got {other:?}"),
    }
}
