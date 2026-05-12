//! refs: /specs/phase-0/architecture.md §4.1

use axum::http::StatusCode;
use tokio::net::TcpListener;

#[tokio::test]
async fn healthz_returns_ok() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral test port");
    let addr = listener.local_addr().expect("read local test address");

    tokio::spawn(async move {
        axum::serve(listener, seasoned_hand_server::app())
            .await
            .expect("serve healthz test app");
    });

    let resp = reqwest::get(format!("http://{addr}/healthz"))
        .await
        .expect("GET /healthz");

    assert_eq!(resp.status(), StatusCode::OK);

    let body: serde_json::Value = resp.json().await.expect("parse healthz JSON");
    assert_eq!(body["status"], "ok");
    assert_eq!(body["version"], seasoned_hand_core::version());
}
