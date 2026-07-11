//! Channel health + test routes (story 2.5).
//! Moved from `lib.rs` (issue #43); pure code move.

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};

use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::error::{ApiError, api_err};
use crate::guards::require_loopback;

// ---------------------------------------------------------------------------
// Story 2.5: channel HTTP routes.
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub(crate) struct ChannelHealthDto {
    name: String,
    capabilities: Vec<&'static str>,
}

impl From<seasoned_hand_core::channel::ChannelHealth> for ChannelHealthDto {
    fn from(h: seasoned_hand_core::channel::ChannelHealth) -> Self {
        Self {
            name: h.name,
            capabilities: h.capabilities,
        }
    }
}

pub(crate) async fn list_channels_handler(
    State(state): State<AppState>,
    axum::extract::ConnectInfo(remote): axum::extract::ConnectInfo<std::net::SocketAddr>,
) -> Result<Json<Vec<ChannelHealthDto>>, (StatusCode, Json<ApiError>)> {
    require_loopback(remote)?;
    let snapshot = state
        .channels
        .health()
        .into_iter()
        .map(ChannelHealthDto::from)
        .collect();
    Ok(Json(snapshot))
}

pub(crate) async fn get_channel_health_handler(
    State(state): State<AppState>,
    axum::extract::ConnectInfo(remote): axum::extract::ConnectInfo<std::net::SocketAddr>,
    Path(name): Path<String>,
) -> Result<Json<ChannelHealthDto>, (StatusCode, Json<ApiError>)> {
    require_loopback(remote)?;
    state
        .channels
        .health()
        .into_iter()
        .find(|h| h.name == name)
        .map(ChannelHealthDto::from)
        .map(Json)
        .ok_or_else(|| api_err(StatusCode::NOT_FOUND, "channel_not_found".into()))
}

#[derive(Debug, Deserialize)]
pub(crate) struct ChannelTestQuery {
    pub(crate) role: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ChannelTestResponse {
    name: String,
    role: String,
    ok: bool,
}

/// Phase 2 stub: confirm `channel` is registered AND has the requested
/// role implemented. Real synthetic round-trips land per-channel in
/// stories 2.9–2.13 (each can specialise `dry-run`).
pub(crate) async fn post_channel_test_handler(
    State(state): State<AppState>,
    axum::extract::ConnectInfo(remote): axum::extract::ConnectInfo<std::net::SocketAddr>,
    Path(name): Path<String>,
    Query(q): Query<ChannelTestQuery>,
) -> Result<Json<ChannelTestResponse>, (StatusCode, Json<ApiError>)> {
    require_loopback(remote)?;
    let role = q.role.as_deref().unwrap_or("delivery");
    let registered = match role {
        "intake" => state.channels.get_intake(&name).is_some(),
        "delivery" => state.channels.get_delivery(&name).is_some(),
        "notify" => state.channels.get_notify(&name).is_some(),
        _other => {
            return Err(api_err(StatusCode::BAD_REQUEST, "unknown_role".into()));
        }
    };
    if !registered {
        // Distinguish channel-missing from role-missing so operators
        // know whether to fix the registration or pick a different role.
        let channel_exists = state.channels.health().iter().any(|h| h.name == name);
        let err = if channel_exists {
            "role_not_implemented"
        } else {
            "channel_not_found"
        };
        return Err(api_err(StatusCode::NOT_FOUND, err.to_string()));
    }
    Ok(Json(ChannelTestResponse {
        name,
        role: role.to_string(),
        ok: true,
    }))
}
