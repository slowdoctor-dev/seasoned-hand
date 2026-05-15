//! Story 2.25 — Phase 2 deterministic E2E (overnight workflow).
//!
//! Acceptance gate test for Phase 2 on the default `cargo test --workspace`
//! path. Drives the full "Do this overnight" flow end-to-end against
//! wiremock'd Bifrost + sandbox `/v1/shell/exec`, in-memory SQLite, and
//! the [`RecordingTransport`] / [`MockMailbox`] in-tree fakes from
//! story 2.11.
//!
//! Acceptance criteria mirror `/specs/phase-2/stories/story-2.25.md`:
//! 1. Email-shaped IntakeEvent → Task drafted (via
//!    `intake_router.handle_event`; we bypass the IMAP fixture path and
//!    push the synthetic event directly — the 2.11 unit tests already
//!    cover the IMAP-parse leg, and this story is testing orchestration).
//! 2. Briefing confirm gate emits + auto-receives `Confirm` via
//!    `briefing_senders` → Task → `running`.
//! 3. Worker runs ≥50 scripted tool iterations split across two sessions
//!    (10 on session-A, 40+ on session-B after rebuild) without
//!    `stuck_terminate` / `max_steps_reached` / `cost_cap`.
//! 4. Mid-run durable pause + handle drop + resume via
//!    [`task::resume_task`] produces a new session id; the
//!    `task_resume_rebuild_required` Misc lands on the old session
//!    timeline before the rebuild starts.
//! 5. Worker calls `task_deliver` with a `.docx` target. The wiremock'd
//!    sandbox `/v1/shell/exec` parses the pandoc command and writes a
//!    sentinel docx into the workspace_host_path so
//!    `fingerprint_artifact` reads non-empty bytes back.
//! 6. `Deliverable` row persists with `format == "docx"`.
//! 7. Email delivery via `delivery_router.deliver_task`: the
//!    [`RecordingTransport`] captures exactly one outbound message; its
//!    `In-Reply-To` matches the synthetic intake's `Message-ID` and the
//!    attachment filename ends in `.docx`.
//! 8. Verifier verdict: exactly one `verifier_verdict` Misc with
//!    `trigger_kind == "TaskComplete"` and `verdict == "pass"`.
//! 9. Provenance manifest is diffed (after redacting the volatile id /
//!    timestamp keys) against
//!    `tests/fixtures/phase2_overnight/expected_provenance.json`.
//!
//! Default-path wall-clock budget: <30s. Under `SEASONED_HAND_PHASE2_SMOKE=1`
//! a 600 s assert mirrors the Phase 1 pattern. Spec divergence: the
//! story-spec line about a wiremock'd `async-imap` fixture is dropped
//! in favour of pushing a synthetic IntakeEvent directly through
//! `intake_router.handle_event` — the `MockMailbox` + `RecordingTransport`
//! seams established in 2.11 are the in-tree way to exercise this; the
//! IMAP-parse leg is covered by its own unit tests in
//! `crates/seasoned-hand-core/src/channel/email/tests.rs`.
//!
//! refs: /specs/phase-2/stories/story-2.25.md
//! refs: /specs/phase-2/architecture.md §11 ("Acceptance gate")

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::{Json, Router, extract::State, routing::get, routing::post};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::sync::Mutex;

use seasoned_hand_core::agent::RunRequest;
use seasoned_hand_core::agent::init::briefing::{BriefingAction, UserResponse};
use seasoned_hand_core::channel::email::{
    AllowList, CHANNEL_NAME as EMAIL_CHANNEL, EmailChannel, MockMailbox, RecordingTransport,
};
use seasoned_hand_core::channel::{ChannelRegistration, DeliveryTarget, IntakeEvent};
use seasoned_hand_core::db;
use seasoned_hand_core::events::{EventQuery, EventStore, EventType, NewEvent};
use seasoned_hand_core::intake::HandleOutcome;
use seasoned_hand_core::project::TaskStatus;
use seasoned_hand_core::pubsub;
use seasoned_hand_core::router::SlotRouter;
use seasoned_hand_core::sandbox::{SandboxClient, SandboxError, SandboxHandle};
use seasoned_hand_core::search::{SearchClient, SearchProvider};
use seasoned_hand_core::task::{
    ResumeDeps, ResumeOutcome, SandboxOps, replay::WorkspaceWriter, resume_task,
};
use seasoned_hand_core::verifier::{
    VerifyRequest, VerifyTrigger, Worker, WorkerDeps, gate::VerifierGate,
};
use seasoned_hand_server::AppState;

/// One iteration of the worker loop on session-A before the
/// `message_ask_user` self-suspends it. Keep this modest — the
/// fixture redaction step skips Action event ids, but a longer A
/// run just inflates `tool_calls` in the provenance metrics.
const SESSION_A_ITERATIONS: usize = 10;

