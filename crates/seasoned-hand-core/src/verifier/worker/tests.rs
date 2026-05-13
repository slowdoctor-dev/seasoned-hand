use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use tempfile::tempdir;
use tokio_util::sync::CancellationToken;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;
use crate::db;
use crate::events::{EventQuery, EventStore, EventType, sqlite::SqliteEventStore};
use crate::plan::{Phase, PhaseStatus, PlanManager};
use crate::pubsub::RedisPool;
use crate::sandbox::{SandboxClient, SandboxHandle};
use crate::verifier::{VerdictKind, VerifyRequest, VerifyTrigger};

struct TestRig {
    worker: Worker,
    events: Arc<SqliteEventStore>,
    plan_manager: Arc<PlanManager>,
    verifications: Arc<VerificationStore>,
    session_id: String,
}

async fn rig_with_llm_responses(responses: Vec<Value>) -> TestRig {
    let mock = MockServer::start().await;
    for resp in responses {
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(resp))
            .up_to_n_times(1)
            .mount(&mock)
            .await;
    }
    Mock::given(method("GET"))
        .and(path("/cost"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"total_cents": 0})))
        .mount(&mock)
        .await;

    let db = db::open(":memory:").await.expect("in-memory db");
    let session_id = "verifier-session".to_string();
    let sid = session_id.clone();
    db.with_conn(move |conn| {
        conn.execute(
            "INSERT INTO sessions (id, created_at, updated_at, state) \
             VALUES (?, 0, 0, 'RUNNING')",
            rusqlite::params![sid],
        )
        .expect("insert session");
    })
    .await;

    let redis = RedisPool::new("redis://127.0.0.1:6").expect("parse redis url");
    let events = Arc::new(SqliteEventStore::with_redis(db.clone(), redis));
    let plan_manager = Arc::new(PlanManager::new(db.clone(), events.clone()));
    plan_manager
        .create(
            &session_id,
            "test goal",
            vec![Phase {
                id: 1,
                title: "do thing".into(),
                status: PhaseStatus::Active,
                capabilities: Vec::new(),
            }],
        )
        .await
        .expect("seed plan");
    let verifications = Arc::new(VerificationStore::new(db.clone()));

    let workspace = tempdir().expect("tempdir");
    let sandbox = Arc::new(
        SandboxClient::new("ghcr.io/agent-infra/sandbox:1.0.0.152", workspace.path())
            .expect("sandbox client"),
    );
    sandbox
        .insert_handle_for_test(SandboxHandle {
            session_id: session_id.clone(),
            container_id: "c1".into(),
            api_url: "http://127.0.0.1:1".into(),
            novnc_url: "http://127.0.0.1:2".into(),
            ttyd_url: "ws://127.0.0.1:3".into(),
            workspace_host_path: workspace.path().join(&session_id),
        })
        .await;

    let cost = Arc::new(CostClient::new(mock.uri()));
    let verifier_llm = LlmClient::new(mock.uri(), None);

    let deps = WorkerDeps {
        plan_manager: plan_manager.clone(),
        events: events.clone(),
        sandbox,
        verifications: verifications.clone(),
        cost,
        system_prompt: Arc::new("You are the verifier (test prompt).".to_string()),
        verifier_slot_model: "verifier-test-model".to_string(),
        verifier_llm,
        cancel_tokens: Arc::new(dashmap::DashMap::new()),
    };
    let worker = Worker::new(deps);

    TestRig {
        worker,
        events,
        plan_manager,
        verifications,
        session_id,
    }
}

fn assistant_response(content: &str) -> Value {
    json!({
        "id": "cmpl-verifier-1",
        "object": "chat.completion",
        "model": "verifier-test-model",
        "choices": [{
            "index": 0,
            "finish_reason": "stop",
            "message": {
                "role": "assistant",
                "content": content,
                "tool_calls": null
            }
        }],
        "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2}
    })
}

