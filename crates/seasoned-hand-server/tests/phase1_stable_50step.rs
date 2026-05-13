use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::{Json, Router, extract::State, routing::get, routing::post};
use seasoned_hand_core::agent::RunRequest;
use seasoned_hand_core::events::{EventQuery, EventStore, EventType, NewEvent};
use seasoned_hand_core::router::SlotRouter;
use seasoned_hand_core::sandbox::SandboxClient;
use seasoned_hand_core::search::{SearchClient, SearchProvider};
use seasoned_hand_core::verifier::gate::VerifierGate;
use seasoned_hand_core::verifier::{VerifyRequest, VerifyTrigger};
use seasoned_hand_core::verifier::{Worker, WorkerDeps};
use seasoned_hand_core::{db, pubsub};
use seasoned_hand_server::AppState;
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::sync::Mutex;

#[derive(Clone)]
struct MockBifrost {
    scripted: Arc<Mutex<VecDeque<Value>>>,
}

#[tokio::test]
async fn phase1_stable_50step() {
    let smoke = std::env::var("SEASONED_HAND_PHASE1_SMOKE").ok().as_deref() == Some("1");
    let started = Instant::now();

    let Some(bifrost_base) = start_mock_bifrost().await else {
        if smoke {
            panic!("SEASONED_HAND_PHASE1_SMOKE=1 requires local mock bind");
        }
        eprintln!("phase1_stable_50step skipped: unable to bind local mock port");
        return;
    };

    let pool = db::open(":memory:").await.expect("db");
    let redis = pubsub::RedisPool::new("redis://127.0.0.1:6").expect("redis url");
    let sandbox = SandboxClient::new(
        "ghcr.io/agent-infra/sandbox:1.0.0.152",
        std::env::temp_dir(),
    )
    .expect("sandbox");
    let search = SearchClient::new(SearchProvider::Brave { api_key: None });
    let router = SlotRouter::from_yaml_str(&format!(
        r#"
slots:
  main:
    provider: bifrost
    model: claude-sonnet-4-6
    base_url: {bifrost_base}
  verifier:
    provider: bifrost
    model: gpt-4o
    base_url: {bifrost_base}
"#
    ))
    .expect("router");

    let state = AppState::new(
        pool.clone(),
        redis,
        sandbox,
        search,
        router,
        Default::default(),
    )
    .with_verifier_prompt(Arc::new("You are verifier".to_string()));

    let session_id = "phase1-stable-50".to_string();
    insert_session(&state.db, &session_id).await;
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join(&session_id);
    std::fs::create_dir_all(&workspace).expect("workspace mkdir");
    state
        .sandbox
        .insert_handle_for_test(seasoned_hand_core::sandbox::SandboxHandle {
            session_id: session_id.clone(),
            container_id: "c1".into(),
            api_url: "http://127.0.0.1:1".into(),
            novnc_url: "http://127.0.0.1:2".into(),
            ttyd_url: "ws://127.0.0.1:3".into(),
            workspace_host_path: workspace,
        })
        .await;

    let run = state
        .runner
        .run(RunRequest {
            session_id: session_id.clone(),
            input: "execute a long deterministic task".to_string(),
            max_steps: 80,
            cost_cap_cents: Some(50),
        })
        .await
        .expect("runner run");
    assert!(run.completed);

    let verify_event = state
        .events
        .append(NewEvent {
            session_id: session_id.clone(),
            event_type: EventType::Misc,
            source: "phase1_stable_50step".to_string(),
            data: json!({
                "kind":"verifier_request",
                "trigger":"TaskComplete",
                "final_message_call_id":"phase1-stable-final"
            }),
        })
        .await
        .expect("append verifier request");

    let req = VerifyRequest {
        session_id: session_id.clone(),
        trigger: VerifyTrigger::TaskComplete {
            final_message_call_id: "phase1-stable-final".to_string(),
        },
        triggered_at_event_id: verify_event.id as u64,
    };

    let deps = WorkerDeps::from_router(
        &state.router,
        state.plan_manager.clone(),
        state.events.clone(),
        state.sandbox.clone(),
        state.verifications.clone(),
        state.cost.clone(),
        state.verifier_system_prompt.clone(),
        state.cancel_tokens.clone(),
    );
    let worker = Worker::new(deps);
    let _verification_id = worker.handle_request(&req).await.expect("worker request");

    let after_worker = state
        .events
        .query(
            &session_id,
            EventQuery {
                limit: Some(1000),
                ..EventQuery::default()
            },
        )
        .await
        .expect("events after worker");
    if !after_worker.iter().any(|e| {
        e.event_type == EventType::Misc
            && e.data.get("kind").and_then(Value::as_str) == Some("verifier_verdict")
    }) {
        for e in after_worker
            .iter()
            .filter(|e| e.event_type == EventType::Misc)
        {
            eprintln!("misc after worker: {}", e.data);
        }
    }

    let gate = VerifierGate::new(state.db.clone(), state.events.clone(), state.runner.clone());
    let _ = gate.poll_once(0).await.expect("gate poll");

    let st = session_state(&state.db, &session_id).await;
    assert_eq!(st, "FINISHED");

    let rows = state
        .events
        .query(
            &session_id,
            EventQuery {
                limit: Some(1000),
                ..EventQuery::default()
            },
        )
        .await
        .expect("events");
    assert!(!rows.iter().any(|e| {
        e.event_type == EventType::Misc
            && matches!(
                e.data.get("kind").and_then(Value::as_str),
                Some("stuck_terminate" | "max_steps_reached" | "cost_cap")
            )
    }));

    let verdicts: Vec<_> = rows
        .iter()
        .filter(|e| {
            e.event_type == EventType::Misc
                && e.data.get("kind").and_then(Value::as_str) == Some("verifier_verdict")
                && e.data.get("trigger_kind").and_then(Value::as_str) == Some("TaskComplete")
        })
        .collect();
    if verdicts.len() != 1
        || verdicts[0].data.get("verdict").and_then(Value::as_str) != Some("pass")
    {
        for row in rows.iter().filter(|e| {
            e.event_type == EventType::Misc
                && e.data.get("kind").and_then(Value::as_str) == Some("verifier_verdict")
        }) {
            eprintln!("verifier_verdict row: {}", row.data);
        }
    }
    assert_eq!(
        verdicts.len(),
        1,
        "must have exactly one TaskComplete pass verdict"
    );
    assert_eq!(
        verdicts[0].data.get("verdict").and_then(Value::as_str),
        Some("pass")
    );

    if smoke {
        assert!(started.elapsed() < Duration::from_secs(600));
    }

    drop(temp);
}

