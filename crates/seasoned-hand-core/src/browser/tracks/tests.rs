use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::PostBrowserActionHook;
use crate::checkpoint::{CheckpointLabelBuffer, CheckpointStore};
use crate::db;
use crate::dispatch::hooks::Hook;
use crate::dispatch::mask::AgentMode;
use crate::events::payload::EventPayloadBody;
use crate::events::sqlite::SqliteEventStore;
use crate::events::{EventQuery, EventStore, EventType};
use crate::plan::PlanManager;
use crate::sandbox::{SandboxClient, SandboxHandle};
use crate::search::{SearchClient, SearchProvider};
use crate::tools::{ToolContext, ToolOutput};

const SESSION: &str = "s1";

struct Fixture {
    ctx: ToolContext,
    events: Arc<SqliteEventStore>,
    hook: PostBrowserActionHook,
    workspace: tempfile::TempDir,
    _mock: MockServer,
}

async fn fixture_with_mock(mock: MockServer) -> Fixture {
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
    let events = Arc::new(SqliteEventStore::new(pool.clone()));
    let plan_manager = Arc::new(PlanManager::new(pool.clone(), events.clone()));
    let checkpoints = Arc::new(CheckpointStore::new(pool));

    let workspace = tempfile::tempdir().unwrap();
    let session_workspace = workspace.path().join(SESSION);
    std::fs::create_dir_all(&session_workspace).unwrap();

    let sandbox = SandboxClient::new("ghcr.io/agent-infra/sandbox:1.0.0.152", workspace.path())
        .expect("docker daemon required for browser::tracks tests");
    sandbox
        .insert_handle_for_test(SandboxHandle {
            session_id: SESSION.into(),
            container_id: "test".into(),
            api_url: mock.uri(),
            novnc_url: "http://127.0.0.1:0".into(),
            ttyd_url: "ws://127.0.0.1:0".into(),
            workspace_host_path: session_workspace,
        })
        .await;
    let sandbox = Arc::new(sandbox);

    let search = SearchClient::new(SearchProvider::Brave { api_key: None });

    let ctx = ToolContext {
        session_id: SESSION.into(),
        mask_mode: AgentMode::Worker,
        events: events.clone(),
        sandbox,
        search: Arc::new(search),
        plan_manager,
        checkpoint_labels: Arc::new(CheckpointLabelBuffer::new()),
        checkpoints,
        matcher_mode: crate::matcher::MatcherMode::Production,
    };
    let hook = PostBrowserActionHook::new(events.clone());
    Fixture {
        ctx,
        events,
        hook,
        workspace,
        _mock: mock,
    }
}

async fn list_misc(events: &SqliteEventStore) -> Vec<Value> {
    events
        .query(
            SESSION,
            EventQuery {
                after_id: None,
                event_type: Some(EventType::Misc),
                limit: None,
            },
        )
        .await
        .unwrap()
        .into_iter()
        .map(|e| e.data)
        .collect()
}

fn ok_output(output: Value) -> ToolOutput {
    ToolOutput {
        ok: true,
        output,
        file_ref: None,
        error: None,
    }
}

async fn mount_browser_view_mock(server: &MockServer, info: Value, elements: Value) {
    Mock::given(method("GET"))
        .and(path("/v1/browser/info"))
        .respond_with(ResponseTemplate::new(200).set_body_json(info))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/browser/page/elements"))
        .respond_with(ResponseTemplate::new(200).set_body_json(elements))
        .mount(server)
        .await;
}