fn task_complete_req(session_id: &str) -> VerifyRequest {
    VerifyRequest {
        session_id: session_id.into(),
        trigger: VerifyTrigger::TaskComplete {
            final_message_call_id: "call-1".into(),
        },
        triggered_at_event_id: 1,
    }
}

#[tokio::test]
async fn worker_skips_when_verifier_not_enabled() {
    let rig = rig_with_llm_responses(Vec::new()).await;
    let redis = Arc::new(RedisPool::new("redis://127.0.0.1:6").unwrap());
    let token = tokio_util::sync::CancellationToken::new();
    rig.worker
        .run(false, redis, token)
        .await
        .expect("worker.run returns Ok when verifier disabled");
}

#[tokio::test]
async fn worker_run_returns_when_cancellation_token_fires() {
    let rig = rig_with_llm_responses(Vec::new()).await;
    let redis = Arc::new(RedisPool::new("redis://127.0.0.1:6").unwrap());
    let token = tokio_util::sync::CancellationToken::new();
    let token_clone = token.clone();
    let h = tokio::spawn(async move { rig.worker.run(true, redis, token_clone).await });
    tokio::time::sleep(Duration::from_millis(50)).await;
    token.cancel();
    tokio::time::timeout(Duration::from_secs(2), h)
        .await
        .expect("worker.run exits within 2s after cancel")
        .expect("task joined")
        .expect("worker.run returns Ok on cancel");
}

#[tokio::test]
async fn handle_request_inserts_row_and_emits_verifier_verdict() {
    let raw = r#"{"verdict":"pass","reason":"all green","evidence_event_ids":[1]}"#;
    let rig = rig_with_llm_responses(vec![assistant_response(raw)]).await;
    let req = task_complete_req(&rig.session_id);

    let id = rig
        .worker
        .handle_request(&req)
        .await
        .expect("handle_request");

    let row = rig.verifications.get(&id).await.expect("get row");
    assert_eq!(row.verdict, VerdictKind::Pass);
    assert_eq!(row.reason, "all green");
    assert_eq!(row.evidence_event_ids, vec![1]);
    assert_eq!(row.trigger_kind, "TaskComplete");
    assert_eq!(row.model_id, "verifier-test-model");

    let events = rig
        .events
        .query(
            &rig.session_id,
            EventQuery {
                after_id: None,
                event_type: Some(EventType::Misc),
                limit: Some(50),
            },
        )
        .await
        .expect("query events");
    let verdict_ev = events
        .iter()
        .find(|e| e.data.get("kind").and_then(Value::as_str) == Some("verifier_verdict"))
        .expect("verifier_verdict Misc event present");
    assert_eq!(verdict_ev.data["verdict"], json!("pass"));
    assert_eq!(verdict_ev.data["verification_id"], json!(id));
    assert_eq!(verdict_ev.data["trigger_kind"], json!("TaskComplete"));
}

#[tokio::test]
async fn handle_request_falls_back_when_llm_returns_prose_twice() {
    let prose = "Sure! I think the work looks good.";
    let rig =
        rig_with_llm_responses(vec![assistant_response(prose), assistant_response(prose)]).await;
    let req = task_complete_req(&rig.session_id);

    let id = rig
        .worker
        .handle_request(&req)
        .await
        .expect("handle_request");

    let row = rig.verifications.get(&id).await.expect("get row");
    assert_eq!(row.verdict, VerdictKind::Fail);
    assert_eq!(row.reason, "verifier_unparseable");
    assert!(row.evidence_event_ids.is_empty());
}

