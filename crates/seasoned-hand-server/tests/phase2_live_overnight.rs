//! Story 2.26 — Phase 2 live-LLM `phase2-live-overnight` workflow_dispatch
//! smoke test. Mirrors the shape of `phase2_overnight.rs` (story 2.25) but
//! swaps the wiremocked Bifrost + sandbox out for the real ones running on
//! the CI runner.
//!
//! Always `#[ignore]`'d; runs in CI only via the `phase2-live-overnight`
//! workflow_dispatch job (see `.github/workflows/ci.yml`). The job sets
//! `SEASONED_HAND_PHASE2_SMOKE=1` + `ANTHROPIC_API_KEY` + `OPENAI_API_KEY`,
//! spins Bifrost on `127.0.0.1:4000`, then runs:
//!
//! ```text
//! cargo test -p seasoned-hand-server --test phase2_live_overnight -- --ignored --nocapture
//! ```
//!
//! The test:
//!   1. Boots an in-process `AppState` + axum server on a random local port.
//!   2. Sends `task_create` over a short-lived WebSocket (one cmd per
//!      connection avoids the 30 s server-side ping/pong timeout during
//!      the long polling waits later in the test).
//!   3. Polls the events store for the `briefing_pending` Misc, then
//!      sends `briefing_confirm { action: "confirm" }` so the runner
//!      starts without waiting on the 5-minute auto-confirm timer.
//!   4. Once the worker has fired a few real tool calls, sends
//!      `task_pause { durable: true }`, hard-kills the docker container
//!      backing the session via `docker rm -f`, drops the in-memory
//!      handle, then sends `task_resume` so the production rebuild path
//!      (story 2.16) runs end-to-end.
//!   5. Waits until the task transitions to a terminal status
//!      (`Completed` / `Failed`) within the 30-minute CI budget.
//!   6. Manually triggers `Worker::handle_request` for the TaskComplete
//!      verifier verdict so the assertion bar mirrors story 2.25.
//!   7. Asserts: deliverable row exists with `format == "docx"`; provenance
//!      manifest deserializes into the §2.11 schema and the required fields
//!      land; no `cost_cap` / `max_steps_reached` / `stuck_terminate` Misc
//!      events; at least one TaskComplete pass verdict.
//!   8. Prints the required summary line so the CI run page surfaces it.
//!
//! refs: /specs/phase-2/stories/story-2.26.md
//! refs: /specs/phase-2/architecture.md §11 ("E2E live-LLM workflow_dispatch")

use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use seasoned_hand_core::events::{EventQuery, EventStore, EventType, NewEvent};
use seasoned_hand_core::router::SlotRouter;
use seasoned_hand_core::sandbox::SandboxClient;
use seasoned_hand_core::search::{SearchClient, SearchProvider};
use seasoned_hand_core::verifier::{
    VerifyRequest, VerifyTrigger, Worker, WorkerDeps, gate::VerifierGate,
};
use seasoned_hand_core::{db, pubsub};
use seasoned_hand_server::{AppState, app};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message;

/// Brief the live runner executes. Uses `shell_exec` (proves the sandbox
/// API works) + produces a `.docx` (exercises the pandoc renderer
/// toolchain the §2.6 install bootstrap puts in place) + `task_deliver`
/// (proves the LLM tool catalog). Content is intentionally trivial —
/// we are testing the FLOW, not LLM-output quality.
const BRIEF: &str = "Generate a 3-page markdown summary of the following git log. \
                     Run `git -C /workspace log --oneline | head -50` and produce \
                     summary.docx with paragraphs grouping commits by week.";

/// Real-clock budget (CI workflow timeout is 30 minutes; leave headroom).
const TEST_TIMEOUT: Duration = Duration::from_secs(28 * 60);

/// `cost_cap_cents` for the runner. $1.50 cap matches §11's per-task
/// budget so a runaway LLM loop fails CI cleanly instead of burning
/// provider credit.
const COST_CAP_CENTS: u32 = 150;
const MAX_STEPS: u32 = 80;

/// How many Action events must land before we trigger the durable pause +
/// rebuild leg. Picked to give the LLM enough breathing room to be deep
/// in the task without bumping into the runner's natural finish.
const PAUSE_AFTER_ACTIONS: usize = 4;

