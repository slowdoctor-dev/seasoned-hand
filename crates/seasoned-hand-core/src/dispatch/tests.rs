use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};

use super::*;
use crate::db;
use crate::dispatch::mask::{AgentMode, MaskContext};
use crate::events::payload::EventPayloadBody;
use crate::events::sqlite::SqliteEventStore;
use crate::events::{EventQuery, EventStore, EventType};
use crate::plan::PlanManager;
use crate::sandbox::SandboxClient;
use crate::search::{SearchClient, SearchProvider};
use crate::tools::{Tool, ToolContext, ToolError, ToolOutput, register_builtin_tools};

async fn fixture() -> (ToolDispatcher, ToolContext) {
    let pool = db::open(":memory:").await.unwrap();
    pool.with_conn(|conn| {
        conn.execute(
            "INSERT INTO sessions (id, created_at, updated_at, state) \
             VALUES ('s1', 1, 1, 'RUNNING')",
            [],
        )
        .unwrap();
    })
    .await;
    let store = Arc::new(SqliteEventStore::new(pool.clone()));
    let plan_manager = Arc::new(PlanManager::new(pool, store.clone()));
    let sandbox = Arc::new(
        SandboxClient::new(
            "ghcr.io/agent-infra/sandbox:1.0.0.152",
            std::env::temp_dir(),
        )
        .unwrap(),
    );
    let search = Arc::new(SearchClient::new(SearchProvider::Brave { api_key: None }));
    let ctx = ToolContext {
        session_id: "s1".into(),
        mask_ctx: MaskContext {
            session_id: "s1".into(),
            iteration: 0,
            mode: AgentMode::Worker,
        },
        events: store,
        sandbox,
        search,
        plan_manager,
        checkpoint_labels: Arc::new(crate::checkpoint::CheckpointLabelBuffer::new()),
    };
    let dispatcher = ToolDispatcher::new(register_builtin_tools());
    (dispatcher, ctx)
}

#[tokio::test]
async fn dispatch_unknown_tool_returns_unknown_tool() {
    let (d, ctx) = fixture().await;
    let out = d.dispatch(&ctx, "no_such_tool", Value::Null).await;
    assert!(!out.ok);
    assert_eq!(out.error.unwrap().kind, "unknown_tool");
}

#[tokio::test]
async fn dispatch_real_tool_succeeds() {
    let (d, ctx) = fixture().await;
    let out = d.dispatch(&ctx, "idle", Value::Null).await;
    assert!(out.ok);
    assert_eq!(out.output["signal"], "task_complete");
}

#[tokio::test]
async fn dispatch_sandbox_tool_returns_not_ready_without_session_sandbox() {
    let (d, ctx) = fixture().await;
    let out = d.dispatch(&ctx, "file_read", json!({"path": "/x"})).await;
    assert!(!out.ok);
    // ToolError::Backend → wrapped in ToolOutput error.kind="backend"
    assert_eq!(out.error.as_ref().unwrap().kind, "backend");
    assert!(out.error.unwrap().message.contains("sandbox not ready"));
}

#[tokio::test]
async fn dispatch_search_returns_missing_api_key_when_unset() {
    let (d, ctx) = fixture().await;
    let out = d
        .dispatch(&ctx, "info_search_web", json!({"query": "hello"}))
        .await;
    assert!(!out.ok);
    assert_eq!(out.error.unwrap().kind, "missing_api_key");
}

// ===== EventEmittingHook tests (story 0.10) =====

