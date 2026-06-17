use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::templates::template_for;
use super::{NarrationConfig, NarratorHook};
use crate::checkpoint::{CheckpointLabelBuffer, CheckpointStore};
use crate::db;
use crate::dispatch::hooks::Hook;
use crate::dispatch::mask::AgentMode;
use crate::events::sqlite::SqliteEventStore;
use crate::events::{EventQuery, EventStore, EventType};
use crate::llm::{LlmClient, Role};
use crate::plan::PlanManager;
use crate::sandbox::SandboxClient;
use crate::search::{SearchClient, SearchProvider};
use crate::tools::ToolContext;

const SESSION: &str = "s1";

struct Fixture {
    ctx: ToolContext,
    events: Arc<SqliteEventStore>,
    plan_manager: Arc<PlanManager>,
}

async fn fixture() -> Fixture {
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
    let sandbox = SandboxClient::new(
        "ghcr.io/agent-infra/sandbox:1.0.0.152",
        std::env::temp_dir(),
    )
    .expect("docker daemon required for narrate tests");
    let search = SearchClient::new(SearchProvider::Brave { api_key: None });
    let ctx = ToolContext {
        session_id: SESSION.into(),
        mask_mode: AgentMode::Worker,
        events: events.clone(),
        sandbox: Arc::new(sandbox),
        search: Arc::new(search),
        plan_manager: plan_manager.clone(),
        checkpoint_labels: Arc::new(CheckpointLabelBuffer::new()),
        checkpoints,
        matcher_mode: crate::matcher::MatcherMode::Production,
    };
    Fixture {
        ctx,
        events,
        plan_manager,
    }
}

async fn all_events(events: &SqliteEventStore) -> Vec<crate::events::Event> {
    events.query(SESSION, EventQuery::default()).await.unwrap()
}

fn narration_contents(events: &[crate::events::Event]) -> Vec<String> {
    events
        .iter()
        .filter(|e| {
            e.event_type == EventType::Message
                && e.data.get("ui").and_then(Value::as_str) == Some("narrate")
        })
        .filter_map(|e| {
            e.data
                .get("content")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .collect()
}

#[tokio::test]
async fn templated_path_returns_expected_string() {
    let cases = [
        ("plan_advance", json!({}), "Advancing the plan"),
        ("plan_update", json!({}), "Updating the plan"),
        ("plan_create", json!({}), "Drafting the plan"),
        ("idle", json!({}), "Wrapping up"),
        (
            "feature_mark_done",
            json!({"feature_id": "f-7"}),
            "Marking feature f-7 done",
        ),
        ("progress_update", json!({}), "Logging progress"),
        ("checkpoint_label", json!({}), "Labeling next checkpoint"),
        (
            "file_read",
            json!({"path": "/workspace/README.md"}),
            "Reading /workspace/README.md",
        ),
        (
            "file_find_by_name",
            json!({}),
            "Searching workspace for a file",
        ),
        (
            "file_find_in_content",
            json!({}),
            "Searching workspace content",
        ),
        ("glossary_lookup", json!({}), "Looking up the glossary"),
        ("playbook_search", json!({}), "Searching playbooks"),
        ("sop_read", json!({}), "Reading an SOP"),
    ];

    for (tool, args, expected) in cases {
        assert_eq!(template_for(tool, &args), expected, "tool {tool}");
    }

    // Also exercise the hook end-to-end on `file_read` so the event
    // shape (role/content/ui/call_id) is covered alongside the
    // templates::template_for unit assertions.
    let f = fixture().await;
    let hook = NarratorHook::new(f.events.clone());
    hook.pre(
        "call-templated",
        "file_read",
        &json!({"path": "/workspace/notes.md"}),
        &f.ctx,
    )
    .await;
    let events = all_events(&f.events).await;
    let narrate = events
        .iter()
        .find(|e| e.event_type == EventType::Message)
        .expect("narration emitted");
    assert_eq!(narrate.data["role"], "assistant");
    assert_eq!(narrate.data["ui"], "narrate");
    assert_eq!(narrate.data["call_id"], "call-templated");
    assert_eq!(narrate.data["content"], "Reading /workspace/notes.md");
}

#[tokio::test]
async fn llm_path_calls_classifier_slot() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "x",
            "model": "classifier-mini",
            "choices": [{
                "index": 0,
                "finish_reason": "stop",
                "message": {
                    "role": "assistant",
                    "content": "Writing the README",
                },
            }],
        })))
        .expect(1)
        .mount(&mock)
        .await;

    let f = fixture().await;
    let llm = Arc::new(LlmClient::new(mock.uri(), None));
    let hook = NarratorHook::new(f.events.clone()).with_classifier(
        llm,
        "classifier-mini",
        Arc::new("system".into()),
    );

    hook.pre(
        "call-llm",
        "file_write",
        &json!({"path": "/workspace/README.md", "content": "..."}),
        &f.ctx,
    )
    .await;

    let events = all_events(&f.events).await;
    let texts = narration_contents(&events);
    assert_eq!(texts, vec!["Writing the README".to_string()]);
}