async fn mount_screenshot_mock(server: &MockServer, bytes: Vec<u8>) {
    Mock::given(method("GET"))
        .and(path("/v1/browser/screenshot"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(bytes, "image/png"))
        .mount(server)
        .await;
}

#[tokio::test]
async fn browser_view_reuses_dom_text() {
    let mock = MockServer::start().await;
    // Track C is still captured — provide the screenshot endpoint.
    mount_screenshot_mock(&mock, vec![0x89, 0x50, 0x4e, 0x47]).await;
    // Browser view endpoints should NOT be hit; mount with .expect(0)
    // so wiremock fails the test on drop if either fires.
    Mock::given(method("GET"))
        .and(path("/v1/browser/info"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .expect(0)
        .mount(&mock)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/browser/page/elements"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .expect(0)
        .mount(&mock)
        .await;

    let f = fixture_with_mock(mock).await;
    let output = ok_output(json!({
        "browser_info": {"url": "https://example.org"},
        "elements": {"text": "hello world"},
    }));
    f.hook
        .post("call-1", "browser_view", &Value::Null, &output, &f.ctx)
        .await;

    let misc = list_misc(&f.events).await;
    let track_b = misc
        .iter()
        .find(|m| m.get("kind").and_then(Value::as_str) == Some("browser_track_b"))
        .expect("browser_track_b emitted");
    assert_eq!(track_b["call_id"], "call-1");
    let dom = &track_b["dom_text_ref"];
    assert_eq!(dom["kind"], "inline");
    let inline_bytes = dom["bytes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_u64().unwrap() as u8)
        .collect::<Vec<_>>();
    assert_eq!(std::str::from_utf8(&inline_bytes).unwrap(), "hello world");
}

#[tokio::test]
async fn browser_click_captures_both_tracks_b_and_c() {
    let mock = MockServer::start().await;
    mount_browser_view_mock(
        &mock,
        json!({"url": "https://example.org"}),
        json!({"text": "post-click DOM"}),
    )
    .await;
    mount_screenshot_mock(&mock, b"PNGDATA".to_vec()).await;

    let f = fixture_with_mock(mock).await;
    let output = ok_output(json!({"ok": true}));
    f.hook
        .post("call-2", "browser_click", &Value::Null, &output, &f.ctx)
        .await;

    let misc = list_misc(&f.events).await;
    let has_b = misc
        .iter()
        .any(|m| m.get("kind").and_then(Value::as_str) == Some("browser_track_b"));
    let has_c = misc
        .iter()
        .any(|m| m.get("kind").and_then(Value::as_str) == Some("browser_track_c"));
    assert!(has_b, "browser_track_b missing: {misc:?}");
    assert!(has_c, "browser_track_c missing: {misc:?}");

    let track_c = misc
        .iter()
        .find(|m| m.get("kind").and_then(Value::as_str) == Some("browser_track_c"))
        .unwrap();
    assert_eq!(track_c["call_id"], "call-2");
    assert_eq!(track_c["file_ref"]["content_type"], "image/png");
    assert_eq!(track_c["file_ref"]["size"], 7);
    assert_eq!(track_c["file_ref"]["path"], "/workspace/.tracks/call-2.png");
}

#[tokio::test]
async fn large_dom_text_becomes_file_ref() {
    let mock = MockServer::start().await;
    let big_text = "A".repeat(50 * 1024);
    mount_browser_view_mock(&mock, json!({}), json!({"text": big_text.clone()})).await;
    mount_screenshot_mock(&mock, vec![0x89, 0x50, 0x4e, 0x47]).await;

    let f = fixture_with_mock(mock).await;
    let output = ok_output(json!({"ok": true}));
    f.hook
        .post("call-3", "browser_navigate", &Value::Null, &output, &f.ctx)
        .await;

    let misc = list_misc(&f.events).await;
    let track_b = misc
        .iter()
        .find(|m| m.get("kind").and_then(Value::as_str) == Some("browser_track_b"))
        .expect("browser_track_b emitted");
    let dom = &track_b["dom_text_ref"];
    assert_eq!(dom["kind"], "file_ref");
    let path = dom["path"].as_str().unwrap();
    let body = f
        .ctx
        .sandbox
        .read_workspace_file(SESSION, path)
        .await
        .unwrap();
    assert_eq!(body.len(), big_text.len());
    assert!(body.iter().all(|b| *b == b'A'));

    // Sanity: round-trip via the typed enum to ensure shape compatibility
    // with the rest of the event pipeline.
    let parsed: EventPayloadBody = serde_json::from_value(dom.clone()).unwrap();
    matches!(parsed, EventPayloadBody::FileRef { .. });
}

#[tokio::test]
async fn screenshot_timeout_emits_skipped_misc() {
    let mock = MockServer::start().await;
    mount_browser_view_mock(&mock, json!({}), json!({"text": "ok"})).await;
    // Make the screenshot endpoint hang past the hook's per-call budget.
    Mock::given(method("GET"))
        .and(path("/v1/browser/screenshot"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw(b"PNG".to_vec(), "image/png")
                .set_delay(Duration::from_secs(5)),
        )
        .mount(&mock)
        .await;

    let mut f = fixture_with_mock(mock).await;
    f.hook = PostBrowserActionHook::new(f.events.clone())
        .with_screenshot_timeout(Duration::from_millis(50));
    let output = ok_output(json!({"ok": true}));
    f.hook
        .post("call-4", "browser_click", &Value::Null, &output, &f.ctx)
        .await;

    let misc = list_misc(&f.events).await;
    let skipped = misc
        .iter()
        .find(|m| m.get("kind").and_then(Value::as_str) == Some("browser_track_c_skipped"))
        .expect("browser_track_c_skipped emitted");
    assert_eq!(skipped["call_id"], "call-4");
    assert_eq!(skipped["reason"], "timeout");
    let has_c = misc
        .iter()
        .any(|m| m.get("kind").and_then(Value::as_str) == Some("browser_track_c"));
    assert!(!has_c, "track_c must not be emitted when capture timed out");
}

#[tokio::test]
async fn non_browser_tool_does_not_trigger_hook() {
    let mock = MockServer::start().await;
    // No endpoints mounted — if the hook fires, the GET 404s would
    // surface as browser_track_*_skipped events.
    let f = fixture_with_mock(mock).await;
    let output = ok_output(json!({"content": "ignored"}));
    f.hook
        .post("call-5", "file_read", &Value::Null, &output, &f.ctx)
        .await;

    let misc = list_misc(&f.events).await;
    assert!(misc.is_empty(), "no events expected: {misc:?}");
}

#[tokio::test]
async fn track_c_filename_matches_call_id() {
    let mock = MockServer::start().await;
    mount_browser_view_mock(&mock, json!({}), json!({"text": "view"})).await;
    mount_screenshot_mock(&mock, b"PNGBYTES".to_vec()).await;

    let f = fixture_with_mock(mock).await;
    let output = ok_output(json!({"ok": true}));
    f.hook
        .post(
            "call-6-with-uuid",
            "browser_click",
            &Value::Null,
            &output,
            &f.ctx,
        )
        .await;

    let expected = f
        .workspace
        .path()
        .join(SESSION)
        .join(".tracks/call-6-with-uuid.png");
    assert!(expected.exists(), "screenshot file missing at {expected:?}");
    let bytes = tokio::fs::read(&expected).await.unwrap();
    assert_eq!(bytes, b"PNGBYTES");
}
