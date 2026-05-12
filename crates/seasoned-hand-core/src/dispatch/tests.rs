use std::sync::Arc;

use serde_json::{Value, json};

use super::*;
use crate::db;
use crate::events::sqlite::SqliteEventStore;
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
    let store = Arc::new(SqliteEventStore::new(pool));
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
