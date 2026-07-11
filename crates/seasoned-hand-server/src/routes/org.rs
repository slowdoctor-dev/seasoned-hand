//! Org / sharing surfaces: task handoff, audit, billing reconcile, invites, SOP shares.
//! Moved from `lib.rs` (issue #43); pure code move.

use axum::{
    Extension, Json,
    extract::{Path, Query, State},
    http::StatusCode,
};

use seasoned_hand_core::audit::{AuditLogger, AuditQuery};
use seasoned_hand_core::auth::{Action, AuthContext};
use seasoned_hand_core::billing::{ReconciliationJob, ReconciliationReport};
use seasoned_hand_core::handoff::{HandoffRequest, TaskHandoffService};
use seasoned_hand_core::org::{InvitationService, InviteOutcome, MembershipRow};
use seasoned_hand_core::sharing::sop::{SopPermission, SopShareService};
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::error::{
    ApiResult, api_err, map_audit_query_error, map_handoff_error, map_invitation_error,
    map_sop_share_error,
};
use crate::guards::{authorize_in_handler, require_loopback, require_task_tenant};

#[derive(Debug, Deserialize)]
pub(crate) struct TaskHandoffBody {
    to_user_email: String,
    reason: Option<String>,
    expected_updated_at: Option<i64>,
}

