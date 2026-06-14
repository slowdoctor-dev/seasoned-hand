//! refs: /specs/phase-0/architecture.md §4.1

use axum::http::StatusCode;
use seasoned_hand_core::router::SlotRouter;
use seasoned_hand_core::sandbox::SandboxClient;
use seasoned_hand_core::search::{SearchClient, SearchProvider};
use seasoned_hand_core::{db, pubsub};
use seasoned_hand_server::{AppState, app};
use serde_json::json;
use tokio::net::TcpListener;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn healthz_returns_ok_with_db_only() {
    // No Redis required; expect degraded when Redis is unreachable, but
    // db field still reports ok.
    let pool = db::open(":memory:").await.expect("open in-memory db");
    // Use an unreachable Redis port for deterministic offline behavior.
    let redis = pubsub::RedisPool::new("redis://127.0.0.1:6").expect("build pool");
    let sandbox = SandboxClient::new(
        "ghcr.io/agent-infra/sandbox:1.0.0.152",
        std::env::temp_dir(),
    )
    .expect("docker socket");
    let search = SearchClient::new(SearchProvider::Brave { api_key: None });
    let router = SlotRouter::default_for_bifrost();
    let state = AppState::new(pool, redis, sandbox, search, router, Default::default());

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral test port");
    let addr = listener.local_addr().expect("read local test address");

    tokio::spawn(async move {
        axum::serve(listener, app(state))
            .await
            .expect("serve healthz test app");
    });

    let resp = reqwest::get(format!("http://{addr}/healthz"))
        .await
        .expect("GET /healthz");

    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body: serde_json::Value = resp.json().await.expect("parse healthz JSON");
    assert_eq!(body["status"], "degraded");
    assert_eq!(body["db"], "ok");
    assert_eq!(body["redis"], "unreachable");
    assert_eq!(body["version"], seasoned_hand_core::version());
}

#[tokio::test]
async fn cost_route_proxies_bifrost_cost() {
    let bifrost = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/cost"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "total_cents": 42,
            "currency": "USD"
        })))
        .mount(&bifrost)
        .await;

    let pool = db::open(":memory:").await.expect("open in-memory db");
    let redis = pubsub::RedisPool::new("redis://127.0.0.1:6").expect("build pool");
    let sandbox = SandboxClient::new(
        "ghcr.io/agent-infra/sandbox:1.0.0.152",
        std::env::temp_dir(),
    )
    .expect("docker socket");
    let search = SearchClient::new(SearchProvider::Brave { api_key: None });
    let router = SlotRouter::from_yaml_str(&format!(
        r#"
slots:
  main:
    provider: bifrost
    model: agent-primary
    base_url: {}/v1
"#,
        bifrost.uri()
    ))
    .expect("router config parses");
    let state = AppState::new(pool, redis, sandbox, search, router, Default::default());

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral test port");
    let addr = listener.local_addr().expect("read local test address");

    tokio::spawn(async move {
        // /v1/cost is now self-gated on loopback (issue #21), so the handler needs
        // ConnectInfo — serve with connect-info like the other guarded routes. The
        // request below originates from 127.0.0.1, satisfying require_loopback.
        axum::serve(
            listener,
            app(state).into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .expect("serve cost test app");
    });

    let resp = reqwest::get(format!("http://{addr}/v1/cost"))
        .await
        .expect("GET /v1/cost");

    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value = resp.json().await.expect("parse cost JSON");
    assert_eq!(body["total_cents"], 42);
    assert_eq!(body["currency"], "USD");
}