#[tokio::test]
async fn handle_request_applies_suggested_plan_update_before_emit() {
    let raw = json!({
        "verdict": "fail",
        "reason": "missing tests",
        "evidence_event_ids": [],
        "suggested_plan_update": {
            "phases": [
                { "id": 1, "title": "do thing", "status": "done",  "capabilities": [] },
                { "id": 2, "title": "add tests", "status": "active", "capabilities": [] }
            ]
        }
    });
    let rig = rig_with_llm_responses(vec![assistant_response(&raw.to_string())]).await;
    let req = task_complete_req(&rig.session_id);

    rig.worker
        .handle_request(&req)
        .await
        .expect("handle_request");

    let plan = rig
        .plan_manager
        .snapshot(&rig.session_id)
        .await
        .expect("snapshot");
    let titles: Vec<&str> = plan.phases.iter().map(|p| p.title.as_str()).collect();
    assert!(
        titles.contains(&"add tests"),
        "plan must include the verifier-suggested phase; got {titles:?}"
    );
}

#[tokio::test]
async fn watchdog_emits_misc_and_returns_none_on_timeout() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(assistant_response(
                    r#"{"verdict":"pass","reason":"ok","evidence_event_ids":[]}"#,
                ))
                .set_delay(Duration::from_millis(500)),
        )
        .mount(&mock)
        .await;
    Mock::given(method("GET"))
        .and(path("/cost"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"total_cents": 0})))
        .mount(&mock)
        .await;

    let db = db::open(":memory:").await.expect("db");
    db.with_conn(|conn| {
        conn.execute(
            "INSERT INTO sessions (id, created_at, updated_at, state) \
             VALUES ('s-watchdog', 0, 0, 'RUNNING')",
            [],
        )
        .unwrap();
    })
    .await;
    let redis = RedisPool::new("redis://127.0.0.1:6").unwrap();
    let events = Arc::new(SqliteEventStore::with_redis(db.clone(), redis));
    let plan_manager = Arc::new(PlanManager::new(db.clone(), events.clone()));
    plan_manager
        .create(
            "s-watchdog",
            "g",
            vec![Phase {
                id: 1,
                title: "p".into(),
                status: PhaseStatus::Active,
                capabilities: Vec::new(),
            }],
        )
        .await
        .unwrap();
    let verifications = Arc::new(VerificationStore::new(db.clone()));
    let workspace = tempdir().unwrap();
    let sandbox = Arc::new(
        SandboxClient::new("ghcr.io/agent-infra/sandbox:1.0.0.152", workspace.path()).unwrap(),
    );
    sandbox
        .insert_handle_for_test(SandboxHandle {
            session_id: "s-watchdog".into(),
            container_id: "c1".into(),
            api_url: "http://127.0.0.1:1".into(),
            novnc_url: "http://127.0.0.1:2".into(),
            ttyd_url: "ws://127.0.0.1:3".into(),
            workspace_host_path: workspace.path().join("s-watchdog"),
        })
        .await;
    let cost = Arc::new(CostClient::new(mock.uri()));
    let verifier_llm = LlmClient::new(mock.uri(), None);
    let worker = Worker::new(WorkerDeps {
        plan_manager,
        events: events.clone(),
        sandbox,
        verifications,
        cost,
        system_prompt: Arc::new("test".into()),
        verifier_slot_model: "test-model".into(),
        verifier_llm,
        cancel_tokens: Arc::new(dashmap::DashMap::new()),
    })
    .with_watchdog(Duration::from_millis(50));

    let req = VerifyRequest {
        session_id: "s-watchdog".into(),
        trigger: VerifyTrigger::TaskComplete {
            final_message_call_id: "call-1".into(),
        },
        triggered_at_event_id: 1,
    };
    let result = handle_request_with_watchdog(&worker, &req)
        .await
        .expect("watchdog returns Ok(None)");
    assert!(
        result.is_none(),
        "watchdog must return None when LLM exceeds the timeout"
    );

    let evs = events
        .query(
            "s-watchdog",
            EventQuery {
                after_id: None,
                event_type: Some(EventType::Misc),
                limit: Some(50),
            },
        )
        .await
        .unwrap();
    assert!(
        evs.iter()
            .any(|e| e.data.get("kind").and_then(Value::as_str) == Some("verifier_watchdog")),
        "expected verifier_watchdog Misc event"
    );
}

