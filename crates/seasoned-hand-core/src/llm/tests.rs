use serde_json::json;
use wiremock::matchers::{body_partial_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;

fn mock_completion_body() -> serde_json::Value {
    json!({
        "id": "cmpl-1",
        "object": "chat.completion",
        "model": "agent-primary",
        "choices": [{
            "index": 0,
            "finish_reason": "tool_calls",
            "message": {
                "role": "assistant",
                "content": null,
                "tool_calls": [{
                    "id": "c1",
                    "type": "function",
                    "function": { "name": "idle", "arguments": "{}" }
                }]
            }
        }],
        "usage": { "prompt_tokens": 10, "completion_tokens": 3, "total_tokens": 13 }
    })
}

#[tokio::test]
async fn tool_choice_required_serializes_as_string_required() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .and(body_partial_json(json!({ "tool_choice": "required" })))
        .respond_with(ResponseTemplate::new(200).set_body_json(mock_completion_body()))
        .mount(&mock)
        .await;

    let client = LlmClient::new(mock.uri(), None);
    let req = ChatCompletionRequest {
        model: "agent-primary".into(),
        messages: vec![Message {
            role: Role::User,
            content: Some("hi".into()),
            name: None,
            tool_calls: None,
            tool_call_id: None,
        }],
        tools: Some(vec![ToolSpec::function(
            "idle",
            "signal completion",
            json!({"type":"object","properties":{}}),
        )]),
        tool_choice: Some(ToolChoice::required()),
        temperature: None,
        max_tokens: None,
        top_p: None,
    };
    let resp = client.chat_completion(req).await.unwrap();
    assert_eq!(resp.choices.len(), 1);
    let tcs = resp.choices[0].message.tool_calls.as_ref().unwrap();
    assert_eq!(tcs.len(), 1);
    assert_eq!(tcs[0].function.name, "idle");
    let usage = resp.usage.unwrap();
    assert_eq!(usage.total_tokens, 13);
}

#[tokio::test]
async fn status_error_includes_body() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(429).set_body_string("rate limit"))
        .mount(&mock)
        .await;

    let client = LlmClient::new(mock.uri(), None);
    let req = ChatCompletionRequest {
        model: "agent-primary".into(),
        messages: vec![],
        tools: None,
        tool_choice: None,
        temperature: None,
        max_tokens: None,
        top_p: None,
    };
    let err = client.chat_completion(req).await.unwrap_err();
    match err {
        LlmError::Status { code, body } => {
            assert_eq!(code, 429);
            assert!(body.contains("rate limit"));
        }
        e => panic!("expected Status, got {e:?}"),
    }
}

#[tokio::test]
async fn list_models_parses_data_array() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "object": "list",
            "data": [
                { "id": "agent-primary", "object": "model", "owned_by": "anthropic" },
                { "id": "agent-fallback" }
            ]
        })))
        .mount(&mock)
        .await;

    let client = LlmClient::new(mock.uri(), None);
    let models = client.list_models().await.unwrap();
    assert_eq!(models.len(), 2);
    assert_eq!(models[0].id, "agent-primary");
}

#[tokio::test]
async fn bearer_token_sent_when_api_key_present() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .and(wiremock::matchers::header("authorization", "Bearer secret"))
        .respond_with(ResponseTemplate::new(200).set_body_json(mock_completion_body()))
        .mount(&mock)
        .await;

    let client = LlmClient::new(mock.uri(), Some("secret".into()));
    let req = ChatCompletionRequest {
        model: "agent-primary".into(),
        messages: vec![],
        tools: None,
        tool_choice: None,
        temperature: None,
        max_tokens: None,
        top_p: None,
    };
    client.chat_completion(req).await.unwrap();
}

#[tokio::test]
#[ignore = "requires running Bifrost with real provider keys"]
async fn live_bifrost_chat_completion_smoke() {
    let client = LlmClient::from_env();
    let req = ChatCompletionRequest {
        model: std::env::var("BIFROST_TEST_MODEL").unwrap_or_else(|_| "agent-primary".into()),
        messages: vec![Message {
            role: Role::User,
            content: Some("Say hi in one word".into()),
            name: None,
            tool_calls: None,
            tool_call_id: None,
        }],
        tools: None,
        tool_choice: None,
        temperature: None,
        max_tokens: Some(8),
        top_p: None,
    };
    let resp = client.chat_completion(req).await.expect("live chat call");
    assert!(!resp.choices.is_empty());
}
