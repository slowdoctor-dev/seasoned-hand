//! Runtime endpoint resolution. Mirrors the `API_BASE` / `WS_URL` logic in the
//! legacy `frontend/lib/api.ts` + `frontend/components/home-shell.tsx`: derive
//! the control-plane host from the page origin and target port 3000.
//!
//! Platform seam (issue #4): host resolution is the first piece gated per target.
//! Web reads the page origin via `web-sys`; native (desktop/mobile) reads an env
//! var. The REST/WS transport itself (`gloo-net` today) still needs a native
//! impl (`reqwest` / `tokio-tungstenite`) before desktop/mobile can build — that
//! is the remaining work in issue #4.

/// Resolve the control-plane host. Web: page origin. Native: `SH_UI_HOST` env.
#[cfg(target_arch = "wasm32")]
fn hostname() -> String {
    web_sys::window()
        .and_then(|w| w.location().hostname().ok())
        .unwrap_or_else(|| "localhost".to_string())
}

#[cfg(not(target_arch = "wasm32"))]
fn hostname() -> String {
    std::env::var("SH_UI_HOST").unwrap_or_else(|_| "localhost".to_string())
}

/// REST base, e.g. `http://localhost:3000`.
pub fn api_base() -> String {
    format!("http://{}:3000", hostname())
}

/// WebSocket endpoint, e.g. `ws://localhost:3000/ws`.
pub fn ws_url() -> String {
    format!("ws://{}:3000/ws", hostname())
}