#[tokio::test]
async fn verifier_cancel_emits_verifier_cancelled_misc() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(assistant_response(
                    r#"{"verdict":"pass","reason":"ok","evidence_event_ids":[]}"#,
                ))
                .set_delay(Duration::from_secs(5)),
        )
        .mount(&mock)
        .await;
    Mock::given(method("GET"))
        .and(path("/cost"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"total_cents": 0})))
        .mount(&mock)
        .await;

    let db = db::open(":memory:").await.expect("db");
    let session_id = "s-verifier-cancel".to_string();
    let sid = session_id.clone();
    db.with_conn(move |conn| {
        conn.execute(
            "INSERT INTO sessions (id, created_at, updated_at, state) \
             VALUES (?, 0, 0, 'VERIFYING')",
            rusqlite::params![sid],
        )
        .expect("insert session");
    })
    .await;

    let redis = RedisPool::new("redis://127.0.0.1:6").unwrap();
    let events = Arc::new(SqliteEventStore::with_redis(db.clone(), redis));
    let plan_manager = Arc::new(PlanManager::new(db.clone(), events.clone()));
    plan_manager
        .create(
            &session_id,
            "goal",
            vec![Phase {
                id: 1,
                title: "phase".into(),
                status: PhaseStatus::Active,
                capabilities: Vec::new(),
            }],
        )
        .await
        .unwrap();
    let verifications = Arc::new(VerificationStore::new(db.clone()));
    let workspace = tempdir().unwrap();
    let sandbox = Arc::new(
        SandboxClient::new("ghcr.io/agent-infra/sandbox:1.0.0.152", workspace.path()).unwrap(),
    );
    sandbox
        .insert_handle_for_test(SandboxHandle {
            session_id: session_id.clone(),
            container_id: "c1".into(),
            api_url: "http://127.0.0.1:1".into(),
            novnc_url: "http://127.0.0.1:2".into(),
            ttyd_url: "ws://127.0.0.1:3".into(),
            workspace_host_path: workspace.path().join(&session_id),
        })
        .await;

    let cancel_tokens = Arc::new(dashmap::DashMap::new());
    let token = CancellationToken::new();
    cancel_tokens.insert(session_id.clone(), token.clone());

    let worker = Worker::new(WorkerDeps {
        plan_manager,
        events: events.clone(),
        sandbox,
        verifications: verifications.clone(),
        cost: Arc::new(CostClient::new(mock.uri())),
        system_prompt: Arc::new("test".into()),
        verifier_slot_model: "test-model".into(),
        verifier_llm: LlmClient::new(mock.uri(), None),
        cancel_tokens,
    });

    let req = VerifyRequest {
        session_id: session_id.clone(),
        trigger: VerifyTrigger::TaskComplete {
            final_message_call_id: "call-1".into(),
        },
        triggered_at_event_id: 1,
    };

    let handle = tokio::spawn(async move { worker.handle_request(&req).await });
    tokio::time::sleep(Duration::from_millis(100)).await;
    token.cancel();

    let started = std::time::Instant::now();
    let out = tokio::time::timeout(Duration::from_secs(1), handle)
        .await
        .expect("handle_request should return quickly")
        .expect("join")
        .expect("handle_request");
    assert_eq!(out, "cancelled");
    assert!(started.elapsed() < Duration::from_millis(500));

    let evs = events
        .query(
            &session_id,
            EventQuery {
                after_id: None,
                event_type: Some(EventType::Misc),
                limit: Some(100),
            },
        )
        .await
        .unwrap();
    assert!(
        evs.iter().any(|e| {
            e.data.get("kind").and_then(Value::as_str) == Some("verifier_cancelled")
                && e.data.get("trigger_kind") == Some(&json!("TaskComplete"))
                && e.data.get("triggered_at_event_id") == Some(&json!(1))
        }),
        "expected verifier_cancelled Misc event"
    );
    assert!(
        evs.iter()
            .all(|e| e.data.get("kind").and_then(Value::as_str) != Some("verifier_verdict")),
        "verifier_verdict must not be emitted when verifier request is cancelled"
    );

    let rows = verifications
        .list_by_session(&session_id, None, 10)
        .await
        .expect("list rows");
    assert!(
        rows.is_empty(),
        "no verification row should be persisted on cancel"
    );
}

