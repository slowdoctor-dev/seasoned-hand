use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use tempfile::tempdir;
use wiremock::matchers::{body_partial_json, method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

use super::*;
use crate::cost::CostClient;
use crate::db;
use crate::dispatch::hooks::EventEmittingHook;
use crate::events::EventQuery;
use crate::pubsub::RedisPool;
use crate::router::SlotRouter;
use crate::search::SearchProvider;
use crate::tools::register_builtin_tools;

#[derive(Clone)]
struct ScriptedResponder {
    responses: Arc<Mutex<VecDeque<Value>>>,
    requests: Arc<Mutex<Vec<Value>>>,
}

impl ScriptedResponder {
    fn new(responses: Vec<Value>, requests: Arc<Mutex<Vec<Value>>>) -> Self {
        Self {
            responses: Arc::new(Mutex::new(VecDeque::from(responses))),
            requests,
        }
    }
}

impl Respond for ScriptedResponder {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        if let Ok(body) = serde_json::from_slice(&request.body) {
            let mut requests = self.requests.lock().expect("request lock poisoned");
            requests.push(body);
        }
        let mut guard = self.responses.lock().expect("script lock poisoned");
        match guard.pop_front() {
            Some(body) => ResponseTemplate::new(200).set_body_json(body),
            None => ResponseTemplate::new(500).set_body_string("script exhausted"),
        }
    }
}

struct Harness {
    runner: AgentRunner,
    db: DbPool,
    events: Arc<SqliteEventStore>,
    session_id: String,
    requests: Arc<Mutex<Vec<Value>>>,
    _mock: MockServer,
    _workspace: tempfile::TempDir,
}

async fn harness(responses: Vec<Value>) -> Harness {
    harness_with_cost(responses, Vec::new()).await
}

async fn harness_with_cost(
    responses: Vec<Value>,
    cost_responses: Vec<ResponseTemplate>,
) -> Harness {
    let mock = MockServer::start().await;
    let requests = Arc::new(Mutex::new(Vec::new()));
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .and(body_partial_json(json!({"tool_choice": "required"})))
        .respond_with(ScriptedResponder::new(responses, requests.clone()))
        .mount(&mock)
        .await;
    if !cost_responses.is_empty() {
        Mock::given(method("GET"))
            .and(path("/cost"))
            .respond_with(CostResponder::new(cost_responses))
            .mount(&mock)
            .await;
    }

    let db = db::open(":memory:").await.expect("in-memory db opens");
    let session_id = "session-1".to_string();
    insert_session(&db, &session_id).await;
    let redis = RedisPool::new("redis://127.0.0.1:6379").expect("redis url parses");
    let events = Arc::new(SqliteEventStore::with_redis(db.clone(), redis));
    let dispatcher = Arc::new(
        ToolDispatcher::new(register_builtin_tools())
            .with_hook(Arc::new(EventEmittingHook::new(events.clone()))),
    );
    let router = Arc::new(
        SlotRouter::from_yaml_str(&format!(
            r#"
slots:
  main:
    provider: bifrost
    model: agent-primary
    base_url: {}
"#,
            mock.uri()
        ))
        .expect("router config parses"),
    );
    let workspace = tempdir().expect("tempdir");
    let sandbox = Arc::new(
        SandboxClient::new("ghcr.io/agent-infra/sandbox:1.0.0.152", workspace.path())
            .expect("sandbox client constructs"),
    );
    let search = Arc::new(SearchClient::new(SearchProvider::Brave { api_key: None }));
    let llm = LlmClient::new(mock.uri(), None);
    let cost = Arc::new(CostClient::new(mock.uri()));
    let runner = AgentRunner::new(AgentRunnerDeps {
        llm,
        dispatcher,
        events: events.clone(),
        router,
        sandbox,
        search,
        cost,
        sessions: db.clone(),
    });

    Harness {
        runner,
        db,
        events,
        session_id,
        requests,
        _mock: mock,
        _workspace: workspace,
    }
}

#[derive(Clone)]
struct CostResponder {
    responses: Arc<Mutex<VecDeque<ResponseTemplate>>>,
}

impl CostResponder {
    fn new(responses: Vec<ResponseTemplate>) -> Self {
        Self {
            responses: Arc::new(Mutex::new(VecDeque::from(responses))),
        }
    }
}

impl Respond for CostResponder {
    fn respond(&self, _request: &Request) -> ResponseTemplate {
        let mut guard = self.responses.lock().expect("cost script lock poisoned");
        guard
            .pop_front()
            .unwrap_or_else(|| ResponseTemplate::new(500).set_body_string("cost script exhausted"))
    }
}

