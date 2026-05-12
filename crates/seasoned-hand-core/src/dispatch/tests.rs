use std::sync::Arc;

use serde_json::{Value, json};

use super::*;
use crate::db;
use crate::events::sqlite::SqliteEventStore;
use crate::events::{EventQuery, EventStore, EventType};
use crate::plan::PlanManager;
use crate::sandbox::SandboxClient;
use crate::search::{SearchClient, SearchProvider};
use crate::tools::{ToolContext, register_builtin_tools};

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
        events: store,
        sandbox,
        search,
        plan_manager,
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
        events: store.clone(),
        sandbox,
        search,
        plan_manager,
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
    assert_eq!(events[1].data["output"]["signal"], "task_complete");
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