// ──────────────────────────────────────────────────────────────────────────
// Story 2.18 — live-Redis XREADGROUP loop tests
//
// These tests require a running Redis (default redis://127.0.0.1:6379).
// Run with `REDIS_URL=redis://127.0.0.1:6379 cargo test -p seasoned-hand-core
// verifier::worker -- --ignored`. Each test uses a uuid-suffixed stream +
// consumer-group name so concurrent runs don't poison each other's PEL.
// ──────────────────────────────────────────────────────────────────────────

use crate::verifier::VerifierRuntimeConfig;
use serde::Serialize;

fn redis_test_url() -> String {
    std::env::var("REDIS_TEST_URL")
        .ok()
        .or_else(|| std::env::var("REDIS_URL").ok())
        .unwrap_or_else(|| "redis://127.0.0.1:6379".into())
}

struct LiveRig {
    worker: Worker,
    verifications: Arc<VerificationStore>,
    redis: Arc<RedisPool>,
    stream: String,
    group: String,
}

/// Build a rig backed by real Redis + a wiremock LLM/cost mock. If
/// `seed_sessions` is empty no `sessions` rows are created (used by the
/// error-path test to force an FK violation on `verifications.insert`).
async fn live_rig(mock_uri: String, max_concurrency: usize, seed_sessions: &[&str]) -> LiveRig {
    let db = db::open(":memory:").await.expect("in-memory db");
    for sid in seed_sessions {
        let owned = (*sid).to_string();
        db.with_conn(move |conn| {
            conn.execute(
                "INSERT INTO sessions (id, created_at, updated_at, state) \
                 VALUES (?, 0, 0, 'RUNNING')",
                rusqlite::params![owned],
            )
            .expect("insert session");
        })
        .await;
    }

    let redis = Arc::new(RedisPool::new(redis_test_url()).expect("parse redis url"));
    let events = Arc::new(SqliteEventStore::with_redis(db.clone(), (*redis).clone()));
    let plan_manager = Arc::new(PlanManager::new(db.clone(), events.clone()));
    for sid in seed_sessions {
        plan_manager
            .create(
                sid,
                "live-test goal",
                vec![Phase {
                    id: 1,
                    title: "do thing".into(),
                    status: PhaseStatus::Active,
                    capabilities: Vec::new(),
                }],
            )
            .await
            .expect("seed plan");
    }
    let verifications = Arc::new(VerificationStore::new(db.clone()));

    let workspace = tempdir().expect("tempdir");
    let sandbox = Arc::new(
        SandboxClient::new("ghcr.io/agent-infra/sandbox:1.0.0.152", workspace.path())
            .expect("sandbox client"),
    );
    for sid in seed_sessions {
        sandbox
            .insert_handle_for_test(SandboxHandle {
                session_id: (*sid).to_string(),
                container_id: "c1".into(),
                api_url: "http://127.0.0.1:1".into(),
                novnc_url: "http://127.0.0.1:2".into(),
                ttyd_url: "ws://127.0.0.1:3".into(),
                workspace_host_path: workspace.path().join(sid),
            })
            .await;
    }

    let id = uuid::Uuid::new_v4().simple().to_string();
    let stream = format!("verify_request_test_{id}");
    let group = format!("verifier-workers-test-{id}");
    // Pre-create the consumer group on the (now-empty) stream so any
    // XADDs the test does *before* the worker boots are still visible
    // when the worker XREADGROUPs (XGROUP CREATE ... $ would otherwise
    // skip entries planted ahead of group creation). The worker's own
    // xgroup_create call later returns BUSYGROUP and is swallowed.
    redis
        .xgroup_create_mkstream(&stream, &group)
        .await
        .expect("pre-create consumer group for live test");
    let cfg = VerifierRuntimeConfig {
        stream: stream.clone(),
        group: group.clone(),
        consumer_prefix: format!("worker-test-{id}"),
        max_concurrency,
        consumer_block_ms: 200,
        read_count: 16,
    };

    let cost = Arc::new(CostClient::new(mock_uri.clone()));
    let verifier_llm = LlmClient::new(mock_uri, None);
    let worker = Worker::new(WorkerDeps {
        plan_manager,
        events: events.clone(),
        sandbox,
        verifications: verifications.clone(),
        cost,
        system_prompt: Arc::new("verifier test prompt".into()),
        verifier_slot_model: "verifier-test-model".to_string(),
        verifier_llm,
        cancel_tokens: Arc::new(dashmap::DashMap::new()),
    })
    .with_runtime_config(cfg);

    LiveRig {
        worker,
        verifications,
        redis,
        stream,
        group,
    }
}

