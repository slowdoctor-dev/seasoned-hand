use axum::http::StatusCode;
use seasoned_hand_core::{db, pubsub};
use seasoned_hand_server::{AppState, app};
use serde_json::json;
use tokio::net::TcpListener;

async fn boot() -> String {
    let pool = db::open(":memory:").await.unwrap();
    pool.with_conn(|conn| {
        conn.execute(
            "INSERT INTO organizations (id, tenant_id, slug, display_name, status, created_at, updated_at)
             VALUES ('org-a', 'tenant-a', 'org-a', 'Org A', 'active', 1, 1)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO users (id, tenant_id, email, display_name, status, created_at, updated_at)
             VALUES ('u-admin', 'tenant-a', 'admin@acme.dev', 'Admin', 'active', 1, 1)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO users (id, tenant_id, email, display_name, status, created_at, updated_at)
             VALUES ('u-owner', 'tenant-a', 'owner@acme.dev', 'Owner', 'active', 1, 1)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO users (id, tenant_id, email, display_name, status, created_at, updated_at)
             VALUES ('u-viewer', 'tenant-a', 'viewer@acme.dev', 'Viewer', 'active', 1, 1)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO sops (id, title, content, version, enforced, created_at, updated_at)
             VALUES ('sop-1', 'Deploy', 'Checklist', 1, 1, 1, 1)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO sop_shares (id, tenant_id, sop_id, subject_type, subject_id, permission, granted_by_user_id, created_at, updated_at)
             VALUES ('seed-owner', 'tenant-a', 'sop-1', 'user', 'u-owner', 'owner', 'u-owner', 1, 1)",
            [],
        )
        .unwrap();
    })
    .await;

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

fn auth_client(actor_user_id: &str, role: &str) -> reqwest::Client {
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert("x-seasoned-hand-tenant-id", "tenant-a".parse().unwrap());
    headers.insert("x-seasoned-hand-organization-id", "org-a".parse().unwrap());
    headers.insert(
        "x-seasoned-hand-actor-user-id",
        actor_user_id.parse().unwrap(),
    );
    headers.insert("x-seasoned-hand-org-role", role.parse().unwrap());
    reqwest::Client::builder()
        .default_headers(headers)
        .build()
        .unwrap()
}

#[tokio::test]
async fn viewer_cannot_share_sop() {
    let base = boot().await;
    let client = auth_client("u-viewer", "viewer");
    let resp = client
        .post(format!("{base}/v1/sops/sop-1/shares"))
        .json(&json!({
            "user_email": "viewer@acme.dev",
            "permission": "owner"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn admin_can_override_grant() {
    let base = boot().await;
    let client = auth_client("u-admin", "admin");
    let resp = client
        .post(format!("{base}/v1/sops/sop-1/shares"))
        .json(&json!({
            "user_email": "viewer@acme.dev",
            "permission": "editor"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body: serde_json::Value = client
        .get(format!("{base}/v1/sops/sop-1/shares"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let arr = body.as_array().unwrap();
    let row = arr
        .iter()
        .find(|v| v["subject_email"] == "viewer@acme.dev")
        .expect("viewer share row");
    assert_eq!(row["permission"], "editor");
}
