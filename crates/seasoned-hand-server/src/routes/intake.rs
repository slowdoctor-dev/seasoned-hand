//! Intake surfaces: webhook (story 2.10), CLI intake + inbox + briefing-confirm (story 2.21b).
//! Moved from `lib.rs` (issue #43); pure code move.

use axum::{
    Extension, Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
};

use seasoned_hand_core::auth::{Action, AuthContext};
use seasoned_hand_core::channel::webhook::TokenCheck;
use seasoned_hand_core::time::now_micros;
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::error::{ApiError, api_err};
use crate::guards::{authorize_in_handler, require_loopback, require_task_tenant};

// ---------------------------------------------------------------------------
// Story 2.10: WebhookChannel intake — POST /v1/intake/webhook.
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub(crate) struct WebhookIntakeBody {
    brief: String,
    #[serde(default)]
    project_id: Option<String>,
    #[serde(default)]
    reply_target: Option<seasoned_hand_core::channel::DeliveryTarget>,
    #[serde(default)]
    metadata: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
pub(crate) struct WebhookIntakeAck {
    task_id: String,
    /// Phase 2 reserves this slot per architecture §2.8 — the
    /// briefing-confirmation flow that fills it lands in story 2.8.
    /// Returning `None` is preferable to omitting the field so the
    /// response shape is stable across the briefing rollout.
    briefing_call_id: Option<String>,
}

pub(crate) async fn post_intake_webhook_handler(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    body: Result<Json<WebhookIntakeBody>, axum::extract::rejection::JsonRejection>,
) -> Result<(StatusCode, Json<WebhookIntakeAck>), (StatusCode, Json<ApiError>)> {
    use seasoned_hand_core::channel::IntakeEvent;
    use seasoned_hand_core::channel::webhook::CHANNEL_NAME as WEBHOOK_NAME;
    use seasoned_hand_core::intake::router::{HandleOutcome, RejectionReason};

    // Locate the registered WebhookChannel's intake provider so we
    // can re-use its constant-time token check. Falling back to the
    // raw `webhook_intake_token` keeps the handler honest if a future
    // refactor changes the registration shape.
    let token_check = if !state.webhook_intake_token.is_empty() {
        let supplied = headers
            .get("X-Seasoned-Hand-Intake-Token")
            .and_then(|h| h.to_str().ok());
        use subtle::ConstantTimeEq;
        let ok: bool = supplied
            .unwrap_or("")
            .as_bytes()
            .ct_eq(state.webhook_intake_token.as_bytes())
            .into();
        if ok {
            TokenCheck::Ok
        } else {
            TokenCheck::Mismatch
        }
    } else {
        TokenCheck::NotConfigured
    };

    match token_check {
        TokenCheck::NotConfigured => {
            return Err(api_err(
                StatusCode::SERVICE_UNAVAILABLE,
                "intake_token_not_configured".into(),
            ));
        }
        TokenCheck::Mismatch => {
            return Err(api_err(
                StatusCode::UNAUTHORIZED,
                "unauthorized_token".into(),
            ));
        }
        TokenCheck::Ok => {}
    }

    let Json(body) = body.map_err(|error| {
        tracing::warn!(%error, "webhook intake: invalid JSON body (returning 400)");
        api_err(StatusCode::BAD_REQUEST, "invalid_json_body".into())
    })?;

    let mut metadata = body.metadata.unwrap_or_else(|| serde_json::json!({}));
    if let Some(pid) = body.project_id.as_ref()
        && let Some(obj) = metadata.as_object_mut()
    {
        obj.insert("project_id".into(), serde_json::Value::String(pid.clone()));
    }

    let intake_event = IntakeEvent {
        channel: WEBHOOK_NAME.into(),
        intake_id: format!("http:{}", uuid::Uuid::new_v4()),
        brief_input: body.brief,
        reply_target: body.reply_target,
        metadata,
        tenant_id: None,
        received_at: now_micros(),
    };

    match state.intake_router.handle_event(intake_event).await {
        Ok(HandleOutcome::Created { task_id, .. }) => Ok((
            StatusCode::ACCEPTED,
            Json(WebhookIntakeAck {
                task_id,
                briefing_call_id: None,
            }),
        )),
        Ok(HandleOutcome::DuplicateSkipped) => Err(api_err(
            StatusCode::CONFLICT,
            "intake_rejected:duplicate_intake_id".into(),
        )),
        Ok(HandleOutcome::Rejected(reason)) => {
            // DEBT #12 close-out for the webhook surface: validation
            // rejection surfaces as 4xx with the spec-shaped
            // `intake_rejected:<reason>` payload. The pre-task Misc
            // event remains deferred until a system-session strategy
            // exists.
            let reason_code = match reason {
                RejectionReason::EmptyBrief => "empty_brief",
                RejectionReason::UnknownChannel(_) => "unknown_channel",
            };
            Err(api_err(
                StatusCode::BAD_REQUEST,
                format!("intake_rejected:{reason_code}"),
            ))
        }
        Err(error) => {
            tracing::error!(%error, "webhook intake: IntakeRouter error");
            Err(api_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error".into(),
            ))
        }
    }
}

