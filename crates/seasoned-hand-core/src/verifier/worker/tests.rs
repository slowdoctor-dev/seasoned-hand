use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use tempfile::tempdir;
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