#[tokio::test]
#[ignore = "live LLM smoke; run with SEASONED_HAND_PHASE2_SMOKE=1 + ANTHROPIC_API_KEY + OPENAI_API_KEY"]
async fn phase2_live_overnight() {
    if std::env::var("SEASONED_HAND_PHASE2_SMOKE").ok().as_deref() != Some("1") {
        eprintln!("phase2_live_overnight skipped: set SEASONED_HAND_PHASE2_SMOKE=1");
        return;
    }
    if std::env::var("ANTHROPIC_API_KEY")
        .ok()
        .unwrap_or_default()
        .is_empty()
        || std::env::var("OPENAI_API_KEY")
            .ok()
            .unwrap_or_default()
            .is_empty()
    {
        panic!("phase2_live_overnight requires ANTHROPIC_API_KEY + OPENAI_API_KEY");
    }

    let started = Instant::now();
    let (ws_url, state) = boot_live().await;

    // --- task_create ----------------------------------------------------
    let create_ack = send_cmd(
        &ws_url,
        json!({
            "cmd": "task_create",
            "input": BRIEF,
            "max_steps": MAX_STEPS,
            "cost_cap_cents": COST_CAP_CENTS,
        }),
        "phase2-live-create",
    )
    .await
    .expect("task_create ack");
    assert_eq!(create_ack["ok"], true, "task_create ok: {create_ack}");
    let session_id = create_ack["session_id"]
        .as_str()
        .expect("session_id on ack")
        .to_string();

    // --- Wait for briefing_pending and confirm --------------------------
    let (task_id, briefing_call_id) =
        wait_for_briefing(&state, &session_id, Duration::from_secs(120))
            .await
            .expect("briefing_pending Misc");

    let confirm_ack = send_cmd(
        &ws_url,
        json!({
            "cmd": "briefing_confirm",
            "task_id": task_id,
            "in_reply_to_call_id": briefing_call_id,
            "action": "confirm",
        }),
        "phase2-live-confirm",
    )
    .await
    .expect("briefing_confirm ack");
    assert_eq!(
        confirm_ack["ok"], true,
        "briefing_confirm ok: {confirm_ack}"
    );

    // --- Durable pause + container hard-kill + resume -------------------
    // Wait for the worker to fire a few real tool calls so the rebuild
    // path replays a meaningful event-stream window, not an empty one.
    wait_for_action_count(
        &state,
        &session_id,
        PAUSE_AFTER_ACTIONS,
        Duration::from_secs(600),
    )
    .await
    .expect("≥ PAUSE_AFTER_ACTIONS Action events before pause");

    let container_name = seasoned_hand_core::sandbox::container_name(&session_id);

    let pause_ack = send_cmd(
        &ws_url,
        json!({
            "cmd": "task_pause",
            "session_id": session_id,
            "durable": true,
        }),
        "phase2-live-pause",
    )
    .await
    .expect("task_pause ack");
    assert_eq!(pause_ack["ok"], true, "task_pause durable ack: {pause_ack}");

    // Hard-kill the container so the next resume must take the rebuild
    // branch. The in-memory handle still points at the now-dead docker
    // target; clearing it forces `resume_task::get_handle` to return
    // None and the rebuild + replay path to run.
    let kill_status = std::process::Command::new("docker")
        .args(["rm", "-f", &container_name])
        .status()
        .expect("docker rm -f spawn");
    assert!(
        kill_status.success(),
        "docker rm -f {container_name} succeeded"
    );
    state.sandbox.remove_handle_for_test(&session_id).await;

    let resume_ack = send_cmd(
        &ws_url,
        json!({
            "cmd": "task_resume",
            "session_id": session_id,
        }),
        "phase2-live-resume",
    )
    .await
    .expect("task_resume ack");
    assert_eq!(resume_ack["ok"], true, "task_resume ack: {resume_ack}");

    // The rebuild_required Misc lands on the OLD session timeline.
    let old_misc: Vec<Value> = misc_events(&state, &session_id).await;
    assert!(
        old_misc
            .iter()
            .any(|e| e.get("kind").and_then(Value::as_str) == Some("task_resume_rebuild_required")),
        "rebuild_required Misc on old session id={session_id}; events={old_misc:?}"
    );

    // --- Wait for task to reach a terminal status -----------------------
    let remaining = TEST_TIMEOUT.saturating_sub(started.elapsed());
    let terminal = wait_for_task_terminal(&state, &task_id, remaining)
        .await
        .expect("task reaches terminal status within budget");
    assert!(
        matches!(
            terminal,
            seasoned_hand_core::project::TaskStatus::Completed
                | seasoned_hand_core::project::TaskStatus::Failed
        ),
        "expected Completed/Failed, got {terminal:?}"
    );

    // --- Banned Misc kinds across every session for the task ------------
    let all_session_ids: Vec<String> = state
        .db
        .with_conn({
            let tid = task_id.clone();
            move |conn| {
                let mut stmt = conn
                    .prepare("SELECT id FROM sessions WHERE task_id = ? ORDER BY created_at ASC")
                    .expect("prepare sessions select");
                let ids: Vec<String> = stmt
                    .query_map(rusqlite::params![tid], |row| row.get::<_, String>(0))
                    .expect("query_map sessions")
                    .filter_map(Result::ok)
                    .collect();
                ids
            }
        })
        .await;
    assert!(
        all_session_ids.len() >= 2,
        "expected ≥ 2 sessions for task (rebuild produced a fresh session id); got {all_session_ids:?}"
    );

    let banned = ["cost_cap", "max_steps_reached", "stuck_terminate"];
    for sid in &all_session_ids {
        for ev in misc_events(&state, sid).await {
            let kind = ev.get("kind").and_then(Value::as_str).unwrap_or("");
            assert!(
                !banned.contains(&kind),
                "session {sid} emitted banned Misc kind {kind}: {ev}"
            );
        }
    }

    // --- Trigger verifier verdict ---------------------------------------
    // In production the Verifier Worker XREADGROUPs `verify_request` off
    // Redis (spawned by `main.rs`). The in-process test fixture doesn't
    // spawn the Worker, so we manually fire `Worker::handle_request` on
    // the final-session id once the task is terminal. Mirrors the same
    // post-completion verifier kick that `phase2_overnight.rs` (story
    // 2.25) uses.
    let last_session_id = all_session_ids
        .last()
        .cloned()
        .expect("≥ 1 session for task");
    let verifier_event = state
        .events
        .append(NewEvent {
            session_id: last_session_id.clone(),
            event_type: EventType::Misc,
            source: "phase2_live_overnight".into(),
            data: json!({
                "kind": "verifier_request",
                "trigger": "TaskComplete",
                "final_message_call_id": "phase2-live-final",
            }),
        })
        .await
        .expect("append verifier_request");
    let verify_req = VerifyRequest {
        session_id: last_session_id.clone(),
        trigger: VerifyTrigger::TaskComplete {
            final_message_call_id: "phase2-live-final".into(),
        },
        triggered_at_event_id: verifier_event.id as u64,
    };
    let worker_deps = WorkerDeps::from_router(
        &state.router,
        state.plan_manager.clone(),
        state.events.clone(),
        state.sandbox.clone(),
        state.verifications.clone(),
        state.cost.clone(),
        state.verifier_system_prompt.clone(),
        state.cancel_tokens.clone(),
    );
    let worker = Worker::new(worker_deps);
    let _ = worker
        .handle_request(&verify_req)
        .await
        .expect("verifier worker handle_request");

    // VerifierGate is a no-op for FINISHED sessions but kept here for
    // structural parity with the deterministic 2.25 path.
    let gate = VerifierGate::new(state.db.clone(), state.events.clone(), state.runner.clone());
    let _ = gate.poll_once(0).await.expect("gate poll");

    // --- Deliverable + provenance manifest assertions -------------------
    let deliverables = state
        .deliverables
        .list_by_task(&task_id)
        .await
        .expect("list_by_task");
    assert!(
        !deliverables.is_empty(),
        "task produced at least one deliverable"
    );
    let deliverable = deliverables
        .iter()
        .find(|d| d.format == "docx")
        .unwrap_or_else(|| {
            panic!(
                "expected a docx deliverable; got formats: {:?}",
                deliverables
                    .iter()
                    .map(|d| d.format.as_str())
                    .collect::<Vec<_>>()
            )
        });
    assert_eq!(deliverable.format, "docx");
    // DEBT #32 close-out: persisted path is absolute.
    assert!(
        std::path::Path::new(&deliverable.rendered_content_path).is_absolute(),
        "rendered_content_path is absolute: {}",
        deliverable.rendered_content_path
    );

    let manifest = &deliverable.provenance_manifest;
    let schema_version = manifest
        .get("schema_version")
        .and_then(Value::as_i64)
        .expect("manifest.schema_version present");
    assert_eq!(schema_version, 1, "schema_version pins to 1");
    assert!(manifest.get("task_id").is_some(), "manifest.task_id");
    assert!(manifest.get("project_id").is_some(), "manifest.project_id");
    let intake_channel = manifest
        .get("intake")
        .and_then(|i| i.get("channel"))
        .and_then(Value::as_str)
        .expect("manifest.intake.channel present");
    assert_eq!(intake_channel, "chat", "intake.channel == chat (WS path)");
    let sessions = manifest
        .get("sessions")
        .and_then(Value::as_array)
        .expect("manifest.sessions array");
    assert!(!sessions.is_empty(), "manifest.sessions non-empty");
    let tool_calls = manifest
        .get("metrics")
        .and_then(|m| m.get("tool_calls"))
        .and_then(Value::as_i64)
        .expect("metrics.tool_calls present");
    assert!(tool_calls >= 1, "metrics.tool_calls >= 1; got {tool_calls}");
    let sha = manifest
        .get("rendered_content_sha256")
        .and_then(Value::as_str)
        .expect("rendered_content_sha256 present");
    assert_eq!(
        sha.len(),
        64,
        "rendered_content_sha256 is 64 hex chars: {sha}"
    );

    // --- ≥ 1 TaskComplete pass verdict ----------------------------------
    let verdicts: Vec<Value> = misc_events(&state, &last_session_id)
        .await
        .into_iter()
        .filter(|e| {
            e.get("kind").and_then(Value::as_str) == Some("verifier_verdict")
                && e.get("trigger_kind").and_then(Value::as_str) == Some("TaskComplete")
        })
        .collect();
    assert!(
        verdicts
            .iter()
            .any(|v| v.get("verdict").and_then(Value::as_str) == Some("pass")),
        "≥ 1 TaskComplete pass verdict; verdicts were: {verdicts:?}"
    );

    let wall_seconds = started.elapsed().as_secs();
    println!(
        "phase2 smoke pass: task_id={task_id} deliverable_format={} wall_seconds={wall_seconds}",
        deliverable.format
    );
}