async fn insert_session(db: &DbPool, session_id: &str) {
    let session_id = session_id.to_string();
    db.with_conn(move |conn| {
        conn.execute(
            "INSERT INTO sessions (id, created_at, updated_at, state) VALUES (?, 0, 0, 'IDLE')",
            rusqlite::params![session_id],
        )
    })
    .await
    .expect("session insert succeeds");
}

fn req(session_id: &str, max_steps: u32) -> RunRequest {
    RunRequest {
        session_id: session_id.to_string(),
        input: "Find one thing and finish".into(),
        max_steps,
        cost_cap_cents: Some(100),
    }
}

fn completion(calls: Vec<(&str, &str, Value)>) -> Value {
    json!({
        "id": "cmpl-test",
        "object": "chat.completion",
        "model": "agent-primary",
        "choices": [{
            "index": 0,
            "finish_reason": "tool_calls",
            "message": {
                "role": "assistant",
                "content": null,
                "tool_calls": calls
                    .into_iter()
                    .map(|(id, name, args)| json!({
                        "id": id,
                        "type": "function",
                        "function": {
                            "name": name,
                            "arguments": args.to_string(),
                        },
                    }))
                    .collect::<Vec<_>>()
            }
        }],
        "usage": { "prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2 }
    })
}

#[tokio::test]
async fn single_turn_idle_completes() {
    let h = harness(vec![completion(vec![("call-1", "idle", json!({}))])]).await;

    let result = h.runner.run(req(&h.session_id, 4)).await.expect("run");

    assert!(result.completed);
    assert_eq!(result.steps, 1);
    assert_eq!(session_state(&h.db, &h.session_id).await, "FINISHED");
}

#[tokio::test]
async fn multi_turn_search_then_idle() {
    let h = harness(vec![
        completion(vec![(
            "call-1",
            "info_search_web",
            json!({"query": "rust"}),
        )]),
        completion(vec![
            (
                "call-2",
                "message_notify_user",
                json!({"content": "search failed without key"}),
            ),
            ("call-ignored", "idle", json!({})),
        ]),
        completion(vec![("call-3", "idle", json!({}))]),
    ])
    .await;

    let result = h.runner.run(req(&h.session_id, 5)).await.expect("run");

    assert!(result.completed);
    assert_eq!(result.steps, 3);
    assert_eq!(session_state(&h.db, &h.session_id).await, "FINISHED");
}

#[tokio::test]
async fn one_tool_per_iteration_enforced() {
    let h = harness(vec![completion(vec![
        (
            "call-1",
            "message_notify_user",
            json!({"content": "first only"}),
        ),
        ("call-2", "idle", json!({})),
    ])])
    .await;

    let result = h.runner.run(req(&h.session_id, 1)).await.expect("run");
    let events = h
        .events
        .query(&h.session_id, EventQuery::default())
        .await
        .expect("events query");

    assert!(!result.completed);
    assert_eq!(result.steps, 1);
    assert!(events.iter().any(|event| {
        event.event_type == EventType::Misc
            && event.data.get("kind").and_then(Value::as_str) == Some("multi_tool_warning")
    }));
    let action_tools = events
        .iter()
        .filter(|event| event.event_type == EventType::Action)
        .map(|event| event.data.get("tool").and_then(Value::as_str))
        .collect::<Vec<_>>();
    assert_eq!(action_tools, vec![Some("message_notify_user")]);
}

#[tokio::test]
async fn max_steps_terminates() {
    let h = harness(vec![
        completion(vec![(
            "call-1",
            "message_notify_user",
            json!({"content": "still working"}),
        )]),
        completion(vec![(
            "call-2",
            "message_notify_user",
            json!({"content": "still working"}),
        )]),
    ])
    .await;

    let result = h.runner.run(req(&h.session_id, 2)).await.expect("run");
    let events = h
        .events
        .query(&h.session_id, EventQuery::default())
        .await
        .expect("events query");

    assert!(!result.completed);
    assert_eq!(result.steps, 2);
    assert!(events.iter().any(|event| {
        event.event_type == EventType::Misc
            && event.data.get("kind").and_then(Value::as_str) == Some("max_steps_reached")
    }));
}

#[tokio::test]
async fn message_ask_user_suspends_run() {
    let h = harness(vec![completion(vec![(
        "call-1",
        "message_ask_user",
        json!({"content": "Need input"}),
    )])])
    .await;

    let result = h.runner.run(req(&h.session_id, 4)).await.expect("run");

    assert!(!result.completed);
    assert_eq!(result.steps, 1);
    assert_eq!(session_state(&h.db, &h.session_id).await, "SUSPENDED");
}

#[tokio::test]
async fn agent_runner_emits_stuck_inject_event_and_continues() {
    let repeated = completion(vec![(
        "call-1",
        "message_notify_user",
        json!({"content": "same"}),
    )]);
    let h = harness(vec![
        repeated.clone(),
        repeated,
        completion(vec![("call-3", "idle", json!({}))]),
    ])
    .await;

    let result = h.runner.run(req(&h.session_id, 4)).await.expect("run");
    let events = h
        .events
        .query(&h.session_id, EventQuery::default())
        .await
        .expect("events query");
    let requests = h.requests.lock().expect("request lock poisoned");

    assert!(result.completed);
    assert_eq!(result.steps, 3);
    assert!(events.iter().any(|event| {
        event.event_type == EventType::Misc
            && event.data.get("kind").and_then(Value::as_str) == Some("stuck_inject")
            && event.data.get("duplicate_count").and_then(Value::as_u64) == Some(2)
    }));
    let third_messages = requests[2]
        .get("messages")
        .and_then(Value::as_array)
        .expect("third request has messages");
    assert!(third_messages.iter().any(|message| {
        message.get("role").and_then(Value::as_str) == Some("system")
            && message
                .get("content")
                .and_then(Value::as_str)
                .is_some_and(|content| content.contains("Try a different strategy"))
    }));
}

#[tokio::test]
async fn agent_runner_terminates_on_four_duplicates() {
    let repeated = completion(vec![(
        "call-1",
        "message_notify_user",
        json!({"content": "same"}),
    )]);
    let h = harness(vec![
        repeated.clone(),
        repeated.clone(),
        repeated.clone(),
        repeated,
    ])
    .await;

    let error = h
        .runner
        .run(req(&h.session_id, 6))
        .await
        .expect_err("run should terminate as stuck");
    let events = h
        .events
        .query(&h.session_id, EventQuery::default())
        .await
        .expect("events query");

    assert!(matches!(error, AgentError::StuckTerminated { count: 4 }));
    assert_eq!(session_state(&h.db, &h.session_id).await, "ERROR");
    assert!(events.iter().any(|event| {
        event.event_type == EventType::Misc
            && event.data.get("kind").and_then(Value::as_str) == Some("stuck_terminate")
            && event.data.get("duplicate_count").and_then(Value::as_u64) == Some(4)
    }));
}

#[tokio::test]
async fn agent_runner_halts_on_cost_cap() {
    let h = harness_with_cost(
        vec![
            completion(vec![(
                "call-1",
                "message_notify_user",
                json!({"content": "costly"}),
            )]),
            completion(vec![("call-2", "idle", json!({}))]),
        ],
        vec![
            ResponseTemplate::new(200).set_body_json(json!({"total_cents": 0})),
            ResponseTemplate::new(200).set_body_json(json!({"total_cents": 5})),
        ],
    )
    .await;
    let mut request = req(&h.session_id, 4);
    request.cost_cap_cents = Some(5);

    let result = h.runner.run(request).await.expect("run");
    let events = h
        .events
        .query(&h.session_id, EventQuery::default())
        .await
        .expect("events query");

    assert!(!result.completed);
    assert_eq!(result.steps, 1);
    assert_eq!(session_state(&h.db, &h.session_id).await, "SUSPENDED");
    assert_eq!(session_cost(&h.db, &h.session_id).await, 5);
    assert!(events.iter().any(|event| {
        event.event_type == EventType::Misc
            && event.data.get("kind").and_then(Value::as_str) == Some("cost_cap")
            && event.data.get("current_cents").and_then(Value::as_i64) == Some(5)
    }));
}

#[tokio::test]
async fn agent_runner_continues_when_cost_poll_fails() {
    let h = harness_with_cost(
        vec![completion(vec![("call-1", "idle", json!({}))])],
        vec![
            ResponseTemplate::new(503).set_body_string("baseline unavailable"),
            ResponseTemplate::new(503).set_body_string("delta unavailable"),
        ],
    )
    .await;
    let mut request = req(&h.session_id, 2);
    request.cost_cap_cents = Some(1);

    let result = h.runner.run(request).await.expect("run");

    assert!(result.completed);
    assert_eq!(result.steps, 1);
    assert_eq!(session_state(&h.db, &h.session_id).await, "FINISHED");
    assert_eq!(session_cost(&h.db, &h.session_id).await, 0);
}

async fn session_state(db: &DbPool, session_id: &str) -> String {
    let session_id = session_id.to_string();
    db.with_conn(move |conn| {
        conn.query_row(
            "SELECT state FROM sessions WHERE id = ?",
            rusqlite::params![session_id],
            |row| row.get::<_, String>(0),
        )
    })
    .await
    .expect("state query")
}

async fn session_cost(db: &DbPool, session_id: &str) -> i64 {
    let session_id = session_id.to_string();
    db.with_conn(move |conn| {
        conn.query_row(
            "SELECT cost_cents FROM sessions WHERE id = ?",
            rusqlite::params![session_id],
            |row| row.get::<_, i64>(0),
        )
    })
    .await
    .expect("cost query")
}