// ---------------------------------------------------------------------------
// Story 2.21b: CLI intake + inbox + briefing-confirm HTTP routes.
//
// All three are loopback-only (Phase 2 single-operator); Phase 5
// multi-user adds real auth. The CLI binary owns these — they back
// `seasoned-hand task new`, `seasoned-hand inbox`, and
// `seasoned-hand brief {confirm,edit,cancel}` respectively.
// ---------------------------------------------------------------------------

/// Default ceiling for the long-poll `task new` blocking flow. Operators
/// who want to wait longer can bump via env; tests override via the
/// `?max_wait_ms=` query param. The Initializer's confirm gate plus
/// agent loop comfortably fit inside 10 minutes for Phase 2 briefs;
/// anything longer is "go reach for `--detach`".
pub(crate) const CLI_INTAKE_DEFAULT_MAX_WAIT_SECS: u64 = 600;

#[derive(Debug, Deserialize)]
pub(crate) struct CliIntakeBody {
    brief: String,
    #[serde(default)]
    project_id: Option<String>,
    #[serde(default)]
    metadata: Option<serde_json::Value>,
    /// When `true` (default) the handler holds the request open until
    /// the deliverable lands (or the timeout fires). `false` (matches
    /// the CLI's `--detach` flag) acks as soon as the task row is
    /// minted; the deliverable lands in `~/.seasoned-hand/deliverables/`.
    #[serde(default = "default_cli_intake_wait")]
    wait: bool,
}

pub(crate) fn default_cli_intake_wait() -> bool {
    true
}

#[derive(Debug, Deserialize, Default)]
pub(crate) struct CliIntakeQuery {
    /// Test seam — override the long-poll ceiling without waiting the
    /// full env-derived window. Production callers (the CLI) don't set
    /// this; the smoke test does.
    max_wait_ms: Option<u64>,
}

#[derive(Debug, Serialize)]
pub(crate) struct CliIntakeAck {
    task_id: String,
    intake_id: String,
    /// `Some` on the blocking happy path; `None` when `wait=false` or
    /// when the deliver timed out (a follow-up `task deliverable` /
    /// `inbox` call surfaces it once it lands).
    #[serde(skip_serializing_if = "Option::is_none")]
    deliverable: Option<seasoned_hand_core::deliverable::Deliverable>,
    /// Phase 2 reserves this slot — the briefing-confirm gate is keyed
    /// by `task_id` (DEBT #20) so we echo `task_id` back here as a
    /// stable handle the CLI can hand to `brief confirm/edit/cancel`.
    briefing_call_id: Option<String>,
}