async fn fixture_with_hook() -> (ToolDispatcher, ToolContext, Arc<SqliteEventStore>) {
    let pool = db::open(":memory:").await.unwrap();
    pool.with_conn(|conn| {
        conn.execute(
            "INSERT INTO sessions (id, created_at, updated_at, state) \
             VALUES ('s1', 1, 1, 'RUNNING')",
            [],
        )
        .unwrap();
    })
    .await;
    let store = Arc::new(SqliteEventStore::new(pool.clone()));
    let plan_manager = Arc::new(PlanManager::new(pool, store.clone()));
    let sandbox = Arc::new(
        SandboxClient::new(
            "ghcr.io/agent-infra/sandbox:1.0.0.152",
            std::env::temp_dir(),
        )
        .unwrap(),
    );
    let search = Arc::new(SearchClient::new(SearchProvider::Brave { api_key: None }));
    let ctx = ToolContext {
        session_id: "s1".into(),
        mask_ctx: MaskContext {
            session_id: "s1".into(),
            iteration: 0,
            mode: AgentMode::Worker,
        },
        events: store.clone(),
        sandbox,
        search,
        plan_manager,
        checkpoint_labels: Arc::new(crate::checkpoint::CheckpointLabelBuffer::new()),
    };
    let dispatcher = ToolDispatcher::new(register_builtin_tools())
        .with_hook(Arc::new(hooks::EventEmittingHook::new(store.clone())));
    (dispatcher, ctx, store)
}

#[tokio::test]
async fn hook_emits_action_then_observation_with_linked_call_id() {
    let (d, ctx, store) = fixture_with_hook().await;
    let _out = d.dispatch(&ctx, "idle", Value::Null).await;

    let events = store.query("s1", EventQuery::default()).await.unwrap();
    assert_eq!(events.len(), 2, "expected Action + Observation");
    assert_eq!(events[0].event_type, EventType::Action);
    assert_eq!(events[1].event_type, EventType::Observation);
    assert_eq!(events[0].source, "tool:idle");
    assert_eq!(events[1].source, "tool:idle");

    let action_call_id = events[0].data["call_id"].as_str().unwrap();
    let obs_call_id = events[1].data["call_id"].as_str().unwrap();
    assert_eq!(action_call_id, obs_call_id, "call_id must link pre→post");
    assert_eq!(events[1].data["ok"], true);
    let obs_body: EventPayloadBody =
        serde_json::from_value(events[1].data["body"].clone()).unwrap();
    let bytes = obs_body.body_bytes(&ctx.sandbox, "s1").await.unwrap();
    let output_value: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(output_value["signal"], "task_complete");
}

#[tokio::test]
async fn hook_records_failure_observation_for_unknown_session_sandbox_tool() {
    // file_read returns ToolError::Backend (sandbox not ready) because the
    // test fixture has no sandbox running. That hits the failure() path.
    let (d, ctx, store) = fixture_with_hook().await;
    let out = d.dispatch(&ctx, "file_read", json!({"path": "/x"})).await;
    assert!(!out.ok);

    let events = store.query("s1", EventQuery::default()).await.unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].event_type, EventType::Action);
    assert_eq!(events[1].event_type, EventType::Observation);
    assert_eq!(events[1].data["ok"], false);
    assert_eq!(events[1].data["error"]["kind"], "backend");
    assert!(
        events[1].data["error"]["message"]
            .as_str()
            .unwrap()
            .contains("sandbox not ready")
    );
}

#[tokio::test]
async fn hook_failure_does_not_break_dispatch() {
    // Use a real fixture, then poison the events store by replacing it with
    // one whose underlying DB connection has been dropped. We simulate this
    // by constructing a hook over a pool that's been replaced with an empty
    // in-memory DB lacking the sessions table — the hook's append will
    // fail with a foreign-key violation but the tool dispatch must still
    // return its normal output.
    let broken_pool = db::open(":memory:").await.unwrap();
    // intentionally drop the sessions row so append() will FK-fail
    broken_pool
        .with_conn(|conn| {
            conn.execute("DROP TABLE sessions", []).unwrap();
        })
        .await;
    let broken_store = Arc::new(SqliteEventStore::new(broken_pool));

    let (_, ctx, _store) = fixture_with_hook().await;
    let dispatcher = ToolDispatcher::new(register_builtin_tools())
        .with_hook(Arc::new(hooks::EventEmittingHook::new(broken_store)));
    let out = dispatcher.dispatch(&ctx, "idle", Value::Null).await;
    // Tool succeeded even though the hook's append failed.
    assert!(out.ok);
    assert_eq!(out.output["signal"], "task_complete");
}

struct LargeOutputTool;