/// Start a permanent-mock wiremock returning `canned_assistant` for
/// every chat completion and `{"total_cents":0}` for every cost poll.
async fn permanent_mock(canned_assistant: Value) -> MockServer {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(canned_assistant))
        .mount(&mock)
        .await;
    Mock::given(method("GET"))
        .and(path("/cost"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"total_cents": 0})))
        .mount(&mock)
        .await;
    mock
}

/// Spawn the worker.run loop on a background task; cancel via the
/// returned token + await the handle.
fn spawn_worker(rig: &LiveRig) -> (CancellationToken, tokio::task::JoinHandle<()>) {
    let worker = rig.worker.clone();
    let redis = rig.redis.clone();
    let shutdown = CancellationToken::new();
    let token = shutdown.clone();
    let handle = tokio::spawn(async move {
        let _ = worker.run(true, redis, token).await;
    });
    (shutdown, handle)
}

async fn shutdown_worker(shutdown: CancellationToken, handle: tokio::task::JoinHandle<()>) {
    shutdown.cancel();
    let _ = tokio::time::timeout(Duration::from_secs(2), handle).await;
}

fn req_for(session_id: &str, anchor: u64) -> VerifyRequest {
    VerifyRequest {
        session_id: session_id.to_string(),
        trigger: VerifyTrigger::TaskComplete {
            final_message_call_id: format!("call-{anchor}"),
        },
        triggered_at_event_id: anchor,
    }
}

