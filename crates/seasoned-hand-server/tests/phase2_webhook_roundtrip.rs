//! Story 2.26 — Phase 2 `phase2-live-webhook-roundtrip` workflow_dispatch
//! smoke test. Drives a real "webhook intake → email delivery" round-trip
//! against live Bifrost + live SMTP + live IMAP.
//!
//! Always `#[ignore]`'d; runs in CI only via the
//! `phase2-live-webhook-roundtrip` workflow_dispatch job (see
//! `.github/workflows/ci.yml`). The job sets `SEASONED_HAND_PHASE2_SMOKE=1`,
//! the `ANTHROPIC_API_KEY` / `OPENAI_API_KEY` provider keys, the SMTP envs
//! (`SMTP_HOST` / `SMTP_USERNAME` / `SMTP_PASSWORD`), and the matching IMAP
//! envs (`IMAP_HOST` / `IMAP_USERNAME` / `IMAP_PASSWORD`). Skipped when any
//! of those are absent.
//!
//! Spec divergence from `/specs/phase-2/stories/story-2.26.md` (§"AC 2"):
//! the story spec mentions both "asserts the webhook callback URL received
//! a POST" AND "verify a real email arrived in the test mailbox". Those
//! describe two distinct channel pairings — webhook-intake/webhook-delivery
//! vs webhook-intake/email-delivery. Architecture §11 names this job
//! "webhook intake → email delivery", so this test takes the email-delivery
//! reading. The spec is updated to match.
//!
//! refs: /specs/phase-2/stories/story-2.26.md
//! refs: /specs/phase-2/architecture.md §11 ("E2E live-LLM workflow_dispatch")

use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use seasoned_hand_core::channel::email::{
    AllowList, AsyncImapFetcher, CHANNEL_NAME as EMAIL_CHANNEL, EmailChannel, ImapConfig,
    LettreSmtpTransport, MailboxFetcher, MockMailbox, SmtpConfig,
};
use seasoned_hand_core::channel::{ChannelRegistration, webhook::CHANNEL_NAME as WEBHOOK_CHANNEL};
use seasoned_hand_core::events::{EventQuery, EventStore, EventType, NewEvent};
use seasoned_hand_core::router::SlotRouter;
use seasoned_hand_core::sandbox::SandboxClient;
use seasoned_hand_core::search::{SearchClient, SearchProvider};
use seasoned_hand_core::verifier::{VerifyRequest, VerifyTrigger, Worker, WorkerDeps};
use seasoned_hand_core::{db, pubsub};
use seasoned_hand_server::{AppState, app};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message;

/// Brief the live runner executes. Same shape as the overnight smoke
/// brief — produces a docx via pandoc — so the renderer-toolchain
/// install bootstrap is exercised end-to-end here too.
const BRIEF: &str = "Generate a 1-page markdown summary of the git log. \
                     Run `git -C /workspace log --oneline | head -20` and produce \
                     summary.docx with a short list of commits.";

const COST_CAP_CENTS: u32 = 150;
const MAX_STEPS: u32 = 60;

const TEST_TIMEOUT: Duration = Duration::from_secs(13 * 60);

const INTAKE_TOKEN: &str = "phase2-webhook-roundtrip-token";