#[derive(Debug, Serialize)]
pub(crate) struct TaskHandoffCanResponse {
    can_handoff: bool,
    reason: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub(crate) struct AuditListQuery {
    actor: Option<String>,
    action: Option<String>,
    since: Option<String>,
    limit: Option<usize>,
    task: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct UserCostReconcileBody {
    month_yyyymm: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct InviteUserBody {
    email: String,
    role: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SopShareBody {
    pub(crate) user_email: String,
    pub(crate) permission: String,
    /// Story 5.21: optimistic concurrency precondition. When present
    /// and the live share row's `updated_at` doesn't match, the
    /// response is `409 stale_revision`.
    #[serde(default)]
    pub(crate) expected_updated_at: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SopUnshareBody {
    pub(crate) user_email: String,
    /// Story 5.21: optimistic concurrency precondition (see SopShareBody).
    #[serde(default)]
    pub(crate) expected_updated_at: Option<i64>,
}

#[derive(Debug, Serialize)]
pub(crate) struct SopShareDto {
    id: String,
    tenant_id: String,
    sop_id: String,
    subject_type: String,
    subject_id: String,
    subject_email: Option<String>,
    permission: String,
    granted_by_user_id: String,
    created_at: i64,
    updated_at: i64,
}

impl From<seasoned_hand_core::sharing::sop::SopShareRow> for SopShareDto {
    fn from(value: seasoned_hand_core::sharing::sop::SopShareRow) -> Self {
        Self {
            id: value.id,
            tenant_id: value.tenant_id,
            sop_id: value.sop_id,
            subject_type: value.subject_type,
            subject_id: value.subject_id,
            subject_email: value.subject_email,
            permission: match value.permission {
                SopPermission::Viewer => "viewer".into(),
                SopPermission::Editor => "editor".into(),
                SopPermission::Owner => "owner".into(),
            },
            granted_by_user_id: value.granted_by_user_id,
            created_at: value.created_at,
            updated_at: value.updated_at,
        }
    }
}

pub(crate) async fn post_task_handoff_handler(
    State(state): State<AppState>,
    axum::extract::ConnectInfo(remote): axum::extract::ConnectInfo<std::net::SocketAddr>,
    Extension(auth_ctx): Extension<AuthContext>,
    Path(task_id): Path<String>,
    Json(body): Json<TaskHandoffBody>,
) -> ApiResult<(
    StatusCode,
    Json<seasoned_hand_core::handoff::HandoffOutcome>,
)> {
    require_loopback(remote)?;
    authorize_in_handler(Action::TaskHandoff, &auth_ctx)?;
    let handoff = TaskHandoffService::new(
        state.db.clone(),
        state.events.clone(),
        AuditLogger::new(state.db.clone(), state.events.clone()),
    );
    let outcome = handoff
        .handoff(
            &auth_ctx,
            HandoffRequest {
                task_id,
                to_user_email: body.to_user_email,
                reason: body.reason,
                expected_updated_at: body.expected_updated_at,
            },
        )
        .await
        .map_err(map_handoff_error)?;
    Ok((StatusCode::OK, Json(outcome)))
}

pub(crate) async fn get_task_handoff_can_handler(
    State(state): State<AppState>,
    axum::extract::ConnectInfo(remote): axum::extract::ConnectInfo<std::net::SocketAddr>,
    Extension(auth_ctx): Extension<AuthContext>,
    Path(task_id): Path<String>,
) -> ApiResult<Json<TaskHandoffCanResponse>> {
    require_loopback(remote)?;
    authorize_in_handler(Action::TaskHandoff, &auth_ctx)?;
    // Issue #8: the coarse RBAC gate lets a User reach this read; scope the task
    // to the caller's tenant (404 on mismatch) so cross-tenant handoff-status
    // isn't exposed by id.
    require_task_tenant(&state, &task_id, &auth_ctx).await?;
    let handoff = TaskHandoffService::new(
        state.db.clone(),
        state.events.clone(),
        AuditLogger::new(state.db.clone(), state.events.clone()),
    );
    let can = handoff
        .can_handoff(&task_id)
        .await
        .map_err(map_handoff_error)?;
    let reason = if can {
        None
    } else {
        Some("pause required or terminal/unknown task".to_string())
    };
    Ok(Json(TaskHandoffCanResponse {
        can_handoff: can,
        reason,
    }))
}

pub(crate) async fn list_audit_handler(
    State(state): State<AppState>,
    axum::extract::ConnectInfo(remote): axum::extract::ConnectInfo<std::net::SocketAddr>,
    Extension(auth_ctx): Extension<AuthContext>,
    Query(q): Query<AuditListQuery>,
) -> ApiResult<Json<Vec<seasoned_hand_core::audit::AuditRow>>> {
    require_loopback(remote)?;
    authorize_in_handler(Action::AuditRead, &auth_ctx)?;
    let logger = AuditLogger::new(state.db.clone(), state.events.clone());
    let action = match q.action.as_deref() {
        Some(v) => Some(
            parse_audit_action(v)
                .ok_or(api_err(StatusCode::BAD_REQUEST, "invalid_action".into()))?,
        ),
        None => None,
    };
    let since_micros = q
        .since
        .as_deref()
        .map(parse_since_to_micros)
        .transpose()
        .map_err(|_| api_err(StatusCode::BAD_REQUEST, "invalid_since".into()))?;
    let rows = logger
        .query(
            &auth_ctx,
            AuditQuery {
                actor_user_id: q.actor,
                action,
                since_micros,
                limit: q.limit,
            },
        )
        .await
        .map_err(map_audit_query_error)?;
    let rows = if let Some(task_id) = q.task {
        rows.into_iter()
            .filter(|r| r.resource_type == "task" && r.resource_id == task_id)
            .collect()
    } else {
        rows
    };
    Ok(Json(rows))
}

pub(crate) async fn post_user_cost_reconcile_handler(
    State(state): State<AppState>,
    axum::extract::ConnectInfo(remote): axum::extract::ConnectInfo<std::net::SocketAddr>,
    Extension(auth_ctx): Extension<AuthContext>,
    Json(body): Json<UserCostReconcileBody>,
) -> ApiResult<Json<ReconciliationReport>> {
    require_loopback(remote)?;
    authorize_in_handler(Action::AuditRead, &auth_ctx)?;
    if !is_valid_month_yyyymm(&body.month_yyyymm) {
        return Err(api_err(
            StatusCode::BAD_REQUEST,
            "invalid_month_yyyymm".into(),
        ));
    }
    let job = ReconciliationJob::new(state.db.clone(), state.events.clone());
    let mut report = job.run(&body.month_yyyymm).await.map_err(|error| {
        tracing::error!(%error, "user_cost reconcile failed");
        api_err(StatusCode::INTERNAL_SERVER_ERROR, "internal_error".into())
    })?;
    // P5-HARD-IT7-M10: the reconciliation job aggregates across ALL
    // tenants (it's a global ops cron). The HTTP trigger is admin-gated,
    // but an admin is scoped to their own tenant — so restrict the
    // returned drift findings (which carry tenant_id/user_id/cost) to
    // the caller's tenant before responding.
    report.drifts.retain(|d| d.tenant_id == auth_ctx.tenant_id);
    report.drifted_rows = report.drifts.len();
    Ok(Json(report))
}

pub(crate) async fn post_org_invite_user_handler(
    State(state): State<AppState>,
    axum::extract::ConnectInfo(remote): axum::extract::ConnectInfo<std::net::SocketAddr>,
    Extension(auth_ctx): Extension<AuthContext>,
    Path(slug): Path<String>,
    Json(body): Json<InviteUserBody>,
) -> ApiResult<(StatusCode, Json<InviteOutcome>)> {
    require_loopback(remote)?;
    authorize_in_handler(Action::MembershipManage, &auth_ctx)?;
    let service = InvitationService::new(
        state.db.clone(),
        AuditLogger::new(state.db.clone(), state.events.clone()),
    );
    let out = service
        .invite_user(&auth_ctx, &slug, &body.email, &body.role)
        .await
        .map_err(map_invitation_error)?;
    Ok((StatusCode::OK, Json(out)))
}

pub(crate) async fn list_org_users_handler(
    State(state): State<AppState>,
    axum::extract::ConnectInfo(remote): axum::extract::ConnectInfo<std::net::SocketAddr>,
    Extension(auth_ctx): Extension<AuthContext>,
    Path(slug): Path<String>,
) -> ApiResult<Json<Vec<MembershipRow>>> {
    require_loopback(remote)?;
    authorize_in_handler(Action::MembershipManage, &auth_ctx)?;
    let service = InvitationService::new(
        state.db.clone(),
        AuditLogger::new(state.db.clone(), state.events.clone()),
    );
    let rows = service
        .list_org_users(&auth_ctx, &slug)
        .await
        .map_err(map_invitation_error)?;
    Ok(Json(rows))
}

pub(crate) fn parse_audit_action(value: &str) -> Option<seasoned_hand_core::audit::AuditAction> {
    use seasoned_hand_core::audit::AuditAction;
    match value {
        "task.handoff" => Some(AuditAction::TaskHandoff),
        "task.cancel" => Some(AuditAction::TaskCancel),
        "sop.share" => Some(AuditAction::SopShare),
        "sop.unshare" => Some(AuditAction::SopUnshare),
        "playbook.share" => Some(AuditAction::PlaybookShare),
        "playbook.unshare" => Some(AuditAction::PlaybookUnshare),
        "playbook.approve" => Some(AuditAction::PlaybookApprove),
        "user.invite" => Some(AuditAction::UserInvite),
        "user.deactivate" => Some(AuditAction::UserDeactivate),
        "membership.update" => Some(AuditAction::MembershipUpdate),
        "event.raw_read" => Some(AuditAction::EventRawRead),
        _ => None,
    }
}

pub(crate) fn parse_since_to_micros(value: &str) -> Result<i64, ()> {
    value.parse::<i64>().map_err(|_| ())
}

pub(crate) fn is_valid_month_yyyymm(value: &str) -> bool {
    value.len() == 6
        && value.chars().all(|c| c.is_ascii_digit())
        && value[4..6]
            .parse::<u32>()
            .is_ok_and(|m| (1..=12).contains(&m))
}

pub(crate) async fn post_sop_share_handler(
    State(state): State<AppState>,
    axum::extract::ConnectInfo(remote): axum::extract::ConnectInfo<std::net::SocketAddr>,
    Extension(auth_ctx): Extension<AuthContext>,
    Path(sop_id): Path<String>,
    Json(body): Json<SopShareBody>,
) -> ApiResult<(StatusCode, Json<SopShareDto>)> {
    // SEC-IT1-H2: match the loopback defense-in-depth every other
    // sensitive Phase 5 handler applies (these 3 SOP-share routes were
    // the only sensitive handlers missing it).
    require_loopback(remote)?;
    authorize_in_handler(Action::SopShare, &auth_ctx)?;
    let permission = parse_sop_permission(&body.permission)?;
    let service = SopShareService::new(state.db.clone());
    let row = service
        .share(
            &auth_ctx,
            &sop_id,
            &body.user_email,
            permission,
            body.expected_updated_at,
        )
        .await
        .map_err(map_sop_share_error)?;
    Ok((StatusCode::OK, Json(SopShareDto::from(row))))
}

pub(crate) async fn delete_sop_share_handler(
    State(state): State<AppState>,
    axum::extract::ConnectInfo(remote): axum::extract::ConnectInfo<std::net::SocketAddr>,
    Extension(auth_ctx): Extension<AuthContext>,
    Path(sop_id): Path<String>,
    Json(body): Json<SopUnshareBody>,
) -> ApiResult<StatusCode> {
    require_loopback(remote)?; // SEC-IT1-H2
    authorize_in_handler(Action::SopShare, &auth_ctx)?;
    let service = SopShareService::new(state.db.clone());
    let deleted = service
        .unshare(
            &auth_ctx,
            &sop_id,
            &body.user_email,
            body.expected_updated_at,
        )
        .await
        .map_err(map_sop_share_error)?;
    if !deleted {
        return Err(api_err(StatusCode::NOT_FOUND, "share_not_found".into()));
    }
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn list_sop_shares_handler(
    State(state): State<AppState>,
    axum::extract::ConnectInfo(remote): axum::extract::ConnectInfo<std::net::SocketAddr>,
    Extension(auth_ctx): Extension<AuthContext>,
    Path(sop_id): Path<String>,
) -> ApiResult<Json<Vec<SopShareDto>>> {
    require_loopback(remote)?; // SEC-IT1-H2
    authorize_in_handler(Action::SopShare, &auth_ctx)?;
    let service = SopShareService::new(state.db.clone());
    let rows = service
        .list_for_sop(&auth_ctx, &sop_id)
        .await
        .map_err(map_sop_share_error)?;
    let out = rows.into_iter().map(SopShareDto::from).collect();
    Ok(Json(out))
}

pub(crate) fn parse_sop_permission(value: &str) -> ApiResult<SopPermission> {
    match value {
        "viewer" => Ok(SopPermission::Viewer),
        "editor" => Ok(SopPermission::Editor),
        "owner" => Ok(SopPermission::Owner),
        _ => Err(api_err(
            StatusCode::BAD_REQUEST,
            "invalid_permission".into(),
        )),
    }
}
