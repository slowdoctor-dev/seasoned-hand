# Story 0.11 — LLM client (OpenAI-compatible over Bifrost)

> **Status**: done
> **Estimated**: 2 hours
> **Dependencies**: story 0.1 (Bifrost up)
> **Phase**: 0
> **Type**: backend
> **Reads first**: `/specs/phase-0/architecture.md` §1 (LLM client), §4 (agent loop "one tool per iteration"), §4.4 (Bifrost interface), §5.1 (reqwest)

---

## Goal

Add the `seasoned-hand-core::llm` module: a thin, opinionated
OpenAI-compatible chat-completions client that talks to Bifrost at
`http://localhost:4000/v1`. The client supports tool-calling with
`tool_choice="required"` (architecture §4 HARD constraint — one tool per
iteration) and parses the standard OpenAI response into our internal
shape.

Story 0.11 only ships the client. Story 0.12 builds the 12-slot
resolver on top. Story 0.13 adds capability detection. Story 0.14 wires
the client into the agent loop.

## Acceptance criteria

- [ ] `seasoned-hand-core::llm` module with:
      - `LlmClient::new(base_url: String, api_key: Option<String>) -> Self`
      - `LlmClient::from_env() -> Self` reading `BIFROST_BASE_URL` (default
        `http://localhost:4000/v1`) and optionally `BIFROST_MASTER_KEY`
        (Phase 0 typically unused per architecture §9)
      - `chat_completion(request) -> Result<ChatCompletionResponse, LlmError>`
      - `list_models() -> Result<Vec<ModelInfo>, LlmError>` (story 0.13
        builds capability detection on top)
- [ ] Request types:
      - `ChatCompletionRequest { model, messages, tools?, tool_choice?, max_tokens?, temperature?, ... }`
      - `Message { role: "system"|"user"|"assistant"|"tool", content?, tool_calls?, tool_call_id? }`
      - `ToolSpec { type:"function", function: { name, description, parameters: JsonSchema } }`
      - `ToolChoice` enum: `Auto | None | Required | Specific { name }`
- [ ] Response types:
      - `ChatCompletionResponse { id, model, choices: Vec<Choice>, usage: Option<Usage> }`
      - `Choice { index, finish_reason, message: AssistantMessage }`
      - `AssistantMessage { content?, tool_calls?: Vec<ToolCall> }`
      - `ToolCall { id, type:"function", function: { name, arguments: String /* raw JSON */ } }`
      - `Usage { prompt_tokens, completion_tokens, total_tokens }`
- [ ] `ToolChoice::Required` serializes as `"required"` — verified
      against OpenAI tool-calling spec
- [ ] `LlmError` variants: `Http`, `JsonParse`, `Status { code, body }`,
      `MissingChoice`
- [ ] On HTTP 4xx/5xx, error includes the response body for debugging
- [ ] **Network non-default in tests**: a unit test uses `wiremock` to
      stand up a fake Bifrost; assert request shape and response parsing.
      Real-Bifrost tests are `#[ignore]`'d.
- [ ] `cargo clippy / fmt / test / spec-check` all pass

## Non-goals

- Slot routing (story 0.12)
- Capability auto-detection (story 0.13)
- Streaming (Phase 1+)
- Embeddings (auxiliary slot; not Phase 0)
- Vision / multimodal (auxiliary slot; not Phase 0)
- Retry / backoff logic (Phase 1 hardening — Bifrost itself does
  fallback per ADR-001)
- Cost accounting (story 0.16)

---

## Implementation steps

### 1. Module skeleton

```rust
// crates/seasoned-hand-core/src/llm/mod.rs
use reqwest::Client;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum LlmError {
    #[error("http: {0}")]
    Http(#[from] reqwest::Error),
    #[error("status {code}: {body}")]
    Status { code: u16, body: String },
    #[error("json parse: {0}")]
    JsonParse(#[from] serde_json::Error),
    #[error("response missing choices")]
    MissingChoice,
}

#[derive(Clone)]
pub struct LlmClient {
    http: Client,
    base_url: String,         // typically http://localhost:4000/v1
    api_key: Option<String>,
}

impl LlmClient {
    pub fn new(base_url: impl Into<String>, api_key: Option<String>) -> Self {
        Self { http: Client::new(), base_url: base_url.into(), api_key }
    }
    pub fn from_env() -> Self { ... }
    pub async fn chat_completion(&self, req: ChatCompletionRequest) -> Result<ChatCompletionResponse, LlmError> { ... }
    pub async fn list_models(&self) -> Result<Vec<ModelInfo>, LlmError> { ... }
}
```

