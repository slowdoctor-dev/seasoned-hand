//! Client-side verified-session auth (issue #26, on top of ADR-018).
//!
//! Obtains an opaque session token — automatically via `/v1/auth/dev-login` in
//! local dev, or from an invitation token via `/v1/auth/login` — persists the
//! session response in `localStorage`, and exposes the current bearer token to
//! `api.rs` (REST) and `ws.rs` (the `/ws` subprotocol).
//!
//! Storage: `localStorage`, because invitation tokens are single-use — an
//! in-memory token would be lost on reload while the invitation is already
//! consumed, locking the user out. Only the opaque session response is stored
//! (never the invitation token). Expiry is enforced server-side; a stale token
//! returns 401, which clears the session and returns the app to the login state.

use dioxus::prelude::*;
use serde::{Deserialize, Serialize};

const STORAGE_KEY: &str = "sh_auth_session";

/// Reactive auth state used to gate rendering (Loading screen / login form /
/// app). Written by the startup bootstrap, the login form, and `api.rs` on 401.
pub static AUTH: GlobalSignal<AuthState> = Signal::global(|| AuthState::Loading);

#[derive(Clone, PartialEq)]
pub enum AuthState {
    /// Startup: deciding between a stored session, dev-login, and the form.
    Loading,
    /// No valid session; show the invitation-token form (with an optional error).
    NeedLogin(Option<String>),
    /// A session token is available; render the app.
    Authed,
}

/// The verified session returned by `/v1/auth/login` and `/v1/auth/dev-login`.
#[derive(Clone, Serialize, Deserialize)]
pub struct AuthSession {
    pub access_token: String,
    pub expires_at: i64,
    pub tenant_id: String,
    pub organization_id: String,
    pub actor_user_id: String,
    pub org_role: String,
}

// --- localStorage (web) ---------------------------------------------------

#[cfg(target_arch = "wasm32")]
fn storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok()?
}

#[cfg(target_arch = "wasm32")]
pub fn load_session() -> Option<AuthSession> {
    let raw = storage()?.get_item(STORAGE_KEY).ok()??;
    serde_json::from_str(&raw).ok()
}

#[cfg(target_arch = "wasm32")]
pub fn store_session(session: &AuthSession) {
    if let Some(store) = storage() {
        if let Ok(json) = serde_json::to_string(session) {
            let _ = store.set_item(STORAGE_KEY, &json);
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub fn clear_session() {
    if let Some(store) = storage() {
        let _ = store.remove_item(STORAGE_KEY);
    }
}

// --- localStorage (native stubs; the crate currently ships wasm only) -----

#[cfg(not(target_arch = "wasm32"))]
pub fn load_session() -> Option<AuthSession> {
    None
}
#[cfg(not(target_arch = "wasm32"))]
pub fn store_session(_session: &AuthSession) {}
#[cfg(not(target_arch = "wasm32"))]
pub fn clear_session() {}

// --- token access + 401 handling ------------------------------------------

/// Current bearer token, if a session is stored. Read by `api.rs` per request
/// and by `ws.rs` on each (re)connect, so a refreshed session is picked up.
pub fn current_token() -> Option<String> {
    load_session().map(|s| s.access_token)
}

/// Drop a now-invalid session (server returned 401) and return to login. Safe to
/// call from a spawned async task within the Dioxus runtime.
pub fn on_unauthorized() {
    clear_session();
    *AUTH.write() = AuthState::NeedLogin(Some(
        "Session expired or invalid — please sign in again.".to_string(),
    ));
}

// --- login endpoints ------------------------------------------------------

async fn post_session(path: &str, body: Option<serde_json::Value>) -> Result<AuthSession, String> {
    use crate::config::api_base;
    use gloo_net::http::Request;

    let url = format!("{}{}", api_base(), path);
    let builder = Request::post(&url);
    let resp = match body {
        Some(b) => builder
            .json(&b)
            .map_err(|e| e.to_string())?
            .send()
            .await
            .map_err(|e| e.to_string())?,
        None => builder.send().await.map_err(|e| e.to_string())?,
    };
    if !resp.ok() {
        return Err(format!("auth {path} -> {}", resp.status()));
    }
    resp.json::<AuthSession>()
        .await
        .map_err(|e| format!("auth {path} decode -> {e}"))
}

/// Zero-friction local dev: mint a session for the default dev identity. Only
/// succeeds on a loopback server with `SH_INSECURE_AUTH_HEADERS=1`; otherwise it
/// returns an error and the caller falls back to the invitation form.
pub async fn dev_login() -> Result<AuthSession, String> {
    post_session("/v1/auth/dev-login", None).await
}

/// Exchange a single-use invitation token for a session.
pub async fn login(invitation_token: &str) -> Result<AuthSession, String> {
    post_session(
        "/v1/auth/login",
        Some(serde_json::json!({ "invitation_token": invitation_token })),
    )
    .await
}