async fn boot_live() -> (String, AppState) {
    let bifrost_base =
        std::env::var("BIFROST_BASE_URL").unwrap_or_else(|_| "http://127.0.0.1:4000/v1".into());
    let redis_url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".into());

    let pool = db::open(":memory:").await.expect("db");
    let redis = pubsub::RedisPool::new(redis_url).expect("redis");
    // Real workspace root — production sandbox bootstrap writes the
    // `init` commit + the renderer toolchain install lands here.
    let workspace_root = tempfile::tempdir().expect("workspace tempdir");
    let sandbox = SandboxClient::new(
        "ghcr.io/agent-infra/sandbox:1.0.0.152",
        workspace_root.path(),
    )
    .expect("sandbox client");
    let search = SearchClient::new(SearchProvider::Brave { api_key: None });
    // Verifier slot deliberately differs from main so `Worker::handle_request`
    // hits a distinct Bifrost alias (gpt-4o via openai), preventing
    // the §2.7 "verifier ≠ main" startup gate from collapsing the two.
    let router = SlotRouter::from_yaml_str(&format!(
        r#"
slots:
  main:
    provider: bifrost
    model: agent-primary
    base_url: {bifrost_base}
  planner:
    provider: bifrost
    model: agent-primary
    base_url: {bifrost_base}
  verifier:
    provider: bifrost
    model: agent-fallback
    base_url: {bifrost_base}
"#
    ))
    .expect("router");

    let state = AppState::new(pool, redis, sandbox, search, router, Default::default())
        .with_verifier_prompt(Arc::new(
            std::fs::read_to_string("config/prompts/verifier.system.txt")
                .unwrap_or_else(|_| "You are verifier.".to_string()),
        ));

    // Leak the tempdir so the workspace survives until process exit
    // (the in-process server holds the path inside SandboxClient).
    let _ = workspace_root.keep();

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let serve_state = state.clone();
    tokio::spawn(async move {
        axum::serve(listener, app(serve_state))
            .await
            .expect("serve");
    });
    (format!("ws://{addr}/ws"), state)
}