#[tokio::test]
async fn llm_timeout_emits_skipped_misc_and_does_not_block() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({
                    "id": "x",
                    "model": "classifier-mini",
                    "choices": [{
                        "index": 0,
                        "finish_reason": "stop",
                        "message": {"role": "assistant", "content": "Too slow"},
                    }],
                }))
                .set_delay(Duration::from_secs(5)),
        )
        .mount(&mock)
        .await;

    let f = fixture().await;
    let llm = Arc::new(LlmClient::new(mock.uri(), None));
    let config = NarrationConfig {
        enabled: true,
        llm_path: vec!["shell_*".into()],
        timeout: Duration::from_millis(50),
    };
    let hook = NarratorHook::new(f.events.clone())
        .with_config(config)
        .with_classifier(llm, "classifier-mini", Arc::new("system".into()));

    hook.pre(
        "call-timeout",
        "shell_exec",
        &json!({"command": "ls"}),
        &f.ctx,
    )
    .await;

    let events = all_events(&f.events).await;
    let skipped = events
        .iter()
        .find(|e| {
            e.event_type == EventType::Misc
                && e.data.get("kind").and_then(Value::as_str) == Some("narration_skipped")
        })
        .expect("narration_skipped emitted");
    assert_eq!(skipped.data["tool"], "shell_exec");
    assert_eq!(skipped.data["reason"], "timeout");
    assert!(
        narration_contents(&events).is_empty(),
        "no narration on timeout"
    );
}

#[tokio::test]
async fn narration_excluded_from_agent_context() {
    let f = fixture().await;
    let hook = NarratorHook::new(f.events.clone());
    // Mix narration events with a regular user Message so the
    // assertion is sharper: only the user Message must reach
    // build_messages, the three narrate Messages must not.
    f.events
        .append(crate::events::NewEvent {
            session_id: SESSION.into(),
            event_type: EventType::Message,
            source: "user".into(),
            data: json!({"role": "user", "content": "Hello"}),
        })
        .await
        .unwrap();
    for (call_id, tool) in [("c1", "plan_advance"), ("c2", "idle"), ("c3", "file_read")] {
        hook.pre(call_id, tool, &json!({}), &f.ctx).await;
    }

    let messages = crate::agent::build_messages(&f.events, &f.plan_manager, SESSION)
        .await
        .expect("build_messages");

    let user_count = messages.iter().filter(|m| m.role == Role::User).count();
    let assistant_count = messages
        .iter()
        .filter(|m| m.role == Role::Assistant)
        .count();
    assert_eq!(user_count, 1, "the one user Message survives");
    assert_eq!(
        assistant_count, 0,
        "all three narrate Messages are filtered out"
    );
}

#[tokio::test]
async fn disabled_config_emits_no_narration() {
    let f = fixture().await;
    let hook = NarratorHook::new(f.events.clone()).with_config(NarrationConfig {
        enabled: false,
        ..NarrationConfig::default()
    });
    hook.pre("call-x", "file_read", &json!({"path": "/a"}), &f.ctx)
        .await;
    let events = all_events(&f.events).await;
    assert!(events.is_empty(), "no events with narrator disabled");
}

#[tokio::test]
async fn unknown_tool_falls_through_to_templated_generic() {
    let f = fixture().await;
    let hook = NarratorHook::new(f.events.clone());
    hook.pre(
        "call-unknown",
        "experimental_brand_new_tool",
        &json!({}),
        &f.ctx,
    )
    .await;
    let events = all_events(&f.events).await;
    let texts = narration_contents(&events);
    assert_eq!(
        texts,
        vec!["Invoking experimental_brand_new_tool".to_string()]
    );
}

#[tokio::test]
async fn build_messages_anchors_seed_brief_and_keeps_recent_window() {
    // Issue #22: on a long task the per-iteration context must keep the most
    // RECENT events (what the agent just did) and still anchor the original brief,
    // while events in the middle fall out of the bounded window.
    let f = fixture().await;
    let push = |content: String| {
        let events = f.events.clone();
        async move {
            events
                .append(crate::events::NewEvent {
                    session_id: SESSION.into(),
                    event_type: EventType::Message,
                    source: "user".into(),
                    data: json!({"role": "user", "content": content}),
                })
                .await
                .unwrap();
        }
    };
    push("ORIGINAL_BRIEFING".into()).await; // event 1 (the seed)
    push("EARLY_FILLER".into()).await; // event 2 (early, non-seed)
    for i in 0..130 {
        push(format!("FILLER_{i}")).await;
    }
    push("MOST_RECENT".into()).await; // the tail

    let messages = crate::agent::build_messages(&f.events, &f.plan_manager, SESSION)
        .await
        .expect("build_messages");
    let blob = messages
        .iter()
        .filter_map(|m| m.content.clone())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(
        blob.contains("ORIGINAL_BRIEFING"),
        "the seed brief must be anchored even past the window"
    );
    assert!(
        blob.contains("MOST_RECENT"),
        "the most recent activity must be in the window"
    );
    assert!(
        !blob.contains("EARLY_FILLER"),
        "an early non-seed event must fall out of the bounded window"
    );
}