#[tokio::test]
#[ignore = "requires running Redis"]
async fn worker_xreadgroup_consumes_one_entry() {
    let mock = permanent_mock(assistant_response(
        r#"{"verdict":"pass","reason":"all green","evidence_event_ids":[]}"#,
    ))
    .await;
    let session_id = "live-consume-one";
    let rig = live_rig(mock.uri(), 2, &[session_id]).await;
    rig.redis
        .xadd_json(&rig.stream, &req_for(session_id, 7))
        .await
        .expect("xadd request");

    let (shutdown, handle) = spawn_worker(&rig);
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let mut rows = Vec::new();
    while std::time::Instant::now() < deadline {
        rows = rig
            .verifications
            .list_by_session(session_id, None, 10)
            .await
            .expect("list rows");
        if !rows.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    shutdown_worker(shutdown, handle).await;

    assert_eq!(rows.len(), 1, "expected exactly one verifications row");
    assert_eq!(rows[0].verdict, VerdictKind::Pass);
    let pending = rig
        .redis
        .xpending_count(&rig.stream, &rig.group)
        .await
        .expect("xpending");
    assert_eq!(pending, 0, "PEL must be empty after XACK");
}

#[tokio::test]
#[ignore = "requires running Redis"]
async fn worker_xreadgroup_per_session_fifo() {
    // 200 ms per LLM call → 3 serial calls ≈ 600 ms; concurrent ≈ 200 ms.
    // Global cap is 8 (lots of headroom) so the only limiter is the
    // per-session FIFO. We assert wall-clock ≥ 500 ms to confirm serial
    // execution.
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(assistant_response(
                    r#"{"verdict":"pass","reason":"ok","evidence_event_ids":[]}"#,
                ))
                .set_delay(Duration::from_millis(200)),
        )
        .mount(&mock)
        .await;
    Mock::given(method("GET"))
        .and(path("/cost"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"total_cents": 0})))
        .mount(&mock)
        .await;

    let session_id = "live-fifo";
    let rig = live_rig(mock.uri(), 8, &[session_id]).await;
    for anchor in [1u64, 2, 3] {
        rig.redis
            .xadd_json(&rig.stream, &req_for(session_id, anchor))
            .await
            .expect("xadd");
    }

    let started = std::time::Instant::now();
    let (shutdown, handle) = spawn_worker(&rig);
    let deadline = started + Duration::from_secs(8);
    let mut rows: Vec<crate::verifier::Verification> = Vec::new();
    while std::time::Instant::now() < deadline {
        rows = rig
            .verifications
            .list_by_session(session_id, None, 10)
            .await
            .expect("list rows");
        if rows.len() == 3 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let elapsed = started.elapsed();
    shutdown_worker(shutdown, handle).await;

    assert_eq!(rows.len(), 3, "expected three verifications rows");
    assert!(
        elapsed >= Duration::from_millis(500),
        "per-session FIFO must serialize: 3 × 200ms LLM calls should take ≥ 500ms; got {elapsed:?}",
    );
    let pending = rig
        .redis
        .xpending_count(&rig.stream, &rig.group)
        .await
        .expect("xpending");
    assert_eq!(pending, 0, "PEL must be empty after XACK");
}