pub(crate) async fn post_intake_cli_handler(
    State(state): State<AppState>,
    axum::extract::ConnectInfo(remote): axum::extract::ConnectInfo<std::net::SocketAddr>,
    headers: HeaderMap,
    Query(q): Query<CliIntakeQuery>,
    Json(body): Json<CliIntakeBody>,
) -> Result<(StatusCode, Json<CliIntakeAck>), (StatusCode, Json<ApiError>)> {
    use seasoned_hand_core::channel::cli::{CHANNEL_NAME, INTAKE_ID_PREFIX, TARGET_INTAKE_PREFIX};
    use seasoned_hand_core::channel::{DeliveryTarget, IntakeEvent};
    use seasoned_hand_core::intake::router::{HandleOutcome, RejectionReason};

    require_loopback(remote)?;

    if body.brief.trim().is_empty() {
        return Err(api_err(
            StatusCode::BAD_REQUEST,
            "intake_rejected:empty_brief".into(),
        ));
    }

    let intake_id = format!("{INTAKE_ID_PREFIX}{}", uuid::Uuid::new_v4());
    let tenant_id = headers
        .get("x-seasoned-hand-tenant-id")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .unwrap_or("legacy-default")
        .to_string();

    // Register the oneshot BEFORE handing the event to the router so a
    // very fast deliver() can't race past our pending slot.
    let rx_opt = if body.wait {
        Some(state.cli_channel.register_pending(intake_id.clone()))
    } else {
        None
    };

    let mut metadata = body.metadata.unwrap_or_else(|| serde_json::json!({}));
    if let Some(obj) = metadata.as_object_mut()
        && let Some(pid) = body.project_id.as_ref()
    {
        obj.insert("project_id".into(), serde_json::Value::String(pid.clone()));
    }

    let event = IntakeEvent {
        channel: CHANNEL_NAME.into(),
        intake_id: intake_id.clone(),
        brief_input: body.brief,
        reply_target: Some(DeliveryTarget {
            channel: CHANNEL_NAME.into(),
            target_ref: format!("{TARGET_INTAKE_PREFIX}{intake_id}"),
            metadata: serde_json::json!({}),
        }),
        metadata,
        tenant_id: Some(tenant_id),
        received_at: now_micros(),
    };

    let task_id = match state.intake_router.handle_event(event).await {
        Ok(HandleOutcome::Created { task_id, .. }) => task_id,
        Ok(HandleOutcome::DuplicateSkipped) => {
            // Shouldn't happen — we mint a fresh UUID — but stay honest.
            if let Some(_rx) = rx_opt {
                state.cli_channel.drop_pending(&intake_id);
            }
            return Err(api_err(
                StatusCode::CONFLICT,
                "intake_rejected:duplicate_intake_id".into(),
            ));
        }
        Ok(HandleOutcome::Rejected(reason)) => {
            if let Some(_rx) = rx_opt {
                state.cli_channel.drop_pending(&intake_id);
            }
            let reason_code = match reason {
                RejectionReason::EmptyBrief => "empty_brief",
                RejectionReason::UnknownChannel(_) => "unknown_channel",
            };
            return Err(api_err(
                StatusCode::BAD_REQUEST,
                format!("intake_rejected:{reason_code}"),
            ));
        }
        Err(error) => {
            if let Some(_rx) = rx_opt {
                state.cli_channel.drop_pending(&intake_id);
            }
            tracing::error!(%error, "cli intake: IntakeRouter error");
            return Err(api_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error".into(),
            ));
        }
    };

    // Detached: ack the task_id; the deliverable lands in the fallback
    // file path. The CLI's `task deliverable <id>` (and `inbox`) surface
    // it after the fact.
    let Some(rx) = rx_opt else {
        return Ok((
            StatusCode::ACCEPTED,
            Json(CliIntakeAck {
                task_id: task_id.clone(),
                intake_id,
                deliverable: None,
                briefing_call_id: Some(task_id),
            }),
        ));
    };

    let max_wait = match q.max_wait_ms {
        Some(ms) => std::time::Duration::from_millis(ms),
        None => std::time::Duration::from_secs(
            std::env::var("CLI_INTAKE_MAX_WAIT_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(CLI_INTAKE_DEFAULT_MAX_WAIT_SECS),
        ),
    };

    match tokio::time::timeout(max_wait, rx).await {
        Ok(Ok(deliverable)) => Ok((
            StatusCode::OK,
            Json(CliIntakeAck {
                task_id: task_id.clone(),
                intake_id,
                deliverable: Some(deliverable),
                briefing_call_id: Some(task_id),
            }),
        )),
        Ok(Err(_recv_err)) => {
            // Sender dropped — the file fallback will catch the
            // deliverable. Surface 504 so the CLI knows to look at
            // the inbox / fallback dir.
            tracing::warn!(%task_id, %intake_id, "cli intake deliver sender dropped before response");
            Err(api_err(
                StatusCode::GATEWAY_TIMEOUT,
                "deliver_dropped:pending_delivery".into(),
            ))
        }
        Err(_elapsed) => {
            // Leave the pending sender registered — when the
            // deliverable finally lands, CliChannel::deliver hits the
            // oneshot, gets a dropped-receiver, and falls back to the
            // file path. The operator can still recover the artifact.
            tracing::warn!(%task_id, %intake_id, "cli intake timed out waiting for delivery");
            Err(api_err(
                StatusCode::GATEWAY_TIMEOUT,
                "deliver_timeout:pending_delivery".into(),
            ))
        }
    }
}