#[tokio::test]
#[ignore = "live LLM + SMTP + IMAP smoke; run with SEASONED_HAND_PHASE2_SMOKE=1 + provider keys + SMTP/IMAP envs"]
async fn phase2_webhook_roundtrip() {
    if std::env::var("SEASONED_HAND_PHASE2_SMOKE").ok().as_deref() != Some("1") {
        eprintln!("phase2_webhook_roundtrip skipped: set SEASONED_HAND_PHASE2_SMOKE=1");
        return;
    }
    for var in ["ANTHROPIC_API_KEY", "OPENAI_API_KEY"] {
        if std::env::var(var).ok().unwrap_or_default().is_empty() {
            panic!("phase2_webhook_roundtrip requires {var}");
        }
    }
    let smtp_host = std::env::var("SMTP_HOST").ok().unwrap_or_default();
    let smtp_user = std::env::var("SMTP_USERNAME").ok().unwrap_or_default();
    let smtp_pass = std::env::var("SMTP_PASSWORD").ok().unwrap_or_default();
    let imap_host = std::env::var("IMAP_HOST").ok().unwrap_or_default();
    let imap_user = std::env::var("IMAP_USERNAME").ok().unwrap_or_default();
    let imap_pass = std::env::var("IMAP_PASSWORD").ok().unwrap_or_default();
    if smtp_host.is_empty()
        || smtp_user.is_empty()
        || smtp_pass.is_empty()
        || imap_host.is_empty()
        || imap_user.is_empty()
        || imap_pass.is_empty()
    {
        eprintln!(
            "phase2_webhook_roundtrip skipped: missing SMTP_HOST/USERNAME/PASSWORD + IMAP_HOST/USERNAME/PASSWORD"
        );
        return;
    }

    let started = Instant::now();
    let smtp_port: u16 = std::env::var("SMTP_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(587);
    let imap_port: u16 = std::env::var("IMAP_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(993);
    let mailbox_address = smtp_user.clone();

    // --- Boot server with webhook + email (delivery only) registered ---
    let (base_url, ws_url, state) = boot_live(BootArgs {
        smtp: SmtpConfig {
            host: smtp_host.clone(),
            port: smtp_port,
            username: smtp_user.clone(),
            password: smtp_pass.clone(),
        },
        from_address: mailbox_address.clone(),
    })
    .await;

    // --- POST /v1/intake/webhook ---------------------------------------
    let intake_url = format!("{base_url}/v1/intake/webhook");
    let unique_marker = format!("phase2-roundtrip-{}", uuid::Uuid::new_v4());
    let payload = json!({
        "brief": BRIEF,
        "reply_target": {
            "channel": EMAIL_CHANNEL,
            "target_ref": format!("webhook:{unique_marker}"),
            "metadata": {
                "to": mailbox_address,
                "subject": format!("[sh] {unique_marker}"),
            }
        },
        "metadata": {
            "subject_marker": unique_marker,
            "max_steps": MAX_STEPS,
            "cost_cap_cents": COST_CAP_CENTS,
        }
    });
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .expect("reqwest client");
    let resp = client
        .post(&intake_url)
        .header("X-Seasoned-Hand-Intake-Token", INTAKE_TOKEN)
        .json(&payload)
        .send()
        .await
        .expect("POST /v1/intake/webhook");
    assert_eq!(
        resp.status(),
        reqwest::StatusCode::ACCEPTED,
        "webhook intake 202"
    );
    let ack: Value = resp.json().await.expect("ack json");
    let task_id = ack
        .get("task_id")
        .and_then(Value::as_str)
        .expect("ack.task_id")
        .to_string();

    // --- Find the session created for this task ------------------------
    let session_id = wait_for_session(&state, &task_id, Duration::from_secs(30))
        .await
        .expect("session row created for task");

    // --- Wait for briefing_pending + confirm via WS --------------------
    let (briefing_task_id, briefing_call_id) =
        wait_for_briefing(&state, &session_id, Duration::from_secs(120))
            .await
            .expect("briefing_pending Misc");
    assert_eq!(briefing_task_id, task_id);

    let confirm_ack = send_cmd(
        &ws_url,
        json!({
            "cmd": "briefing_confirm",
            "task_id": task_id,
            "in_reply_to_call_id": briefing_call_id,
            "action": "confirm",
        }),
        "phase2-webhook-confirm",
    )
    .await
    .expect("briefing_confirm ack");
    assert_eq!(confirm_ack["ok"], true, "briefing_confirm: {confirm_ack}");

    // --- Wait for terminal status --------------------------------------
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

    // --- Trigger verifier verdict + delivery ---------------------------
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
    let last_session_id = all_session_ids
        .last()
        .cloned()
        .expect("≥ 1 session for task");

    let verifier_event = state
        .events
        .append(NewEvent {
            session_id: last_session_id.clone(),
            event_type: EventType::Misc,
            source: "phase2_webhook_roundtrip".into(),
            data: json!({
                "kind": "verifier_request",
                "trigger": "TaskComplete",
                "final_message_call_id": "phase2-webhook-final",
            }),
        })
        .await
        .expect("append verifier_request");
    let verify_req = VerifyRequest {
        session_id: last_session_id.clone(),
        trigger: VerifyTrigger::TaskComplete {
            final_message_call_id: "phase2-webhook-final".into(),
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

    // Delivery is wired into the DeliveryRouter; trigger the dispatch
    // explicitly so the assertion bar doesn't depend on the
    // mark_task_complete -> auto-delivery side-chain timing.
    let receipt = state
        .delivery_router
        .deliver_task(&task_id)
        .await
        .expect("delivery_router.deliver_task");
    assert_eq!(
        receipt.channel, EMAIL_CHANNEL,
        "delivered via email channel"
    );
    assert!(receipt.ok, "delivery row marked ok");

    // --- Poll IMAP for the reply --------------------------------------
    let imap = AsyncImapFetcher::new(ImapConfig {
        host: imap_host,
        port: imap_port,
        username: imap_user,
        password: imap_pass,
    });
    let imap_deadline = Instant::now() + Duration::from_secs(180);
    let mut matched_filename: Option<String> = None;
    while Instant::now() < imap_deadline {
        let messages = imap.fetch_unseen().await.unwrap_or_default();
        for raw in messages {
            let body_text = String::from_utf8_lossy(&raw.bytes).to_string();
            if !body_text.contains(&unique_marker) {
                let _ = imap.mark_seen(raw.uid).await;
                continue;
            }
            // Parse the attachment filename from the multipart body.
            // Phase 2 SMTP rendering emits a Content-Disposition header
            // with `filename="..."`; look for the value.
            let filename = extract_attachment_filename(&body_text);
            let _ = imap.mark_seen(raw.uid).await;
            if filename.is_some() {
                matched_filename = filename;
                break;
            }
        }
        if matched_filename.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
    let attachment_filename = matched_filename.expect(
        "no inbound email matched the subject marker — check SMTP delivery / IMAP credentials",
    );
    assert!(
        attachment_filename.ends_with(".docx"),
        "attachment filename ends with .docx: {attachment_filename}"
    );

    // --- Deliverable assertions ----------------------------------------
    let deliverables = state
        .deliverables
        .list_by_task(&task_id)
        .await
        .expect("list_by_task");
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
    assert!(
        std::path::Path::new(&deliverable.rendered_content_path).is_absolute(),
        "rendered_content_path absolute: {}",
        deliverable.rendered_content_path
    );

    let manifest = &deliverable.provenance_manifest;
    let intake_channel = manifest
        .get("intake")
        .and_then(|i| i.get("channel"))
        .and_then(Value::as_str)
        .expect("manifest.intake.channel present");
    assert_eq!(intake_channel, WEBHOOK_CHANNEL, "intake.channel == webhook");
    let delivered_to_channel = manifest
        .get("delivered_to")
        .and_then(Value::as_array)
        .and_then(|arr| arr.first())
        .and_then(|d| d.get("channel"))
        .and_then(Value::as_str)
        .expect("manifest.delivered_to[0].channel present");
    assert_eq!(
        delivered_to_channel, EMAIL_CHANNEL,
        "delivered_to[0].channel == email"
    );

    let wall_seconds = started.elapsed().as_secs();
    println!(
        "phase2 smoke pass: task_id={task_id} deliverable_format={} wall_seconds={wall_seconds}",
        deliverable.format
    );
}

struct BootArgs {
    smtp: SmtpConfig,
    from_address: String,
}

async fn boot_live(args: BootArgs) -> (String, String, AppState) {
    let bifrost_base =
        std::env::var("BIFROST_BASE_URL").unwrap_or_else(|_| "http://127.0.0.1:4000/v1".into());
    let redis_url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".into());

    let pool = db::open(":memory:").await.expect("db");
    let redis = pubsub::RedisPool::new(redis_url).expect("redis");
    let workspace_root = tempfile::tempdir().expect("workspace tempdir");
    let sandbox = SandboxClient::new(
        "ghcr.io/agent-infra/sandbox:1.0.0.152",
        workspace_root.path(),
    )
    .expect("sandbox client");
    let search = SearchClient::new(SearchProvider::Brave { api_key: None });
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

    let mut state = AppState::new(pool, redis, sandbox, search, router, Default::default())
        .with_verifier_prompt(Arc::new(
            std::fs::read_to_string("config/prompts/verifier.system.txt")
                .unwrap_or_else(|_| "You are verifier.".to_string()),
        ));

    // Register the webhook channel with a deterministic intake token so
    // the POST /v1/intake/webhook handler accepts the test's request.
    state = state.register_webhook_channel(Arc::new(INTAKE_TOKEN.to_string()), Vec::new());

    // Register an EmailChannel for DELIVERY ONLY. The intake side uses a
    // `MockMailbox` (empty queue) so the real IMAP mailbox isn't drained
    // by the long-lived intake provider — the test's IMAP poll fetches
    // unseen messages directly outside the AppState.
    let smtp_transport = LettreSmtpTransport::new(&args.smtp).expect("smtp transport");
    let email_channel = Arc::new(
        EmailChannel::builder()
            .fetcher(Arc::new(MockMailbox::new()) as Arc<dyn MailboxFetcher>)
            .transport(Arc::new(smtp_transport))
            .from_address(args.from_address)
            .allow_list(AllowList::default())
            .poll_interval(Duration::from_secs(3600))
            .build()
            .expect("email channel"),
    );
    state = state.register_channel(
        ChannelRegistration::new(EMAIL_CHANNEL)
            .with_intake(email_channel.clone())
            .with_delivery(email_channel.clone())
            .with_notify(email_channel),
    );

    let _ = workspace_root.keep();

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let serve_state = state.clone();
    tokio::spawn(async move {
        axum::serve(listener, app(serve_state))
            .await
            .expect("serve");
    });
    let base = format!("http://{addr}");
    let ws = format!("ws://{addr}/ws");
    (base, ws, state)
}

/// Extract `filename="..."` (RFC 2183 Content-Disposition) from an
/// email body. Handles both quoted + bare forms; returns the first
/// match. Sufficient for asserting on the rendered docx attachment.
fn extract_attachment_filename(raw: &str) -> Option<String> {
    let needle = "filename=";
    let pos = raw.find(needle)?;
    let after = &raw[pos + needle.len()..];
    let after = after.trim_start();
    if let Some(rest) = after.strip_prefix('"') {
        rest.find('"').map(|end| rest[..end].to_string())
    } else {
        let end = after
            .find(|c: char| c.is_whitespace() || c == ';')
            .unwrap_or(after.len());
        Some(after[..end].to_string())
    }
}

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

async fn wait_for_session(
    state: &AppState,
    task_id: &str,
    timeout: Duration,
) -> Result<String, String> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        let tid = task_id.to_string();
        let found: Option<String> = state
            .db
            .with_conn(move |conn| {
                conn.query_row(
                    "SELECT id FROM sessions WHERE task_id = ? ORDER BY created_at ASC LIMIT 1",
                    rusqlite::params![tid],
                    |row| row.get::<_, String>(0),
                )
                .ok()
            })
            .await;
        if let Some(sid) = found {
            return Ok(sid);
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    Err(format!(
        "no session row for task {task_id} within {timeout:?}"
    ))
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