/// Worker iterations on session-B (post-rebuild) BEFORE `task_deliver`
/// fires. Total tool calls ≥ 50 once `task_deliver` + `idle` are added.
const SESSION_B_ITERATIONS: usize = 41;

/// Synthetic Message-ID stamped on the inbound mail and round-tripped
/// through `reply_target` → `In-Reply-To` on the captured reply.
const INBOUND_MSGID: &str = "abc123@phase2-overnight.example";

/// Target filename the worker hands `task_deliver`. The pandoc
/// renderer would normally produce real bytes via sandbox shell-exec;
/// the wiremock'd sandbox writes a 1 KB sentinel docx instead.
const TARGET_FILENAME: &str = "phase2-summary.docx";

/// Single recipient address — matches the test intake's `From` header.
const OPERATOR_EMAIL: &str = "operator@phase2-overnight.example";

#[tokio::test]
async fn phase2_overnight_default_path() {
    let smoke = std::env::var("SEASONED_HAND_PHASE2_SMOKE").ok().as_deref() == Some("1");
    let started = Instant::now();

    // Renderer install runs ~30-60 s of `apt-get install -y pandoc ...`
    // commands against the sandbox API on real `SandboxClient::create`.
    // Tests skip the install — we never call `create()`; the env var is
    // belt-and-braces.
    // SAFETY: tests run sequentially on the default `cargo test`
    // path when no `--test-threads=N` is set, and we own the process
    // env. The cargo runner spawns separate processes per test binary,
    // so concurrent test binaries can't race on this var.
    unsafe {
        std::env::set_var(
            seasoned_hand_core::sandbox::bootstrap::SKIP_INSTALL_ENV,
            "1",
        );
    }

    // --- Wiremock'd Bifrost: scripted FIFO of completions ---------------
    let bifrost = start_mock_bifrost().await.expect("bifrost mock bind");

    // --- Wiremock'd sandbox /v1/shell/exec ------------------------------
    let workspace_root = tempfile::tempdir().expect("workspace tempdir");
    let sandbox_workspace_root: Arc<PathBuf> = Arc::new(workspace_root.path().to_path_buf());
    let sandbox_api = start_mock_sandbox(sandbox_workspace_root.clone())
        .await
        .expect("sandbox mock bind");

    // --- AppState -------------------------------------------------------
    let pool = db::open(":memory:").await.expect("db");
    // Unreachable redis placeholder. NotifyWorker / IntakeRouter / etc.
    // are NOT spawned by AppState::new (main.rs does that), so nothing
    // in this test ever attempts XREADGROUP / pubsub against it; the
    // mark_task_complete / hook-emit paths fail-soft into tracing.
    let redis = pubsub::RedisPool::new("redis://127.0.0.1:6").expect("redis url");
    let sandbox_client =
        SandboxClient::new("ghcr.io/agent-infra/sandbox:test", workspace_root.path())
            .expect("sandbox client");
    let search = SearchClient::new(SearchProvider::Brave { api_key: None });
    let router = SlotRouter::from_yaml_str(&format!(
        r#"
slots:
  main:
    provider: bifrost
    model: claude-sonnet-4-6
    base_url: {bifrost}
  planner:
    provider: bifrost
    model: planner-primary
    base_url: {bifrost}
  verifier:
    provider: bifrost
    model: gpt-4o
    base_url: {bifrost}
"#
    ))
    .expect("router");

    let mut state = AppState::new(
        pool.clone(),
        redis,
        sandbox_client,
        search,
        router,
        Default::default(),
    );

    // --- Register an EmailChannel built with the test seams -------------
    let mailbox = Arc::new(MockMailbox::new());
    let transport = Arc::new(RecordingTransport::new());
    let email_channel = Arc::new(
        EmailChannel::builder()
            .fetcher(mailbox.clone())
            .transport(transport.clone())
            .from_address("Seasoned Hand <bot@phase2-overnight.example>".to_string())
            .allow_list(AllowList::parse(OPERATOR_EMAIL).expect("allow-list"))
            .poll_interval(Duration::from_secs(60))
            .build()
            .expect("email channel"),
    );
    state = state.register_channel(
        ChannelRegistration::new(EMAIL_CHANNEL)
            .with_intake(email_channel.clone())
            .with_delivery(email_channel.clone())
            .with_notify(email_channel),
    );

    // --- Push the synthetic intake event --------------------------------
    let intake_metadata = json!({
        "from": OPERATOR_EMAIL,
        "subject": "[sh] Summarize the Q4 deck",
        "message_id": format!("<{INBOUND_MSGID}>"),
    });
    let reply_target = DeliveryTarget {
        channel: EMAIL_CHANNEL.into(),
        target_ref: format!("msgid:<{INBOUND_MSGID}>"),
        metadata: json!({
            "to": OPERATOR_EMAIL,
            "subject": "[sh] Summarize the Q4 deck",
        }),
    };
    let intake_event = IntakeEvent {
        channel: EMAIL_CHANNEL.into(),
        intake_id: format!("imap:phase2-overnight-{INBOUND_MSGID}"),
        brief_input: "Summarize the Q4 deck and reply with a docx.".into(),
        reply_target: Some(reply_target.clone()),
        metadata: intake_metadata,
        tenant_id: None,
        received_at: now_micros(),
    };

    let outcome = state
        .intake_router
        .handle_event(intake_event)
        .await
        .expect("handle_event ok");
    let (task_id, session_a_id) = match outcome {
        HandleOutcome::Created {
            task_id,
            session_id,
            ..
        } => (
            task_id,
            session_id.expect("spawner attached → session_id present"),
        ),
        other => panic!("expected Created, got {other:?}"),
    };

    // Sanity: task starts in `drafted` (Initializer hasn't yet
    // confirmed). Pull immediately — the spawner's task hasn't had a
    // chance to advance to Briefed yet because we haven't ticked
    // the runtime.
    let initial_status = state.tasks.get(&task_id).await.expect("task get").status;
    assert!(
        matches!(
            initial_status,
            TaskStatus::Drafted | TaskStatus::Briefed | TaskStatus::Confirmed
        ),
        "task should be drafted/briefing right after intake; got {initial_status:?}"
    );

    // Pre-register the session-A workspace handle pointing at our
    // wiremock'd sandbox API. The spawner only inserts a sessions row;
    // the SandboxClient handle cache is the test's job.
    register_handle(
        &state.sandbox,
        &session_a_id,
        &sandbox_workspace_root,
        &sandbox_api,
    )
    .await;

    // --- Briefing confirm: forward via the registered sender ------------
    // The spawner inserted the mpsc sender BEFORE tokio::spawn, so a
    // tight poll catches it on the very first iteration in practice.
    // Cap at 5 s to keep the failure-mode timely.
    let sender = wait_for_briefing_sender(&state, &task_id, Duration::from_secs(5))
        .await
        .expect("briefing_senders gains task_id entry");
    sender
        .send(UserResponse {
            in_reply_to_call_id: String::new(),
            action: BriefingAction::Confirm,
        })
        .await
        .expect("forward Confirm");

    // --- Wait for session-A to suspend at `message_ask_user` ------------
    // The scripted Bifrost queue ends session-A's iterations with
    // `message_ask_user` (see start_mock_bifrost) — the runner sets
    // session state to SUSPENDED + returns; the spawned task ends here.
    wait_for_session_state(&state, &session_a_id, "SUSPENDED", Duration::from_secs(15))
        .await
        .expect("session-A reaches SUSPENDED");

    // --- Simulate durable pause -----------------------------------------
    // The WS `task_pause { durable: true }` handler calls
    // `sandbox.pause()` (bollard) which fails without docker; we emit
    // the same Misc events + walk the task / session state machines by
    // hand. AC #1 (landmine #2 in story prompt) — auto-confirm is
    // exercised at the unit-test layer in
    // `crates/seasoned-hand-core/src/agent/init/tests.rs::briefing_auto_confirm_after_timeout`.
    let session_a_cursor = latest_event_id(&pool, &session_a_id).await;
    state
        .events
        .append(NewEvent {
            session_id: session_a_id.clone(),
            event_type: EventType::Misc,
            source: "phase2_overnight".into(),
            data: json!({
                "kind": "task_paused_durable",
                "sandbox_id": "test-container",
                "workspace_path": sandbox_workspace_root.join(&session_a_id).display().to_string(),
                "event_cursor": session_a_cursor,
                "paused_at": now_micros() / 1_000_000,
            }),
        })
        .await
        .expect("task_paused_durable Misc");
    state
        .events
        .append(NewEvent {
            session_id: session_a_id.clone(),
            event_type: EventType::Misc,
            source: "phase2_overnight".into(),
            data: json!({"kind": "task_paused"}),
        })
        .await
        .expect("task_paused Misc");
    state
        .tasks
        .set_status(&task_id, TaskStatus::Paused)
        .await
        .expect("task → Paused");

    // --- Resume via the rebuild path ------------------------------------
    // The adapter intercepts `get_handle` to lie about session-A being
    // gone (forces the rebuild branch) and `create_handle` to inject a
    // fresh handle into the SHARED SandboxClient cache so the subsequent
    // worker / renderer paths see it without docker.
    let adapter = DroppedSandboxAdapter {
        inner: state.sandbox.clone(),
        dropped: Arc::new(Mutex::new([session_a_id.clone()].into_iter().collect())),
        workspace_root: sandbox_workspace_root.clone(),
        api_url: sandbox_api.clone(),
    };

    let resume_outcome = resume_task(
        &task_id,
        ResumeDeps {
            task_store: state.tasks.as_ref(),
            events: state.events.as_ref(),
            plan_manager: state.plan_manager.as_ref(),
            sandbox: &adapter,
            db: &state.db,
        },
    )
    .await
    .expect("resume_task ok");
    let session_b_id = match resume_outcome {
        ResumeOutcome::Rebuilt {
            old_session_id,
            new_session_id,
        } => {
            assert_eq!(old_session_id, session_a_id);
            assert_ne!(new_session_id, session_a_id, "fresh session id");
            new_session_id
        }
        ResumeOutcome::UnpausedExisting { .. } => {
            panic!("expected Rebuilt; adapter should have hidden the handle")
        }
    };

    // `task_resume_rebuild_required` lands on the OLD session timeline.
    let old_misc: Vec<Value> = misc_events(&state, &session_a_id).await;
    assert!(
        old_misc
            .iter()
            .any(|e| e.get("kind").and_then(Value::as_str) == Some("task_resume_rebuild_required")),
        "rebuild_required misc on old session; events were: {old_misc:?}"
    );
    // Task is back to Running (resume_task transitions it).
    assert_eq!(
        state.tasks.get(&task_id).await.expect("task").status,
        TaskStatus::Running
    );

    // --- Continue the worker loop on session-B --------------------------
    let _ = state
        .runner
        .resume(RunRequest {
            session_id: session_b_id.clone(),
            input: "execute overnight workflow".into(),
            max_steps: 80,
            cost_cap_cents: Some(50),
        })
        .await
        .expect("runner resume session-B");

    // After `idle`, the runner sets FINISHED (verifier disabled by
    // YAML — see phase1_stable_50step for the same shape) and returns.
    let st_b = session_state(&pool, &session_b_id).await;
    assert_eq!(st_b.as_deref(), Some("FINISHED"), "session-B FINISHED");

    // --- Trigger the verifier verdict (TaskComplete) --------------------
    let verifier_request_event = state
        .events
        .append(NewEvent {
            session_id: session_b_id.clone(),
            event_type: EventType::Misc,
            source: "phase2_overnight".into(),
            data: json!({
                "kind": "verifier_request",
                "trigger": "TaskComplete",
                "final_message_call_id": "phase2-overnight-final",
            }),
        })
        .await
        .expect("verifier_request append");
    let verify_req = VerifyRequest {
        session_id: session_b_id.clone(),
        trigger: VerifyTrigger::TaskComplete {
            final_message_call_id: "phase2-overnight-final".into(),
        },
        triggered_at_event_id: verifier_request_event.id as u64,
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
    let _verification_id = worker
        .handle_request(&verify_req)
        .await
        .expect("worker handle_request");

    // VerifierGate transitions VERIFYING → FINISHED if any session is
    // VERIFYING. Session-B is already FINISHED (verifier disabled
    // skipped mark_task_complete) so this is a no-op poll — kept for
    // structural parity with phase1_stable_50step.
    let gate = VerifierGate::new(state.db.clone(), state.events.clone(), state.runner.clone());
    let _ = gate.poll_once(0).await.expect("gate poll");

    // --- Trigger delivery via the DeliveryRouter ------------------------
    // DEBT #32 closed in story 2.26: `task_deliver` now persists the
    // absolute on-disk path (workspace_host_path + relative tail), so
    // `EmailChannel::deliver`'s naïve `tokio::fs::read(...)` succeeds
    // without the per-test canonicalize workaround.
    let receipt = state
        .delivery_router
        .deliver_task(&task_id)
        .await
        .expect("delivery_router.deliver_task");
    assert_eq!(receipt.channel, EMAIL_CHANNEL);
    assert!(receipt.ok, "delivery row marked ok");

    // --- Assertions: events, deliverable, email, provenance -------------
    let new_misc: Vec<Value> = misc_events(&state, &session_b_id).await;
    let banned = ["stuck_terminate", "max_steps_reached", "cost_cap"];
    for ev in &new_misc {
        let kind = ev.get("kind").and_then(Value::as_str).unwrap_or("");
        assert!(
            !banned.contains(&kind),
            "session-B emitted a banned misc kind {kind}: {ev}"
        );
    }
    // Same check on session-A (durable-pause emits are explicitly allowed).
    for ev in &old_misc {
        let kind = ev.get("kind").and_then(Value::as_str).unwrap_or("");
        assert!(
            !banned.contains(&kind),
            "session-A emitted a banned misc kind {kind}: {ev}"
        );
    }

    // Verifier verdict — exactly one TaskComplete pass.
    let verdicts: Vec<Value> = new_misc
        .iter()
        .filter(|e| {
            e.get("kind").and_then(Value::as_str) == Some("verifier_verdict")
                && e.get("trigger_kind").and_then(Value::as_str) == Some("TaskComplete")
        })
        .cloned()
        .collect();
    assert_eq!(verdicts.len(), 1, "exactly one verdict, got {verdicts:?}");
    assert_eq!(
        verdicts[0].get("verdict").and_then(Value::as_str),
        Some("pass"),
        "verdict was: {:?}",
        verdicts[0]
    );

    // Deliverable persisted with format=docx.
    let deliverables = state
        .deliverables
        .list_by_task(&task_id)
        .await
        .expect("list_by_task");
    assert_eq!(deliverables.len(), 1, "exactly one deliverable");
    let deliverable = &deliverables[0];
    assert_eq!(deliverable.format, "docx", "format docx");
    assert!(
        deliverable
            .rendered_content_path
            .ends_with(&format!("/{TARGET_FILENAME}"))
            || deliverable.rendered_content_path.contains(TARGET_FILENAME),
        "rendered_content_path mentions target filename: {}",
        deliverable.rendered_content_path
    );

    // Email delivery captured exactly one reply with the right
    // In-Reply-To + attachment filename.
    let captured = transport.snapshot().await;
    assert_eq!(captured.len(), 1, "exactly one email captured");
    let raw = String::from_utf8_lossy(&captured[0].formatted()).to_string();
    assert!(
        raw.contains(&format!("In-Reply-To: <{INBOUND_MSGID}>")),
        "missing/incorrect In-Reply-To header in: {raw}"
    );
    assert!(
        raw.contains(&format!("filename=\"{TARGET_FILENAME}\"")),
        "attachment filename not surfaced (looked for {TARGET_FILENAME}) in: {raw}"
    );

    // Provenance manifest: load the persisted Deliverable, redact, diff
    // against the golden fixture. DEBT #24: brief.confirmed/edits_applied
    // are static placeholders in 2.15 — fixture pins them and we assert
    // them explicitly so the next flip is visible.
    let mut actual_manifest: Value = deliverable.provenance_manifest.clone();
    redact_provenance(&mut actual_manifest);
    let expected_text =
        std::fs::read_to_string("tests/fixtures/phase2_overnight/expected_provenance.json")
            .expect("read expected_provenance");
    let expected: Value = serde_json::from_str(&expected_text).expect("parse golden");
    assert_eq!(
        actual_manifest, expected,
        "redacted provenance manifest diverged from golden fixture:\n  actual = {actual_manifest:#?}\n  expected = {expected:#?}"
    );

    if smoke {
        assert!(
            started.elapsed() < Duration::from_secs(600),
            "smoke wall-clock budget exceeded: {:?}",
            started.elapsed()
        );
    }

    drop(workspace_root);
}

// ============================================================================
// Mock Bifrost — scripted chat completions.
// ============================================================================

#[derive(Clone)]
struct MockBifrost {
    scripted: Arc<Mutex<VecDeque<Value>>>,
}

async fn start_mock_bifrost() -> Option<String> {
    let mut scripted: Vec<Value> = Vec::new();
    // 1. Initializer planner-brief response — `author_brief` parses
    //    this as a `Brief` (goal + phases). Validated against
    //    `Brief::validate` so phases must be non-empty + ids ≥ 1.
    scripted.push(planner_brief_completion());
    // 2. ~10 iterations on session-A.
    for i in 1..=SESSION_A_ITERATIONS {
        scripted.push(tool_completion(
            &format!("a-call-{i}"),
            "message_notify_user",
            json!({"content": format!("session-A step {i}")}),
        ));
    }
    // 3. message_ask_user → SUSPENDED → spawned task returns.
    scripted.push(tool_completion(
        "a-call-ask",
        "message_ask_user",
        json!({"content": "Need input — durable pause window."}),
    ));
    // 4. ~41 iterations on session-B (post-rebuild).
    for i in 1..=SESSION_B_ITERATIONS {
        scripted.push(tool_completion(
            &format!("b-call-{i}"),
            "message_notify_user",
            json!({"content": format!("session-B step {i}")}),
        ));
    }
    // 5. task_deliver with a .docx target. citations references event
    //    ids 1+2 which are guaranteed to exist by the time task_deliver
    //    fires (Initializer + worker already wrote events).
    scripted.push(tool_completion(
        "b-call-deliver",
        "task_deliver",
        json!({
            "content": "# Q4 Deck Summary\n\nA short markdown summary.\n",
            "target_filename": TARGET_FILENAME,
            "citations": [1, 2],
        }),
    ));
    // 6. idle — runner sets FINISHED (verifier_enabled=false here).
    scripted.push(tool_completion("b-call-idle", "idle", json!({})));
    // 7. Verifier verdict — popped by Worker::handle_request.
    scripted.push(verifier_pass_completion());

    let state = MockBifrost {
        scripted: Arc::new(Mutex::new(VecDeque::from(scripted))),
    };
    let app = Router::new()
        .route("/v1/chat/completions", post(chat_completions))
        .route("/cost", get(cost))
        .with_state(state);

    let listener = pick_listener(42100, 42200).await?;
    let addr = listener.local_addr().expect("bifrost addr");
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    Some(format!("http://{addr}/v1"))
}

async fn chat_completions(State(state): State<MockBifrost>) -> Json<Value> {
    let mut guard = state.scripted.lock().await;
    let next = guard.pop_front().unwrap_or_else(|| {
        // Fallback verdict shape, matching phase1_stable_50step. Lets
        // the test fail with a clean assertion message rather than a
        // panic from an empty queue.
        json!({
            "id":"cmpl-exhausted",
            "object":"chat.completion",
            "model":"fallback",
            "choices":[{
                "index":0,
                "finish_reason":"stop",
                "message":{
                    "role":"assistant",
                    "content":"{\"verdict\":\"pass\",\"reason\":\"fallback\",\"evidence_event_ids\":[],\"suggested_plan_update\":null}",
                    "tool_calls":null
                }
            }],
            "usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}
        })
    });
    Json(next)
}

async fn cost() -> Json<Value> {
    Json(json!({"total_cents": 0}))
}

fn planner_brief_completion() -> Value {
    // Brief shape (project::brief::Brief) — phases with id ≥ 1 and
    // non-empty titles pass `Brief::validate`.
    json!({
        "id":"cmpl-planner",
        "object":"chat.completion",
        "model":"planner-primary",
        "choices":[{
            "index":0,
            "finish_reason":"stop",
            "message":{
                "role":"assistant",
                "content":"{\"goal\":\"Summarize the Q4 deck overnight\",\"phases\":[{\"id\":1,\"title\":\"Read deck\",\"capabilities\":[]},{\"id\":2,\"title\":\"Compose summary\",\"capabilities\":[]},{\"id\":3,\"title\":\"Deliver docx\",\"capabilities\":[]}]}",
                "tool_calls":null
            }
        }],
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
        "id":"cmpl-verifier",
        "object":"chat.completion",
        "model":"verifier-secondary",
        "choices":[{
            "index":0,
            "finish_reason":"stop",
            "message":{
                "role":"assistant",
                "content":"{\"verdict\":\"pass\",\"reason\":\"phase2 overnight pass\",\"evidence_event_ids\":[],\"suggested_plan_update\":null}",
                "tool_calls":null
            }
        }],
        "usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}
    })
}

// ============================================================================
// Mock sandbox — handles /v1/shell/exec.
// ============================================================================

#[derive(Clone)]
struct MockSandbox {
    workspace_root: Arc<PathBuf>,
}

async fn start_mock_sandbox(workspace_root: Arc<PathBuf>) -> Option<String> {
    let app = Router::new()
        .route("/v1/shell/exec", post(shell_exec))
        .with_state(MockSandbox { workspace_root });
    let listener = pick_listener(42300, 42400).await?;
    let addr = listener.local_addr().expect("sandbox addr");
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    Some(format!("http://{addr}"))
}

async fn shell_exec(State(state): State<MockSandbox>, Json(body): Json<Value>) -> Json<Value> {
    let command = body.get("command").and_then(Value::as_str).unwrap_or("");
    // Parse `pandoc ... -o /workspace/<target> /workspace/<source>` so
    // we can plant a sentinel file on disk for the renderer's
    // fingerprint_artifact step to read back. Matches the shape from
    // `crates/seasoned-hand-core/src/deliverable/renderer/pandoc.rs::pandoc_command`.
    if let Some(target_workspace_relative) = parse_pandoc_target(command) {
        // workspace_root in the test holds the per-session dir at
        // `<root>/<session_id>/...`. The renderer's
        // `SandboxClient::write_workspace_file` puts the source under
        // `<root>/<session_id>/.deliverables/.source/<uuid>.md`, and we
        // need to land the output at
        // `<root>/<session_id>/.deliverables/<target_filename>`.
        // We don't know the session id from the command alone (the
        // pandoc CLI sees `/workspace/.deliverables/...` only), so we
        // walk every per-session dir under workspace_root and write the
        // sentinel under each one — the renderer reads from the right
        // one because the SandboxClient handle resolves
        // `workspace_host_path` per session.
        if let Ok(entries) = std::fs::read_dir(state.workspace_root.as_path()) {
            for entry in entries.flatten() {
                let abs = entry.path().join(&target_workspace_relative);
                if let Some(parent) = abs.parent() {
                    let _ = tokio::fs::create_dir_all(parent).await;
                }
                let _ = tokio::fs::write(&abs, SENTINEL_DOCX_BYTES).await;
            }
        }
    }
    Json(json!({"exit_code": 0, "stdout": "", "stderr": ""}))
}

/// Sentinel docx payload. Real docx is a zip archive starting with the
/// `PK\x03\x04` magic; we use a 1 KB blob with the magic + filler so
/// downstream MIME-sniff checks (if any) don't reject it.
const SENTINEL_DOCX_BYTES: &[u8] = b"PK\x03\x04 phase2-overnight sentinel docx ........................................................................................................................................................................................................................................................................................................................................................................................................................................................................................................................................................................................................................................................................................................................................................................................................................................................";

/// Match `pandoc -f markdown -t docx -o /workspace/.deliverables/X.docx ...`
/// and return `.deliverables/X.docx` (workspace-relative).
fn parse_pandoc_target(cmd: &str) -> Option<String> {
    let mut tokens = cmd.split_whitespace();
    let mut next = tokens.next();
    while let Some(tok) = next {
        if tok == "-o" {
            let path = tokens.next()?;
            return Some(strip_workspace_prefix(path).to_string());
        }
        next = tokens.next();
    }
    None
}

fn strip_workspace_prefix(p: &str) -> &str {
    p.strip_prefix("/workspace/")
        .or_else(|| p.strip_prefix("workspace/"))
        .unwrap_or(p)
}

// ============================================================================
// DroppedSandboxAdapter — lies about session-A's handle so resume_task
// takes the rebuild branch, and routes create_handle into the shared
// SandboxClient cache so post-rebuild renderer / runner calls see the
// new handle without a real docker create.
// ============================================================================

struct DroppedSandboxAdapter {
    inner: Arc<SandboxClient>,
    dropped: Arc<Mutex<std::collections::HashSet<String>>>,
    workspace_root: Arc<PathBuf>,
    api_url: String,
}

impl WorkspaceWriter for DroppedSandboxAdapter {
    async fn write_workspace_file(
        &self,
        session_id: &str,
        relative_path: &str,
        contents: &[u8],
    ) -> Result<(), SandboxError> {
        self.inner
            .write_workspace_file(session_id, relative_path, contents)
            .await
    }
    async fn write_workspace_file_json_value(
        &self,
        session_id: &str,
        relative_path: &str,
        value: &Value,
    ) -> Result<(), SandboxError> {
        self.inner
            .write_workspace_file_json(session_id, relative_path, value)
            .await
    }
}

impl SandboxOps for DroppedSandboxAdapter {
    async fn get_handle(&self, session_id: &str) -> Option<SandboxHandle> {
        if self.dropped.lock().await.contains(session_id) {
            return None;
        }
        self.inner.get(session_id).await
    }

    async fn create_handle(&self, session_id: &str) -> Result<SandboxHandle, SandboxError> {
        let path = self.workspace_root.join(session_id);
        std::fs::create_dir_all(&path).map_err(SandboxError::Io)?;
        let handle = SandboxHandle {
            session_id: session_id.to_string(),
            container_id: format!("rebuilt-{session_id}"),
            api_url: self.api_url.clone(),
            novnc_url: "http://127.0.0.1:0".into(),
            ttyd_url: "ws://127.0.0.1:0".into(),
            workspace_host_path: path,
        };
        self.inner.insert_handle_for_test(handle.clone()).await;
        Ok(handle)
    }

    async fn unpause(&self, _session_id: &str) -> Result<(), SandboxError> {
        Ok(())
    }
}

// ============================================================================
// Helpers.
// ============================================================================

async fn pick_listener(start: u16, end: u16) -> Option<TcpListener> {
    for port in start..end {
        if let Ok(l) = std::net::TcpListener::bind(("127.0.0.1", port)) {
            l.set_nonblocking(true).ok()?;
            return TcpListener::from_std(l).ok();
        }
    }
    None
}

async fn register_handle(
    sandbox: &Arc<SandboxClient>,
    session_id: &str,
    workspace_root: &Path,
    api_url: &str,
) {
    let path = workspace_root.join(session_id);
    std::fs::create_dir_all(&path).expect("session workspace mkdir");
    sandbox
        .insert_handle_for_test(SandboxHandle {
            session_id: session_id.to_string(),
            container_id: format!("test-{session_id}"),
            api_url: api_url.to_string(),
            novnc_url: "http://127.0.0.1:0".into(),
            ttyd_url: "ws://127.0.0.1:0".into(),
            workspace_host_path: path,
        })
        .await;
}

async fn wait_for_briefing_sender(
    state: &AppState,
    task_id: &str,
    timeout: Duration,
) -> Option<tokio::sync::mpsc::Sender<UserResponse>> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if let Some(entry) = state.briefing_senders.get(task_id) {
            return Some(entry.clone());
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    None
}

async fn wait_for_session_state(
    state: &AppState,
    session_id: &str,
    expected: &str,
    timeout: Duration,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    let mut last_seen: Option<String> = None;
    while Instant::now() < deadline {
        let st = session_state(&state.db, session_id).await;
        if st.as_deref() == Some(expected) {
            return Ok(());
        }
        last_seen = st;
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    Err(format!(
        "session {session_id} did not reach {expected}; last_seen={last_seen:?}"
    ))
}

async fn session_state(pool: &seasoned_hand_core::db::DbPool, session_id: &str) -> Option<String> {
    let sid = session_id.to_string();
    pool.with_conn(move |conn| {
        conn.query_row(
            "SELECT state FROM sessions WHERE id = ?",
            rusqlite::params![sid],
            |row| row.get::<_, String>(0),
        )
        .ok()
    })
    .await
}

async fn latest_event_id(pool: &seasoned_hand_core::db::DbPool, session_id: &str) -> i64 {
    let sid = session_id.to_string();
    pool.with_conn(move |conn| {
        conn.query_row(
            "SELECT COALESCE(MAX(id), 0) FROM events WHERE session_id = ?",
            rusqlite::params![sid],
            |row| row.get::<_, i64>(0),
        )
        .unwrap_or(0)
    })
    .await
}

async fn misc_events(state: &AppState, session_id: &str) -> Vec<Value> {
    state
        .events
        .query(
            session_id,
            EventQuery {
                limit: Some(2000),
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

fn now_micros() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_micros() as i64)
        .unwrap_or(0)
}

// ============================================================================
// Provenance redaction.
// ============================================================================

/// Strip every volatile id / timestamp / size / hash so the manifest
/// shape can be field-diffed against the golden fixture. Volatile keys
/// land at well-known positions in the §2.11 schema; we walk the tree
/// once and clear them.
///
/// Kept conservative: anything not redacted MUST be stable across runs.
/// DEBT #24 is the one knob we explicitly assert on (brief.confirmed +
/// edits_applied) — they're placeholders today and the fixture pins
/// them so the next flip surfaces as a test failure.
fn redact_provenance(value: &mut Value) {
    let placeholder_string = Value::String("<redacted>".into());
    let placeholder_id = Value::String("<redacted-id>".into());
    let placeholder_int = Value::Number(0.into());

    if let Some(obj) = value.as_object_mut() {
        obj.insert("task_id".into(), placeholder_id.clone());
        obj.insert("project_id".into(), placeholder_id.clone());
        obj.insert("rendered_content_sha256".into(), placeholder_string.clone());
        if obj.get("source_content_sha256").is_some() {
            obj.insert("source_content_sha256".into(), placeholder_string.clone());
        }

        if let Some(intake) = obj.get_mut("intake").and_then(Value::as_object_mut)
            && let Some(rcvd) = intake.get_mut("received_at")
        {
            *rcvd = placeholder_int.clone();
        }

        if let Some(brief) = obj.get_mut("brief").and_then(Value::as_object_mut) {
            // brief_event_id is the FIRST `briefing` Misc id — varies
            // with how many events landed before the gate fired.
            if brief.get("brief_event_id").is_some() {
                brief.insert("brief_event_id".into(), placeholder_int.clone());
            }
        }

        if let Some(sessions) = obj.get_mut("sessions").and_then(Value::as_array_mut) {
            for s in sessions.iter_mut() {
                if let Some(o) = s.as_object_mut() {
                    o.insert("id".into(), placeholder_id.clone());
                    o.insert("started_at".into(), placeholder_int.clone());
                    if o.get("ended_at").is_some() {
                        o.insert("ended_at".into(), placeholder_int.clone());
                    }
                }
            }
        }

        // `decisions` is an array of event ids; we don't emit any
        // `decision` Misc in this test, so it's [] — leave as is so a
        // future regression that DOES emit one fails loudly.

        if let Some(verdicts) = obj
            .get_mut("verifier_verdicts")
            .and_then(Value::as_array_mut)
        {
            for v in verdicts.iter_mut() {
                *v = placeholder_id.clone();
            }
        }

        if let Some(metrics) = obj.get_mut("metrics").and_then(Value::as_object_mut) {
            // tool_calls / cost_cents / wall_seconds drift between runs;
            // sessions_count / pause_resume_cycles / verifier_runs stay
            // stable but we redact wall_seconds defensively.
            metrics.insert("tool_calls".into(), placeholder_int.clone());
            metrics.insert("cost_cents".into(), placeholder_int.clone());
            metrics.insert("wall_seconds".into(), placeholder_int.clone());
        }

        if let Some(delivered) = obj.get_mut("delivered_to").and_then(Value::as_array_mut) {
            for d in delivered.iter_mut() {
                if let Some(o) = d.as_object_mut() {
                    o.insert("delivery_id".into(), placeholder_id.clone());
                    o.insert("delivered_at".into(), placeholder_int.clone());
                    if o.get("external_id").is_some() {
                        o.insert("external_id".into(), placeholder_string.clone());
                    }
                }
            }
        }
    }
}