#[derive(Debug, Deserialize, Default)]
pub(crate) struct InboxQuery {
    project_id: Option<String>,
    limit: Option<usize>,
}

#[derive(Debug, Serialize)]
pub(crate) struct InboxEntry {
    /// Phase 2 alias: briefing_id := task_id. The Initializer mints a
    /// fresh `briefing_call_id` per edit cycle but reuses the same
    /// per-task mpsc sender (`AppState::briefing_senders` keyed by
    /// `task_id`), so the confirm route only needs a `task_id`. DEBT
    /// #20 documents the loose-match contract; tightening it later
    /// won't break this surface as long as the alias stays.
    briefing_id: String,
    task_id: String,
    project_id: String,
    title: String,
    brief: Option<serde_json::Value>,
    created_at: i64,
}

/// Local typedef just so clippy::type_complexity stops warning at the
/// `with_conn` closure return type — the inbox row is a flat 5-tuple
/// out of a raw SQL projection and adding a struct here would be churn.
type InboxRow = (String, String, String, Option<String>, i64);

pub(crate) async fn get_inbox_handler(
    State(state): State<AppState>,
    axum::extract::ConnectInfo(remote): axum::extract::ConnectInfo<std::net::SocketAddr>,
    Extension(auth_ctx): Extension<AuthContext>,
    Query(q): Query<InboxQuery>,
) -> Result<Json<Vec<InboxEntry>>, (StatusCode, Json<ApiError>)> {
    require_loopback(remote)?;
    authorize_in_handler(Action::TaskRead, &auth_ctx)?;
    let limit = q.limit.unwrap_or(50).clamp(1, 200) as i64;
    let project_id = q.project_id.clone();
    // P5-HARD-IT7-M7: the inbox is a LIST endpoint story 5.5 missed — it
    // returned every tenant's briefed-task titles + brief content. Scope
    // it to the caller's tenant.
    let tenant = auth_ctx.tenant_id.clone();
    let rows: Vec<InboxRow> = state
        .db
        .with_conn(move |conn| -> rusqlite::Result<Vec<InboxRow>> {
            let (sql, mapped) = match project_id.as_deref() {
                Some(_) => (
                    "SELECT id, project_id, title, brief, created_at \
                           FROM tasks \
                          WHERE status = 'briefed' AND tenant_id = ? AND project_id = ? \
                          ORDER BY created_at DESC LIMIT ?",
                    true,
                ),
                None => (
                    "SELECT id, project_id, title, brief, created_at \
                           FROM tasks \
                          WHERE status = 'briefed' AND tenant_id = ? \
                          ORDER BY created_at DESC LIMIT ?",
                    false,
                ),
            };
            let mut stmt = conn.prepare(sql)?;
            let mapper = |row: &rusqlite::Row<'_>| -> rusqlite::Result<_> {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            };
            if mapped {
                let pid = project_id.unwrap();
                stmt.query_map(rusqlite::params![tenant, pid, limit], mapper)?
                    .collect()
            } else {
                stmt.query_map(rusqlite::params![tenant, limit], mapper)?
                    .collect()
            }
        })
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "inbox query failed");
            api_err(StatusCode::INTERNAL_SERVER_ERROR, "internal_error".into())
        })?;

    let entries = rows
        .into_iter()
        .map(|(task_id, project_id, title, brief_text, created_at)| {
            let brief = brief_text
                .as_deref()
                .and_then(|t| serde_json::from_str::<serde_json::Value>(t).ok());
            InboxEntry {
                briefing_id: task_id.clone(),
                task_id,
                project_id,
                title,
                brief,
                created_at,
            }
        })
        .collect();

    Ok(Json(entries))
}

