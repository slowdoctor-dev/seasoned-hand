use std::sync::Arc;

use serde_json::{Value, json};
use wiremock::matchers::{body_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::{ToolContext, register_builtin_tools};
use crate::db;
use crate::dispatch::mask::AgentMode;
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
    let plan_manager = Arc::new(PlanManager::new(pool.clone(), store.clone()));
    let checkpoints = Arc::new(crate::checkpoint::CheckpointStore::new(pool));
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
        mask_mode: AgentMode::Worker,
        events: store.clone(),
        sandbox: Arc::new(sandbox),
        search: Arc::new(search),
        plan_manager,
        checkpoint_labels: Arc::new(crate::checkpoint::CheckpointLabelBuffer::new()),
        checkpoints,
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
    "feature_mark_done",
    "progress_update",
    // checkpoint (2) — story 1.13 + 1.13b
    "checkpoint_label",
    "checkpoint_rollback",
    // deliverable (1) — story 2.14
    "task_deliver",
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
        "plan_advance",
        "plan_update",
        "feature_mark_done",
        "progress_update",
        // Story 1.13: real one-shot label tool.
        "checkpoint_label",
        // Story 1.13b: registered but masked from LLM; dispatch path is
        // real (admin endpoint + opt-in Verifier-driven). Treated as
        // `real` here so the stubs test doesn't try to invoke it
        // against the empty fixture (it would 404 on checkpoint_id).
        "checkpoint_rollback",
        // Story 2.14: registered as a not-wired placeholder in the
        // base `register_builtin_tools()` (production wiring is via
        // `all_with_task_deliver(deps)`). Surfaces
        // `task_deliver_not_wired` rather than `not_implemented` — the
        // distinction matters because operators need to know "deps
        // missing" vs "feature deferred". Excluded from the stub
        // check so it doesn't fail the kind assertion.
        "task_deliver",
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

#[tokio::test]
async fn feature_mark_done_flips_status_and_emits_event() {
    let (cx, store) = ctx().await;
    let ws = tempfile::tempdir().unwrap();
    cx.sandbox
        .insert_handle_for_test(crate::sandbox::SandboxHandle {
            session_id: "s1".into(),
            container_id: "c1".into(),
            api_url: "http://127.0.0.1:1".into(),
            novnc_url: "http://127.0.0.1:2".into(),
            ttyd_url: "ws://127.0.0.1:3".into(),
            workspace_host_path: ws.path().join("s1"),
        })
        .await;
    cx.plan_manager
        .create(
            "s1",
            "goal",
            vec![crate::plan::Phase {
                id: 1,
                title: "phase".into(),
                capabilities: vec![],
                status: crate::plan::PhaseStatus::Pending,
            }],
        )
        .await
        .unwrap();
    cx.sandbox
        .write_workspace_file(
            "s1",
            "feature-list.json",
            serde_json::to_vec(&json!({
                "version": 1,
                "goal": "goal",
                "features": [{"id":"f-1","title":"phase","status":"todo","plan_phase_id":1}]
            }))
            .unwrap()
            .as_slice(),
        )
        .await
        .unwrap();

    let out = register_builtin_tools()
        .get("feature_mark_done")
        .unwrap()
        .invoke(json!({"feature_id":"f-1"}), &cx)
        .await
        .unwrap();
    assert!(out.ok);

    let feature_list = cx
        .sandbox
        .read_workspace_file("s1", "feature-list.json")
        .await
        .unwrap();
    let feature_list: Value = serde_json::from_slice(&feature_list).unwrap();
    assert_eq!(feature_list["features"][0]["status"], "done");
    let events = store.query("s1", EventQuery::default()).await.unwrap();
    assert!(events.iter().any(|e| {
        e.event_type == crate::events::EventType::Misc
            && e.data.get("kind").and_then(Value::as_str) == Some("feature_done")
    }));
}

#[tokio::test]
async fn feature_mark_done_out_of_phase_emits_extra_misc() {
    let (cx, store) = ctx().await;
    let ws = tempfile::tempdir().unwrap();
    cx.sandbox
        .insert_handle_for_test(crate::sandbox::SandboxHandle {
            session_id: "s1".into(),
            container_id: "c1".into(),
            api_url: "http://127.0.0.1:1".into(),
            novnc_url: "http://127.0.0.1:2".into(),
            ttyd_url: "ws://127.0.0.1:3".into(),
            workspace_host_path: ws.path().join("s1"),
        })
        .await;
    cx.plan_manager
        .create(
            "s1",
            "goal",
            vec![crate::plan::Phase {
                id: 1,
                title: "phase".into(),
                capabilities: vec![],
                status: crate::plan::PhaseStatus::Pending,
            }],
        )
        .await
        .unwrap();
    cx.sandbox
        .write_workspace_file(
            "s1",
            "feature-list.json",
            serde_json::to_vec(&json!({
                "version": 1,
                "goal": "goal",
                "features": [{"id":"f-2","title":"other","status":"todo","plan_phase_id":2}]
            }))
            .unwrap()
            .as_slice(),
        )
        .await
        .unwrap();

    register_builtin_tools()
        .get("feature_mark_done")
        .unwrap()
        .invoke(json!({"feature_id":"f-2"}), &cx)
        .await
        .unwrap();

    let events = store.query("s1", EventQuery::default()).await.unwrap();
    assert!(events.iter().any(|e| {
        e.data.get("kind").and_then(Value::as_str) == Some("feature_done_out_of_phase")
    }));
}

#[tokio::test]
async fn progress_update_truncates_long_lines() {
    let (cx, _store) = ctx().await;
    let ws = tempfile::tempdir().unwrap();
    cx.sandbox
        .insert_handle_for_test(crate::sandbox::SandboxHandle {
            session_id: "s1".into(),
            container_id: "c1".into(),
            api_url: "http://127.0.0.1:1".into(),
            novnc_url: "http://127.0.0.1:2".into(),
            ttyd_url: "ws://127.0.0.1:3".into(),
            workspace_host_path: ws.path().join("s1"),
        })
        .await;
    cx.sandbox
        .write_workspace_file("s1", "progress.txt", b"seed\n")
        .await
        .unwrap();
    let line = "x".repeat(500);
    register_builtin_tools()
        .get("progress_update")
        .unwrap()
        .invoke(json!({"line": line}), &cx)
        .await
        .unwrap();
    let text = String::from_utf8_lossy(
        &cx.sandbox
            .read_workspace_file("s1", "progress.txt")
            .await
            .unwrap(),
    )
    .into_owned();
    let last = text.lines().last().unwrap();
    assert!(last.ends_with('…'));
}

// ============================================================================
// Story 1.13b: checkpoint_rollback tool body.
// ============================================================================

#[tokio::test]
async fn checkpoint_rollback_returns_404_when_checkpoint_missing() {
    let (cx, _events) = ctx().await;
    let out = register_builtin_tools()
        .get("checkpoint_rollback")
        .unwrap()
        .invoke(
            serde_json::json!({"checkpoint_id": "does-not-exist", "reason": "manual"}),
            &cx,
        )
        .await
        .expect("invoke");
    assert!(!out.ok, "missing id must return ok=false");
    assert_eq!(
        out.error.as_ref().map(|e| e.kind.as_str()),
        Some("checkpoint_not_found")
    );
}

#[tokio::test]
async fn checkpoint_rollback_validates_reason_length() {
    let (cx, _events) = ctx().await;
    let too_long = "x".repeat(201);
    let out = register_builtin_tools()
        .get("checkpoint_rollback")
        .unwrap()
        .invoke(
            serde_json::json!({"checkpoint_id": "any", "reason": too_long}),
            &cx,
        )
        .await
        .expect("invoke");
    assert!(!out.ok);
    assert_eq!(
        out.error.as_ref().map(|e| e.kind.as_str()),
        Some("reason_too_long")
    );
}

#[tokio::test]
async fn checkpoint_rollback_happy_path_marks_row_and_emits_misc() {
    use wiremock::matchers::{body_partial_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    // Stand up a wiremock sandbox HTTP API that returns a clean
    // /v1/shell/exec for the `git revert` command the tool issues.
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/shell/exec"))
        .and(body_partial_json(serde_json::json!({
            "command": "git -C /workspace revert --no-commit aaaa1111"
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "exit_code": 0, "stdout": "", "stderr": ""
        })))
        .mount(&mock)
        .await;

    let (cx, events) = ctx().await;
    // Point the sandbox handle at the wiremock so the tool's
    // `/v1/shell/exec` call lands there.
    cx.sandbox
        .insert_handle_for_test(crate::sandbox::SandboxHandle {
            session_id: "s1".into(),
            container_id: "c1".into(),
            api_url: mock.uri(),
            novnc_url: "http://127.0.0.1:2".into(),
            ttyd_url: "ws://127.0.0.1:3".into(),
            workspace_host_path: std::env::temp_dir().join("s1"),
        })
        .await;

    // Seed a checkpoint row.
    let cp_id = cx
        .checkpoints
        .insert(crate::checkpoint::NewCheckpoint {
            session_id: "s1".into(),
            plan_phase_id: 1,
            git_sha: "aaaa1111".into(),
            label: None,
            triggered_by_event_id: 1,
        })
        .await
        .unwrap();

    let out = register_builtin_tools()
        .get("checkpoint_rollback")
        .unwrap()
        .invoke(
            serde_json::json!({
                "checkpoint_id": cp_id,
                "reason": "manual rollback",
                "rolled_back_by": "admin:cli",
            }),
            &cx,
        )
        .await
        .expect("invoke");

    assert!(out.ok, "tool should succeed; got {out:?}");
    let row = cx.checkpoints.get(&cp_id).await.unwrap().unwrap();
    assert!(
        row.rolled_back_at.is_some(),
        "rolled_back_at must be set after revert"
    );
    assert_eq!(row.rolled_back_by.as_deref(), Some("admin:cli"));

    let evs = events
        .query(
            "s1",
            crate::events::EventQuery {
                after_id: None,
                event_type: Some(crate::events::EventType::Misc),
                limit: Some(50),
            },
        )
        .await
        .unwrap();
    assert!(
        evs.iter().any(|e| {
            e.data.get("kind").and_then(serde_json::Value::as_str) == Some("checkpoint_rollback")
                && e.data.get("git_sha").and_then(serde_json::Value::as_str) == Some("aaaa1111")
        }),
        "checkpoint_rollback Misc event must be emitted with the git_sha"
    );
}

#[tokio::test]
async fn checkpoint_rollback_surfaces_revert_failure() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/shell/exec"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "exit_code": 1, "stdout": "", "stderr": "error: conflicting changes"
        })))
        .mount(&mock)
        .await;

    let (cx, _events) = ctx().await;
    cx.sandbox
        .insert_handle_for_test(crate::sandbox::SandboxHandle {
            session_id: "s1".into(),
            container_id: "c1".into(),
            api_url: mock.uri(),
            novnc_url: "http://127.0.0.1:2".into(),
            ttyd_url: "ws://127.0.0.1:3".into(),
            workspace_host_path: std::env::temp_dir().join("s1"),
        })
        .await;
    let cp_id = cx
        .checkpoints
        .insert(crate::checkpoint::NewCheckpoint {
            session_id: "s1".into(),
            plan_phase_id: 1,
            git_sha: "bbbb2222".into(),
            label: None,
            triggered_by_event_id: 1,
        })
        .await
        .unwrap();

    let out = register_builtin_tools()
        .get("checkpoint_rollback")
        .unwrap()
        .invoke(
            serde_json::json!({"checkpoint_id": cp_id, "reason": "x"}),
            &cx,
        )
        .await
        .expect("invoke");

    assert!(!out.ok);
    assert_eq!(
        out.error.as_ref().map(|e| e.kind.as_str()),
        Some("revert_failed")
    );
    // Row must NOT be marked rolled-back on failure.
    let row = cx.checkpoints.get(&cp_id).await.unwrap().unwrap();
    assert!(row.rolled_back_at.is_none());
}