#[async_trait]
impl Tool for LargeOutputTool {
    fn name(&self) -> &'static str {
        "test_large_output"
    }
    fn description(&self) -> &'static str {
        "test helper"
    }
    fn schema(&self) -> Value {
        json!({"type":"object"})
    }
    async fn invoke(&self, _args: Value, _ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        Ok(ToolOutput {
            ok: true,
            output: json!({"blob": "x".repeat(100 * 1024)}),
            file_ref: None,
            error: None,
        })
    }
}

#[tokio::test]
async fn large_payload_writes_to_eventfiles() {
    let (mut d, ctx, store) = fixture_with_hook().await;
    let tmp = tempfile::tempdir().unwrap();
    let workspace = tmp.path().join("s1");
    std::fs::create_dir_all(&workspace).unwrap();
    ctx.sandbox
        .insert_handle_for_test(crate::sandbox::SandboxHandle {
            session_id: "s1".into(),
            container_id: "test".into(),
            api_url: "http://127.0.0.1:1".into(),
            novnc_url: "http://127.0.0.1:2".into(),
            ttyd_url: "ws://127.0.0.1:3".into(),
            workspace_host_path: workspace,
        })
        .await;
    d.registry
        .insert("test_large_output", Arc::new(LargeOutputTool));

    let out = d.dispatch(&ctx, "test_large_output", Value::Null).await;
    assert!(out.ok);

    let events = store.query("s1", EventQuery::default()).await.unwrap();
    assert_eq!(events.len(), 2);
    let obs_body: EventPayloadBody =
        serde_json::from_value(events[1].data["body"].clone()).unwrap();
    let EventPayloadBody::FileRef { path, .. } = obs_body else {
        panic!("expected file ref for large payload");
    };
    assert!(path.starts_with("/workspace/.eventfiles/"));
}

#[test]
fn removal_of_inline_preview_path() {
    let needle = ["inline", "preview", "fallback"].join("_");
    let output = std::process::Command::new("rg")
        .args(["-n", &needle, "crates/seasoned-hand-core/src/"])
        .output()
        .unwrap();
    assert!(
        output.stdout.is_empty(),
        "expected no legacy inline preview fallback references"
    );
}

struct PlanCreateTool;

#[async_trait]
impl Tool for PlanCreateTool {
    fn name(&self) -> &'static str {
        "plan_create"
    }
    fn description(&self) -> &'static str {
        "create a plan"
    }
    fn schema(&self) -> Value {
        json!({"type":"object"})
    }
    async fn invoke(&self, _args: Value, _ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        Ok(ToolOutput {
            ok: true,
            output: json!({"ok": true}),
            file_ref: None,
            error: None,
        })
    }
}

#[tokio::test]
async fn dispatcher_rejects_masked_tool() {
    let (mut d, mut ctx, store) = fixture_with_hook().await;
    d.registry.insert("plan_create", Arc::new(PlanCreateTool));
    ctx.mask_ctx.mode = AgentMode::Worker;
    let out = d
        .dispatch(&ctx, "plan_create", json!({"goal":"x","phases":[]}))
        .await;
    assert!(!out.ok);
    assert_eq!(
        out.error.as_ref().map(|e| e.kind.as_str()),
        Some("tool_unavailable_in_iteration")
    );
    let events = store.query("s1", EventQuery::default()).await.unwrap();
    assert!(events.iter().any(|event| {
        event.event_type == EventType::Misc
            && event.data.get("kind").and_then(Value::as_str) == Some("tool_mask_violation")
            && event.data.get("tool").and_then(Value::as_str) == Some("plan_create")
    }));
}

#[tokio::test]
async fn initializer_can_still_call_plan_create_directly() {
    let (mut d, mut ctx, _store) = fixture_with_hook().await;
    d.registry.insert("plan_create", Arc::new(PlanCreateTool));
    ctx.mask_ctx.mode = AgentMode::Initializer;
    let out = d
        .dispatch(
            &ctx,
            "plan_create",
            json!({"goal":"x","phases":[{"id":1,"title":"t"}]}),
        )
        .await;
    assert!(out.ok);
}