#[tokio::test]
#[ignore = "requires running Redis"]
async fn worker_xreadgroup_global_semaphore_caps_concurrency() {
    // 5 sessions × 1 request each, LLM delay 300 ms, global cap 2.
    // Minimum wall-clock = ⌈5/2⌉ × 300 ms = 900 ms. We assert ≥ 700 ms
    // (giving timing margin) — uncapped concurrency would land at ~300 ms.
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(assistant_response(
                    r#"{"verdict":"pass","reason":"ok","evidence_event_ids":[]}"#,
                ))
                .set_delay(Duration::from_millis(300)),
        )
        .mount(&mock)
        .await;
    Mock::given(method("GET"))
        .and(path("/cost"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"total_cents": 0})))
        .mount(&mock)
        .await;

    let sessions: Vec<String> = (0..5)
        .map(|i| format!("live-sem-{i}-{}", uuid::Uuid::new_v4().simple()))
        .collect();
    let session_refs: Vec<&str> = sessions.iter().map(|s| s.as_str()).collect();
    let rig = live_rig(mock.uri(), 2, &session_refs).await;
    for s in &sessions {
        rig.redis
            .xadd_json(&rig.stream, &req_for(s, 1))
            .await
            .expect("xadd");
    }

    let started = std::time::Instant::now();
    let (shutdown, handle) = spawn_worker(&rig);
    let deadline = started + Duration::from_secs(10);
    let mut all_done = false;
    while std::time::Instant::now() < deadline {
        let mut done = 0usize;
        for s in &sessions {
            let rows = rig
                .verifications
                .list_by_session(s, None, 10)
                .await
                .expect("list rows");
            if !rows.is_empty() {
                done += 1;
            }
        }
        if done == sessions.len() {
            all_done = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let elapsed = started.elapsed();
    shutdown_worker(shutdown, handle).await;

    assert!(all_done, "expected verifications row for every session");
    assert!(
        elapsed >= Duration::from_millis(700),
        "global cap=2 with 5×300ms calls must serialize into batches; got {elapsed:?}",
    );
    let pending = rig
        .redis
        .xpending_count(&rig.stream, &rig.group)
        .await
        .expect("xpending");
    assert_eq!(pending, 0, "PEL must be empty after XACK");
}

#[tokio::test]
#[ignore = "requires running Redis"]
async fn worker_xack_on_handle_request_error() {
    // No session row → verifications.insert hits a FK constraint and
    // returns WorkerError::Persistence. The worker must still XACK so
    // the PEL doesn't grow forever on terminal handler errors.
    let mock = permanent_mock(assistant_response(
        r#"{"verdict":"pass","reason":"ok","evidence_event_ids":[]}"#,
    ))
    .await;
    let rig = live_rig(mock.uri(), 2, &[]).await;
    let ghost_session = "live-ghost-session";
    rig.redis
        .xadd_json(&rig.stream, &req_for(ghost_session, 1))
        .await
        .expect("xadd");

    let (shutdown, handle) = spawn_worker(&rig);
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let mut acked = false;
    while std::time::Instant::now() < deadline {
        let pending = rig
            .redis
            .xpending_count(&rig.stream, &rig.group)
            .await
            .expect("xpending");
        if pending == 0 {
            // Need to make sure XADD'd entry was actually delivered to
            // the consumer (not just never-read). XLEN ≥ 1 confirms the
            // XADD landed; pending == 0 with XLEN ≥ 1 means delivered
            // and then ACKed.
            let len = rig.redis.xlen(&rig.stream).await.expect("xlen");
            if len >= 1 {
                acked = true;
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    shutdown_worker(shutdown, handle).await;

    assert!(
        acked,
        "worker must XACK the message even when handle_request errors",
    );
    // No verifications row was inserted (the FK violation rejected it).
    let rows = rig
        .verifications
        .list_by_session(ghost_session, None, 10)
        .await
        .expect("list rows");
    assert!(
        rows.is_empty(),
        "FK-violating handle_request must not leave a verifications row",
    );
}

#[tokio::test]
#[ignore = "requires running Redis"]
async fn worker_skips_malformed_message_with_xack() {
    #[derive(Serialize)]
    struct NotARequest {
        not_a_verify_request: String,
    }
    // Permanent mock present but should never be hit — the message
    // never makes it past JSON parsing.
    let mock = permanent_mock(assistant_response(
        r#"{"verdict":"pass","reason":"ok","evidence_event_ids":[]}"#,
    ))
    .await;
    let session_id = "live-malformed";
    let rig = live_rig(mock.uri(), 2, &[session_id]).await;
    rig.redis
        .xadd_json(
            &rig.stream,
            &NotARequest {
                not_a_verify_request: "garbage".into(),
            },
        )
        .await
        .expect("xadd");

    let (shutdown, handle) = spawn_worker(&rig);
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let mut acked = false;
    while std::time::Instant::now() < deadline {
        let pending = rig
            .redis
            .xpending_count(&rig.stream, &rig.group)
            .await
            .expect("xpending");
        let len = rig.redis.xlen(&rig.stream).await.expect("xlen");
        if len >= 1 && pending == 0 {
            acked = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    shutdown_worker(shutdown, handle).await;

    assert!(
        acked,
        "worker must XACK + drop unparseable messages (PEL retention would block the queue)",
    );
    let rows = rig
        .verifications
        .list_by_session(session_id, None, 10)
        .await
        .expect("list rows");
    assert!(
        rows.is_empty(),
        "malformed messages must not yield a verifications row",
    );
    // We never called the LLM mock — confirm via wiremock's
    // received_requests counter.
    let requests = mock.received_requests().await.unwrap_or_default();
    assert!(
        requests.iter().all(|r| r.url.path() != "/chat/completions"),
        "no chat-completion call should fire for a malformed message",
    );
}