async fn start_mock_bifrost() -> Option<String> {
    let mut scripted = Vec::new();
    scripted.push(planner_completion());
    for i in 1..=50 {
        scripted.push(tool_completion(
            &format!("call-{i}"),
            "message_notify_user",
            json!({"content": format!("step {i} complete")}),
        ));
    }
    scripted.push(tool_completion("call-final", "idle", json!({})));
    scripted.push(verifier_pass_completion());

    let state = MockBifrost {
        scripted: Arc::new(Mutex::new(VecDeque::from(scripted))),
    };
    let app = Router::new()
        .route("/v1/chat/completions", post(chat_completions))
        .route("/cost", get(cost))
        .with_state(state);

    let listener = {
        let mut picked = None;
        for port in 41000..41100 {
            if let Ok(l) = std::net::TcpListener::bind(("127.0.0.1", port)) {
                l.set_nonblocking(true).expect("nonblocking");
                picked = Some(TcpListener::from_std(l).expect("from_std"));
                break;
            }
        }
        picked?
    };
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("mock serve");
    });
    Some(format!("http://{addr}/v1"))
}

async fn chat_completions(State(state): State<MockBifrost>) -> Json<Value> {
    let mut guard = state.scripted.lock().await;
    let next = guard.pop_front().unwrap_or_else(|| {
        json!({
            "id":"cmpl-exhausted",
            "object":"chat.completion",
            "model":"fallback",
            "choices":[{"index":0,"finish_reason":"stop","message":{"role":"assistant","content":"{\"verdict\":\"pass\",\"reason\":\"fallback\",\"evidence_event_ids\":[],\"suggested_plan_update\":null}","tool_calls":null}}],
            "usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}
        })
    });
    Json(next)
}

async fn cost() -> Json<Value> {
    Json(json!({"total_cents": 0}))
}

fn planner_completion() -> Value {
    json!({
        "id":"cmpl-planner",
        "object":"chat.completion",
        "model":"planner",
        "choices":[{"index":0,"finish_reason":"stop","message":{"role":"assistant","content":"{\"goal\":\"long run\",\"phases\":[{\"id\":1,\"title\":\"Plan\",\"capabilities\":[]},{\"id\":2,\"title\":\"Execute\",\"capabilities\":[]}]}","tool_calls":null}}],
        "usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}
    })
}

fn tool_completion(call_id: &str, tool: &str, args: Value) -> Value {
    json!({
        "id":"cmpl-main",
        "object":"chat.completion",
        "model":"agent-primary",
        "choices":[{
            "index":0,
            "finish_reason":"tool_calls",
            "message":{
                "role":"assistant",
                "content":null,
                "tool_calls":[{
                    "id": call_id,
                    "type":"function",
                    "function":{"name": tool, "arguments": args.to_string()}
                }]
            }
        }],
        "usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}
    })
}

fn verifier_pass_completion() -> Value {
    json!({
        "id": "cmpl-verifier",
        "object": "chat.completion",
        "model": "verifier-secondary",
        "choices": [{
            "index": 0,
            "finish_reason": "stop",
            "message": {
                "role": "assistant",
                "content": "{\"verdict\":\"pass\",\"reason\":\"task done\",\"evidence_event_ids\":[],\"suggested_plan_update\":null}",
                "tool_calls": null
            }
        }],
        "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2}
    })
}

async fn insert_session(db: &seasoned_hand_core::db::DbPool, session_id: &str) {
    let sid = session_id.to_string();
    db.with_conn(move |conn| {
        conn.execute(
            "INSERT INTO sessions (id, created_at, updated_at, state) VALUES (?, 0, 0, 'IDLE')",
            rusqlite::params![sid],
        )
        .unwrap();
    })
    .await;
}

async fn session_state(db: &seasoned_hand_core::db::DbPool, session_id: &str) -> String {
    let sid = session_id.to_string();
    db.with_conn(move |conn| {
        conn.query_row(
            "SELECT state FROM sessions WHERE id = ?",
            rusqlite::params![sid],
            |row| row.get::<_, String>(0),
        )
        .unwrap()
    })
    .await
}
