//! Runtime endpoint resolution. Mirrors the `API_BASE` / `WS_URL` logic in the
//! legacy `frontend/lib/api.ts` + `frontend/components/home-shell.tsx`: derive
//! the control-plane host from the page origin and target port 3000.

fn hostname() -> String {
    web_sys::window()
        .and_then(|w| w.location().hostname().ok())
        .unwrap_or_else(|| "localhost".to_string())
}

/// REST base, e.g. `http://localhost:3000`.
pub fn api_base() -> String {
    format!("http://{}:3000", hostname())
}

/// WebSocket endpoint, e.g. `ws://localhost:3000/ws`.
pub fn ws_url() -> String {
    format!("ws://{}:3000/ws", hostname())
}
