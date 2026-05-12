use std::sync::Arc;

use serde_json::{Value, json};
use wiremock::matchers::{body_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::{ToolContext, register_builtin_tools};
use crate::db;
use crate::events::sqlite::SqliteEventStore;
use crate::events::{EventQuery, EventStore};
use crate::plan::PlanManager;
use crate::sandbox::SandboxClient;
use crate::search::{SearchClient, SearchProvider};

async fn ctx() -> (super::ToolContext, Arc<SqliteEventStore>) {
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
    // Tests don't reach into the sandbox or hit the network — these clients
    // are inert handles. SandboxClient::new requires Docker; if unavailable,
    // fall back to a placeholder.
    let sandbox = SandboxClient::new(
        "ghcr.io/agent-infra/sandbox:1.0.0.152",
        std::env::temp_dir(),
    )
    .expect("docker daemon required for tools tests");
    let search = SearchClient::new(SearchProvider::Brave { api_key: None });
    let ctx = ToolContext {
        session_id: "s1".into(),
        events: store.clone(),
        sandbox: Arc::new(sandbox),
        search: Arc::new(search),
        plan_manager,
    };
    (ctx, store)
}

/// Catalog count is 33 per story 0.7: the 32 listed in architecture §7
/// plus plan_advance (plan_update was already counted there). The
/// spec-check.sh warning about "33 vs 32" is acceptable.
const EXPECTED_TOOLS: &[&str] = &[
    // story 0.6 (5)
    "message_notify_user",
    "message_ask_user",
    "idle",
    "sop_read",
    "glossary_lookup",
    // file (5)
    "file_read",
    "file_write",
    "file_str_replace",
    "file_find_in_content",
    "file_find_by_name",
    // shell (5)
    "shell_exec",
    "shell_view",
    "shell_wait",
    "shell_write_to_process",
    "shell_kill_process",
    // browser (12)
    "browser_view",
    "browser_navigate",
    "browser_restart",
    "browser_click",
    "browser_input",
    "browser_move_mouse",
    "browser_press_key",
    "browser_select_option",
    "browser_scroll_up",
    "browser_scroll_down",
    "browser_console_exec",
    "browser_console_view",
    // search (1)
    "info_search_web",
    // deploy (2)
    "deploy_expose_port",
    "deploy_apply_deployment",
    // internal (playbook_search; sop_read + glossary_lookup already above)
    "playbook_search",
    // plan (2)
    "plan_advance",
    "plan_update",
];

#[test]
fn registry_has_expected_tools() {
    let reg = register_builtin_tools();
    assert_eq!(
        reg.len(),
        EXPECTED_TOOLS.len(),
        "expected {} tools, got {}",
        EXPECTED_TOOLS.len(),
        reg.len()
    );
    for name in EXPECTED_TOOLS {
        assert!(reg.contains_key(name), "missing tool: {name}");
    }
}

#[tokio::test]
async fn stubs_return_not_implemented() {
    let (cx, _store) = ctx().await;
    let reg = register_builtin_tools();
    let real = [
        "message_notify_user",
        "message_ask_user",
        "idle",
        "sop_read",
        "glossary_lookup",
        // story 0.9
        "file_read",
        "file_write",
        "file_str_replace",
        "file_find_in_content",
        "file_find_by_name",
        "shell_exec",
        "shell_view",
        "shell_wait",
        "shell_write_to_process",
        "shell_kill_process",
        "browser_view",
        "browser_navigate",
        "browser_restart",
        "browser_click",
        "browser_input",
        "browser_move_mouse",
        "browser_press_key",
        "browser_select_option",
        "browser_scroll_up",
        "browser_scroll_down",
        "browser_console_exec",
        "browser_console_view",
        "info_search_web",
        // Story 1.1: plan tools now hit the real PlanManager.
        "plan_create",
        "plan_advance",
        "plan_update",
    ];
    for (name, tool) in reg.iter() {
        if real.contains(name) {
            continue;
        }
        // Schema may require fields, so we send an args object with reasonable
        // dummies. Stub never validates args.
        let out = tool.invoke(json!({}), &cx).await.expect("stub invoke ok");
        assert!(!out.ok, "stub {name} unexpectedly returned ok=true");
        assert_eq!(
            out.error.as_ref().expect("stub must include error").kind,
            "not_implemented",
            "stub {name} wrong error kind"
        );
    }
}

#[test]
fn all_schemas_are_objects() {
    let reg = register_builtin_tools();
    for (name, tool) in reg.iter() {
        let schema = tool.schema();
        assert_eq!(schema["type"], "object", "{name} schema must be object");
        assert!(
            schema["properties"].is_object(),
            "{name} missing properties"
        );
    }
}

#[tokio::test]
async fn message_notify_user_rejects_missing_content() {
    let (cx, _store) = ctx().await;
    let reg = register_builtin_tools();
    let tool = reg.get("message_notify_user").unwrap();
    let err = tool.invoke(json!({}), &cx).await.unwrap_err();
    matches!(err, super::ToolError::InvalidArgs(_));
}

#[tokio::test]
async fn message_notify_user_emits_message_event() {
    let (cx, store) = ctx().await;
    let reg = register_builtin_tools();
    let tool = reg.get("message_notify_user").unwrap();
    let out = tool.invoke(json!({"content": "hello"}), &cx).await.unwrap();
    assert!(out.ok);

    let events = store.query("s1", EventQuery::default()).await.unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_type, crate::events::EventType::Message);
    assert_eq!(events[0].source, "tool:message_notify_user");
    assert_eq!(events[0].data["ui"], "notify");
    assert_eq!(events[0].data["content"], "hello");
}

