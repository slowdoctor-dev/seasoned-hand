//! refs: /specs/phase-0/architecture.md §3.2, §3.4, §4.1

use axum::http::StatusCode;
use seasoned_hand_core::events::{EventStore, EventType, NewEvent};
use seasoned_hand_core::{db, pubsub};
use seasoned_hand_server::{AppState, app};
use serde_json::json;
use tokio::net::TcpListener;

fn auth_client() -> reqwest::Client {
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        "x-seasoned-hand-tenant-id",
        "legacy-default".parse().unwrap(),
    );
    headers.insert(
        "x-seasoned-hand-organization-id",
        "org-legacy-default".parse().unwrap(),
    );
    headers.insert(
        "x-seasoned-hand-actor-user-id",
        "user-admin".parse().unwrap(),
    );
    headers.insert("x-seasoned-hand-org-role", "admin".parse().unwrap());
    reqwest::Client::builder()
        .default_headers(headers)
        .build()
        .unwrap()
}

async fn boot() -> (String, AppState) {
    let pool = db::open(":memory:").await.unwrap();
    pool.with_conn(|conn| {
        conn.execute(
            "INSERT INTO projects (id, tenant_id, title, status, created_at, updated_at) \
             VALUES ('p1', 'legacy-default', 'proj', 'active', 1, 1)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO sessions (id, project_id, created_at, updated_at, state) \
             VALUES ('s1', 'p1', 1, 1, 'RUNNING')",
            [],
        )
        .unwrap();
    })
    .await;
    // Tests use an unreachable redis URL — append() will log a publish-failure
    // warning but still succeed (PRINCIPLE #10 failure-tolerance).
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
    let state = AppState::new(pool, redis, sandbox, search, router, Default::default())
        .allow_insecure_auth_headers();

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let serve_state = state.clone();
    tokio::spawn(async move {
        axum::serve(
            listener,
            app(serve_state).into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .unwrap();
    });

    (format!("http://{addr}"), state)
}

#[tokio::test]
async fn list_events_returns_appended_events() {
    let (base, state) = boot().await;
    let client = auth_client();
    state
        .events
        .append(NewEvent {
            session_id: "s1".into(),
            event_type: EventType::Message,
            source: "user".into(),
            data: json!({"content": "hello"}),
        })
        .await
        .unwrap();
    state
        .events
        .append(NewEvent {
            session_id: "s1".into(),
            event_type: EventType::Action,
            source: "agent".into(),
            data: json!({"tool": "ping"}),
        })
        .await
        .unwrap();

    let resp = client
        .get(format!("{base}/v1/sessions/s1/events"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value = resp.json().await.unwrap();
    let arr = body.as_array().unwrap();
    assert_eq!(arr.len(), 2);
    assert_eq!(arr[0]["type"], "Message");
    assert_eq!(arr[1]["type"], "Action");
    assert_eq!(arr[0]["data"]["content"], "hello");
}

#[tokio::test]
async fn list_events_filters_by_type() {
    let (base, state) = boot().await;
    let client = auth_client();
    for t in [EventType::Message, EventType::Action, EventType::Action] {
        state
            .events
            .append(NewEvent {
                session_id: "s1".into(),
                event_type: t,
                source: "x".into(),
                data: json!({}),
            })
            .await
            .unwrap();
    }
    let resp = client
        .get(format!("{base}/v1/sessions/s1/events?type=Action"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body.as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn list_events_404_for_unknown_session() {
    let (base, _state) = boot().await;
    let client = auth_client();
    let resp = client
        .get(format!("{base}/v1/sessions/nope/events"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["error"], "session_not_found");
}

#[tokio::test]
async fn list_events_400_for_unknown_type() {
    let (base, _state) = boot().await;
    let client = auth_client();
    let resp = client
        .get(format!("{base}/v1/sessions/s1/events?type=Bogus"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}