#[derive(Debug, Deserialize)]
pub(crate) struct BriefingConfirmBody {
    action: String,
    #[serde(default)]
    edits: Option<seasoned_hand_core::agent::init::briefing::PartialBrief>,
}

pub(crate) async fn post_briefing_confirm_handler(
    State(state): State<AppState>,
    axum::extract::ConnectInfo(remote): axum::extract::ConnectInfo<std::net::SocketAddr>,
    Extension(auth_ctx): Extension<AuthContext>,
    Path(briefing_id): Path<String>,
    Json(body): Json<BriefingConfirmBody>,
) -> Result<StatusCode, (StatusCode, Json<ApiError>)> {
    use seasoned_hand_core::agent::init::briefing::{BriefingAction, UserResponse};

    require_loopback(remote)?;
    // P5-HARD-IT7-H8: confirming/cancelling/editing a briefing advances
    // a task's lifecycle — it was reachable with NO auth and NO tenant
    // check, letting any local caller drive another tenant's briefed
    // task. Gate it (TaskWrite) + tenant-scope the task below.
    authorize_in_handler(Action::TaskWrite, &auth_ctx)?;

    // Translate the wire action → BriefingAction enum.
    let action = match body.action.as_str() {
        "confirm" => BriefingAction::Confirm,
        "cancel" => BriefingAction::Cancel,
        "edit" => match body.edits {
            Some(edits) => BriefingAction::Edit { edits },
            None => {
                return Err(api_err(StatusCode::BAD_REQUEST, "missing_edits".into()));
            }
        },
        _ => {
            return Err(api_err(StatusCode::BAD_REQUEST, "invalid_action".into()));
        }
    };

    // Phase 2 alias: briefing_id := task_id (see InboxEntry doc).
    let task_id = briefing_id;

    // P5-HARD-IT7-H8: tenant-scope before touching the task's briefing.
    require_task_tenant(&state, &task_id, &auth_ctx).await?;

    // The Initializer reuses the same per-task receiver across every
    // call_id it emits, so the `in_reply_to_call_id` echo is loose
    // (DEBT #20). The handler tracks the most recent call_id only when
    // a future tightening lands.
    let sender = state
        .briefing_senders
        .get(&task_id)
        .map(|entry| entry.value().clone());
    let Some(sender) = sender else {
        return Err(api_err(StatusCode::NOT_FOUND, "no_pending_briefing".into()));
    };

    let response = UserResponse {
        in_reply_to_call_id: task_id.clone(),
        action,
    };
    sender
        .send(response)
        .await
        .map_err(|_| api_err(StatusCode::CONFLICT, "briefing_receiver_closed".into()))?;

    Ok(StatusCode::ACCEPTED)
}
