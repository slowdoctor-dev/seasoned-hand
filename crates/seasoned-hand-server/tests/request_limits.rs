//! Issue #22 batch D: the control plane caps request body size
//! (`DefaultBodyLimit`) so an internet-facing intake can't be fed an unbounded
//! payload. The limit is applied globally in `app()`, so any route rejects an
//! oversized body with 413 before the handler runs.

use seasoned_hand_core::{db, pubsub};
use seasoned_hand_server::{AppState, app};
use tokio::net::TcpListener;

async fn boot() -> String {
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
    let state = AppState::new(pool, redis, sandbox, search, router, Default::default());

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

#[tokio::test]
async fn oversized_request_body_is_rejected_413() {
    let base = boot().await;
    let client = reqwest::Client::new();

    // 2 MiB body — over the 1 MiB DefaultBodyLimit. `/v1/auth/login` is a public
    // POST route, so the limit (a global layer) rejects before the handler.
    let big = vec![b'x'; 2 * 1024 * 1024];
    let resp = client
        .post(format!("{base}/v1/auth/login"))
        .header("content-type", "application/json")
        .body(big)
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        reqwest::StatusCode::PAYLOAD_TOO_LARGE,
        "oversized body must be rejected with 413"
    );
}

#[tokio::test]
async fn normal_request_body_is_not_rejected_for_size() {
    let base = boot().await;
    let client = reqwest::Client::new();

    // A small (malformed) body must NOT trip the size limit — it reaches the
    // handler and fails for some OTHER reason (not 413), proving the limit only
    // rejects oversized payloads.
    let resp = client
        .post(format!("{base}/v1/auth/login"))
        .header("content-type", "application/json")
        .body("{}")
        .send()
        .await
        .unwrap();
    assert_ne!(
        resp.status(),
        reqwest::StatusCode::PAYLOAD_TOO_LARGE,
        "a small body must not be rejected for size"
    );
}