/// Open a short-lived WS, send a single command envelope, drain to the
/// matching Ack, return it. Short-lived so the 30 s server-side
/// ping/pong heartbeat never times out during long inter-cmd polling.
async fn send_cmd(ws_url: &str, payload: Value, cmd_id: &str) -> Result<Value, String> {
    let (mut ws, _) = tokio_tungstenite::connect_async(ws_url)
        .await
        .map_err(|e| format!("connect: {e}"))?;
    let envelope = json!({
        "type": "command",
        "id": cmd_id,
        "ts": 0,
        "payload": payload,
    });
    ws.send(Message::Text(envelope.to_string()))
        .await
        .map_err(|e| format!("send: {e}"))?;
    // Drain until we see the Ack with `ref == cmd_id`. Skip pings /
    // events; respond to pings so the server doesn't trip its
    // pong-timeout in the read window.
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        let msg = tokio::time::timeout(Duration::from_secs(5), ws.next())
            .await
            .map_err(|_| "recv timeout".to_string())?
            .ok_or_else(|| "ws closed unexpectedly".to_string())?
            .map_err(|e| format!("recv: {e}"))?;
        let text = match msg {
            Message::Text(t) => t,
            _ => continue,
        };
        let value: Value = serde_json::from_str(&text).map_err(|e| format!("parse: {e}"))?;
        match value.get("type").and_then(Value::as_str) {
            Some("ping") => {
                let ts = value.get("ts").and_then(Value::as_i64).unwrap_or(0);
                let _ = ws
                    .send(Message::Text(json!({"type":"pong","ts":ts}).to_string()))
                    .await;
            }
            Some("event") => continue,
            Some("ack") => {
                if value.get("ref").and_then(Value::as_str) == Some(cmd_id) {
                    let _ = ws.close(None).await;
                    return Ok(value);
                }
            }
            _ => continue,
        }
    }
    Err(format!("no ack for cmd_id={cmd_id} within deadline"))
}