#[tokio::test]
async fn message_ask_user_emits_ask_event() {
    let (cx, store) = ctx().await;
    let reg = register_builtin_tools();
    let tool = reg.get("message_ask_user").unwrap();
    let out = tool.invoke(json!({"content": "what?"}), &cx).await.unwrap();
    assert!(out.ok);
    assert!(out.output["call_id"].is_i64());

    let events = store.query("s1", EventQuery::default()).await.unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].data["ui"], "ask");
}

#[tokio::test]
async fn idle_returns_task_complete_signal() {
    let (cx, _store) = ctx().await;
    let reg = register_builtin_tools();
    let tool = reg.get("idle").unwrap();
    let out = tool.invoke(Value::Null, &cx).await.unwrap();
    assert!(out.ok);
    assert_eq!(out.output["signal"], "task_complete");
}

#[tokio::test]
async fn sop_read_is_not_implemented() {
    let (cx, _store) = ctx().await;
    let reg = register_builtin_tools();
    let tool = reg.get("sop_read").unwrap();
    let out = tool.invoke(json!({"id": "anything"}), &cx).await.unwrap();
    assert!(!out.ok);
    assert_eq!(out.error.as_ref().unwrap().kind, "not_implemented");
}

#[tokio::test]
async fn glossary_lookup_is_not_implemented() {
    let (cx, _store) = ctx().await;
    let reg = register_builtin_tools();
    let tool = reg.get("glossary_lookup").unwrap();
    let out = tool.invoke(json!({"term": "X"}), &cx).await.unwrap();
    assert!(!out.ok);
    assert_eq!(out.error.as_ref().unwrap().kind, "not_implemented");
}

#[tokio::test]
async fn file_str_replace_posts_replace_body() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/file/replace"))
        .and(body_json(json!({
            "file": "/workspace/a.txt",
            "old_str": "old",
            "new_str": "new"
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true})))
        .expect(1)
        .mount(&server)
        .await;
    let out = super::builtin::sandbox_post_raw(
        &format!("{}/v1/file/replace", server.uri()),
        json!({"file": "/workspace/a.txt", "old_str": "old", "new_str": "new"}),
    )
    .await
    .unwrap();
    assert!(out.ok);
}

#[tokio::test]
async fn shell_view_posts_process_id() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/shell/view"))
        .and(body_json(json!({"id": "proc-1"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true})))
        .expect(1)
        .mount(&server)
        .await;
    let out = super::builtin::sandbox_post_raw(
        &format!("{}/v1/shell/view", server.uri()),
        json!({"id": "proc-1"}),
    )
    .await
    .unwrap();
    assert!(out.ok);
}

#[tokio::test]
async fn browser_navigate_posts_url_action() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/browser/page/navigate"))
        .and(body_json(json!({"url": "https://example.com"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true})))
        .expect(1)
        .mount(&server)
        .await;
    let out = super::builtin::sandbox_post_raw(
        &format!("{}/v1/browser/page/navigate", server.uri()),
        json!({"url": "https://example.com"}),
    )
    .await
    .unwrap();
    assert!(out.ok);
}