### 2. Types — match OpenAI's wire format

Use serde rename + skip_serializing_if for optional fields. Tool-choice
deserializes from either a string (`"auto"`, `"none"`, `"required"`) OR
an object `{type:"function", function:{name:"X"}}` — needs a custom
serde implementation or untagged enum.

### 3. `wiremock` test

```rust
use wiremock::{MockServer, Mock, ResponseTemplate, matchers::{method, path, body_partial_json}};

#[tokio::test]
async fn chat_completion_with_required_tool_choice() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .and(body_partial_json(json!({"tool_choice": "required"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "x", "object":"chat.completion",
            "model":"agent-primary",
            "choices":[{"index":0,"finish_reason":"tool_calls","message":{
                "role":"assistant", "content":null,
                "tool_calls":[{"id":"c1","type":"function","function":{"name":"idle","arguments":"{}"}}]
            }}],
            "usage":{"prompt_tokens":10,"completion_tokens":3,"total_tokens":13}
        })))
        .mount(&mock).await;

    let client = LlmClient::new(format!("{}", mock.uri()), None);
    let resp = client.chat_completion(req).await.unwrap();
    assert_eq!(resp.choices[0].message.tool_calls.as_ref().unwrap().len(), 1);
}
```

### 4. `list_models` test

Same pattern: mock `GET /models` returning `{data: [...]}`, parse into
`Vec<ModelInfo>`.

### 5. Real Bifrost smoke (`#[ignore]`)

If `$BIFROST_BASE_URL` is reachable and `ANTHROPIC_API_KEY` is set,
fire a real call: `chat_completion({model:"agent-primary", messages:[{role:"user", content:"say hi in one word"}]})`,
assert response shape (not exact content).

---

## Files changed

- `crates/seasoned-hand-core/Cargo.toml` (add `wiremock` dev-dep)
- `crates/seasoned-hand-core/src/lib.rs` (`pub mod llm`)
- `crates/seasoned-hand-core/src/llm/mod.rs` (new)
- `crates/seasoned-hand-core/src/llm/types.rs` (new — request/response structs)
- `crates/seasoned-hand-core/src/llm/tests.rs` (new — wiremock + ignored real)

---

## Spec references

- `/specs/phase-0/architecture.md` §1, §4, §4.4
- `/specs/01-architecture/decisions/ADR-001-bifrost-llm-gateway.md`

---

## Commit message

```
feat(phase-0): story 0.11 - OpenAI-compatible LLM client over Bifrost

- seasoned-hand-core::llm with LlmClient (reqwest, no streaming) and
  full chat-completions + tools surface
- Request: ChatCompletionRequest, Message, ToolSpec, ToolChoice
  (Auto/None/Required/Specific — Required serializes "required" per
  architecture §4 hard constraint)
- Response: ChatCompletionResponse, Choice, AssistantMessage, ToolCall,
  Usage; tool_calls.arguments stays as raw JSON string (per OpenAI wire
  format) — caller parses
- LlmError: Http / Status (with body) / JsonParse / MissingChoice
- list_models() for story 0.13 capability detection
- Tests: wiremock for chat_completion + tool_choice=required, mocked
  models list; one #[ignore]'d real-Bifrost smoke
- cargo clippy / fmt / test / spec-check all pass

refs: /specs/phase-0/stories/story-0.11.md
```

---

## Notes for next story (0.12)

- `LlmClient::new(base_url, api_key)` builds a low-level client.
  Story 0.12 builds a `SlotRouter` over the same client where each slot
  resolves to a (provider, model, base_url) tuple. Multiple slots can
  share one `LlmClient` if they hit the same Bifrost.
- `list_models()` returns the merged model list across Bifrost's
  configured providers — story 0.13 calls it once at startup to
  populate the capability table.