async fn wait_for_briefing(
    state: &AppState,
    session_id: &str,
    timeout: Duration,
) -> Result<(String, String), String> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        let rows = state
            .events
            .query(
                session_id,
                EventQuery {
                    event_type: Some(EventType::Misc),
                    ..EventQuery::default()
                },
            )
            .await
            .map_err(|e| e.to_string())?;
        for ev in rows {
            if ev.data.get("kind").and_then(Value::as_str) == Some("briefing_pending") {
                let call_id = ev
                    .data
                    .get("briefing_call_id")
                    .and_then(Value::as_str)
                    .ok_or("briefing_pending missing briefing_call_id")?
                    .to_string();
                let task_id = ev
                    .data
                    .get("task_id")
                    .and_then(Value::as_str)
                    .ok_or("briefing_pending missing task_id")?
                    .to_string();
                return Ok((task_id, call_id));
            }
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    Err(format!(
        "briefing_pending Misc not seen on session {session_id} within {timeout:?}"
    ))
}

async fn wait_for_action_count(
    state: &AppState,
    session_id: &str,
    target: usize,
    timeout: Duration,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        let rows = state
            .events
            .query(
                session_id,
                EventQuery {
                    event_type: Some(EventType::Action),
                    ..EventQuery::default()
                },
            )
            .await
            .map_err(|e| e.to_string())?;
        if rows.len() >= target {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    Err(format!(
        "session {session_id} did not reach {target} Action events within {timeout:?}"
    ))
}

async fn wait_for_task_terminal(
    state: &AppState,
    task_id: &str,
    timeout: Duration,
) -> Result<seasoned_hand_core::project::TaskStatus, String> {
    use seasoned_hand_core::project::TaskStatus;
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        let task = state.tasks.get(task_id).await.map_err(|e| e.to_string())?;
        match task.status {
            TaskStatus::Completed | TaskStatus::Failed | TaskStatus::Cancelled => {
                return Ok(task.status);
            }
            _ => {}
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    Err(format!(
        "task {task_id} did not reach terminal status within {timeout:?}"
    ))
}

async fn misc_events(state: &AppState, session_id: &str) -> Vec<Value> {
    state
        .events
        .query(
            session_id,
            EventQuery {
                limit: Some(5000),
                ..EventQuery::default()
            },
        )
        .await
        .expect("events query")
        .into_iter()
        .filter(|e| e.event_type == EventType::Misc)
        .map(|e| e.data)
        .collect()
}
