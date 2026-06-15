//! Issue #33: the control plane optionally serves the built Dioxus UI bundle as
//! the router fallback (opt-in via `SH_UI_DIST` → `AppState::with_ui_dist`).
//!
//! These tests assert the contract main.rs relies on:
//!   * with a bundle configured, `/` and static assets are served, deep client
//!     paths fall back to `index.html` (SPA), and the `/v1` + `/healthz` API
//!     routes still win over the fallback;
//!   * with no bundle configured, the server stays API-only (unmatched → 404).

use seasoned_hand_core::{db, pubsub};
use seasoned_hand_server::{AppState, app};
use std::path::Path;
use tokio::net::TcpListener;

const INDEX_HTML: &str =
    "<!DOCTYPE html><html><body><div id=\"main\">SH-UI-SHELL</div></body></html>";
const ASSET_CSS: &str = ".x{color:red}/*SH-UI-ASSET*/";

/// Build an `AppState` with the minimal dependency graph the other server tests
/// use (no Docker / Redis needed — both degrade gracefully). `ui_dist`, when
/// `Some`, wires the static-serve fallback.
async fn boot(ui_dist: Option<&Path>) -> String {
    let pool = db::open(":memory:").await.unwrap();
    let redis = pubsub::RedisPool::new("redis://127.0.0.1:6").unwrap();
    let sandbox = seasoned_hand_core::sandbox::SandboxClient::new(
        "ghcr.io/agent-infra/sandbox:1.0.0.152",
        std::env::temp_dir(),
    )
    .unwrap();
    let search = seasoned_hand_core::search::SearchClient::new(
        seasoned_hand_core::search::SearchProvider::Brave { api_key: None },
    );
    let router = seasoned_hand_core::router::SlotRouter::default_for_bifrost();
    let mut state = AppState::new(pool, redis, sandbox, search, router, Default::default());
    if let Some(dir) = ui_dist {
        state = state.with_ui_dist(dir);
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(
            listener,
            app(state).into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .unwrap();
    });
    format!("http://{addr}")
}

/// Lay down a fake `dx build` bundle: `index.html` + `assets/app.css`.
fn write_bundle(dir: &Path) {
    std::fs::write(dir.join("index.html"), INDEX_HTML).unwrap();
    let assets = dir.join("assets");
    std::fs::create_dir_all(&assets).unwrap();
    std::fs::write(assets.join("app.css"), ASSET_CSS).unwrap();
}

#[tokio::test]
async fn serves_index_assets_and_spa_fallback_while_api_routes_win() {
    let tmp = tempfile::tempdir().unwrap();
    write_bundle(tmp.path());
    let base = boot(Some(tmp.path())).await;
    let client = reqwest::Client::new();

    // `/` serves the bundle's index.html.
    let root = client.get(format!("{base}/")).send().await.unwrap();
    assert_eq!(root.status(), reqwest::StatusCode::OK);
    assert!(root.text().await.unwrap().contains("SH-UI-SHELL"));

    // Static asset is served with its real bytes.
    let css = client
        .get(format!("{base}/assets/app.css"))
        .send()
        .await
        .unwrap();
    assert_eq!(css.status(), reqwest::StatusCode::OK);
    assert!(css.text().await.unwrap().contains("SH-UI-ASSET"));

    // A deep client-side path that matches no file falls back to the SPA shell.
    let deep = client
        .get(format!("{base}/projects/abc/tasks/xyz"))
        .send()
        .await
        .unwrap();
    assert_eq!(deep.status(), reqwest::StatusCode::OK);
    assert!(deep.text().await.unwrap().contains("SH-UI-SHELL"));

    // The API still wins over the fallback: `/healthz` is a real route, so it
    // returns the health JSON — NOT the SPA shell.
    let health = client.get(format!("{base}/healthz")).send().await.unwrap();
    let health_body = health.text().await.unwrap();
    assert!(
        !health_body.contains("SH-UI-SHELL"),
        "/healthz must hit the API route, not the UI fallback; got: {health_body}"
    );

    // And a `/v1` path is owned by its route, not the SPA fallback (issue #33
    // review): `/v1/auth/login` is registered for POST, so a GET resolves to the
    // route's 405 — it must NOT fall through to the index.html shell.
    let v1 = client
        .get(format!("{base}/v1/auth/login"))
        .send()
        .await
        .unwrap();
    assert_eq!(v1.status(), reqwest::StatusCode::METHOD_NOT_ALLOWED);
    assert!(
        !v1.text().await.unwrap().contains("SH-UI-SHELL"),
        "/v1/* must hit the API router, not the UI fallback"
    );
}

#[tokio::test]
async fn without_bundle_server_is_api_only() {
    let base = boot(None).await;
    let client = reqwest::Client::new();

    // No fallback configured → unmatched paths 404 (unchanged API-only posture).
    let root = client.get(format!("{base}/")).send().await.unwrap();
    assert_eq!(root.status(), reqwest::StatusCode::NOT_FOUND);

    // The API surface is unaffected.
    let health = client.get(format!("{base}/healthz")).send().await.unwrap();
    let health_body = health.text().await.unwrap();
    assert!(!health_body.contains("SH-UI-SHELL"));
}
