//! Phase 5 story 5.29 (part 1) — `phase5_event_redaction_visibility_harness`.
//!
//! Verifies the load-bearing security carry-forward closed by story
//! 5.14 (DEBT #S-1 / SECURITY_REVIEW iter-3): tenant-visible event
//! feeds must return REDACTED payloads, and the admin-only raw-event
//! route must surface the originals + write an audit_log row.
//!
//! The seeded events carry every PII pattern probed during the Phase 4
//! security-hardening iteration:
//!
//! - PEM private key block
//! - Authorization header (`Bearer ...`)
//! - IPv6 address
//! - Email address
//!
//! Each must emerge as a `[REDACTED_*]` marker in `tenant_event_view.
//! redacted_data` and disappear from `searchable_text`. The admin raw
//! read MUST see the originals AND emit one `event.raw_read` audit
//! row per call.
//!
//! refs: /specs/phase-5/stories/story-5.29.md
//! refs: /specs/phase-5/architecture.md §15 harness 4
//! refs: /specs/phase-5/requirements.md F-5.11, F-5.12, NFR-5.6
//! debt verified: #S-1

use rusqlite::params;
use seasoned_hand_core::audit::AuditLogger;
use seasoned_hand_core::auth::{AuthContext, Role};
use seasoned_hand_core::db::{self, DbPool};
use seasoned_hand_core::events::sqlite::SqliteEventStore;
use seasoned_hand_core::events::visibility::{self, EventReadQuery};
use seasoned_hand_core::events::{EventStore, EventType, NewEvent};
use std::sync::Arc;

async fn seed_tenant_and_session(pool: &DbPool) {
    pool.with_conn(|conn| {
        conn.execute(
            "INSERT INTO organizations (id, tenant_id, slug, display_name, status,
                                         created_at, updated_at)
             VALUES ('org-a', 'tenant-a', 'org-a', 'A', 'active', 0, 0)",
            [],
        )?;
        for (uid, email) in [
            ("user-admin", "admin@x.io"),
            ("user-user", "user@x.io"),
            ("user-viewer", "viewer@x.io"),
        ] {
            conn.execute(
                "INSERT INTO users (id, tenant_id, email, display_name, status,
                                    created_at, updated_at)
                 VALUES (?, 'tenant-a', ?, 'X', 'active', 0, 0)",
                params![uid, email],
            )?;
        }
        conn.execute(
            "INSERT INTO projects (id, tenant_id, title, status, created_at, updated_at)
             VALUES ('proj-a', 'tenant-a', 'P', 'active', 0, 0)",
            [],
        )?;
        conn.execute(
            "INSERT INTO tasks (id, project_id, tenant_id, owner_user_id, title,
                                status, created_at, updated_at)
             VALUES ('task-a', 'proj-a', 'tenant-a', NULL, 'T', 'drafted', 0, 0)",
            [],
        )?;
        conn.execute(
            "INSERT INTO sessions (id, created_at, updated_at, state, task_id)
             VALUES ('sess-a', 0, 0, 'IDLE', 'task-a')",
            [],
        )?;
        Ok::<(), rusqlite::Error>(())
    })
    .await
    .unwrap();
}

fn ctx(role: Role, actor: &str) -> AuthContext {
    AuthContext {
        tenant_id: "tenant-a".into(),
        organization_id: "org-a".into(),
        actor_user_id: actor.into(),
        org_role: role,
        project_override_role: None,
    }
}

#[tokio::test]
async fn phase5_event_redaction_visibility_harness() {
    let pool = db::open(":memory:").await.expect("open db");
    seed_tenant_and_session(&pool).await;
    let events = Arc::new(SqliteEventStore::new(pool.clone()));
    let audit = AuditLogger::new(pool.clone(), events.clone());

    // ---------- Seed events with every PII pattern ----------
    let pem = "-----BEGIN RSA PRIVATE KEY-----\nABCDEFGHabcdefgh1234567890+/==\n-----END RSA PRIVATE KEY-----";
    let auth_header = "Authorization: Bearer sk_live_redacttest9999000";
    let ipv6 = "2001:0db8:85a3:0000:0000:8a2e:0370:7334";
    let email_addr = "victim@leak.example";

    events
        .append(NewEvent {
            session_id: "sess-a".into(),
            event_type: EventType::Observation,
            source: "tool".into(),
            data: serde_json::json!({
                "tool_name": "ssh",
                "tool_result": pem,
            }),
        })
        .await
        .expect("append pem");
    events
        .append(NewEvent {
            session_id: "sess-a".into(),
            event_type: EventType::Action,
            source: "tool".into(),
            data: serde_json::json!({
                "tool_name": "curl",
                "tool_input": {"headers": auth_header},
            }),
        })
        .await
        .expect("append auth");
    events
        .append(NewEvent {
            session_id: "sess-a".into(),
            event_type: EventType::Action,
            source: "tool".into(),
            data: serde_json::json!({
                "tool_name": "curl",
                "tool_input": {"target": ipv6},
            }),
        })
        .await
        .expect("append ipv6");
    events
        .append(NewEvent {
            session_id: "sess-a".into(),
            event_type: EventType::Message,
            source: "user".into(),
            data: serde_json::json!({"text": format!("contact me at {email_addr}")}),
        })
        .await
        .expect("append email");

    // ---------- Tenant-visible read MUST be redacted ----------
    let visible = visibility::query(
        &pool,
        &ctx(Role::User, "user-user"),
        "sess-a",
        EventReadQuery::default(),
    )
    .await
    .expect("query visible");

    // For each row, the redacted_data MUST NOT contain the original
    // sensitive substring, and MUST contain the appropriate marker.
    let blob: String = visible
        .iter()
        .map(|r| r.redacted_data.clone())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !blob.contains("BEGIN RSA PRIVATE KEY"),
        "redacted feed must not surface PEM body"
    );
    assert!(
        blob.contains("[REDACTED_PRIVATE_KEY]"),
        "redacted feed must include the PEM marker"
    );
    assert!(
        !blob.contains("sk_live_redacttest"),
        "redacted feed must not surface Bearer token body"
    );
    assert!(
        blob.contains("[REDACTED_AUTH_HEADER]"),
        "redacted feed must include the auth header marker"
    );
    assert!(
        !blob.contains("2001:0db8:85a3"),
        "redacted feed must not surface IPv6"
    );
    assert!(
        blob.contains("[REDACTED_IP]"),
        "redacted feed must include the IP marker"
    );
    assert!(
        !blob.contains("victim@leak.example"),
        "redacted feed must not surface email"
    );
    assert!(
        blob.contains("[REDACTED_EMAIL]"),
        "redacted feed must include the email marker"
    );

    // ---------- Admin raw read MUST see originals + emit audit ----------
    let raw = visibility::query_raw(
        &pool,
        &ctx(Role::Admin, "user-admin"),
        &audit,
        "sess-a",
        EventReadQuery::default(),
    )
    .await
    .expect("query_raw admin");
    let raw_blob: String = raw
        .iter()
        .map(|r| r.data.clone())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        raw_blob.contains("BEGIN RSA PRIVATE KEY")
            && raw_blob.contains("sk_live_redacttest")
            && raw_blob.contains("2001:0db8:85a3")
            && raw_blob.contains("victim@leak.example"),
        "admin raw read must surface every original payload",
    );
    // Audit row emitted with the `event.raw_read` action.
    let audit_n: i64 = pool
        .with_conn(|conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM audit_log
                 WHERE action = 'event.raw_read' AND resource_id = 'sess-a'",
                [],
                |r| r.get(0),
            )
        })
        .await
        .unwrap();
    assert_eq!(
        audit_n, 1,
        "every admin raw-read call must emit exactly one audit row"
    );

    // ---------- Viewer is denied at the policy gate ----------
    let viewer_err = visibility::query_raw(
        &pool,
        &ctx(Role::Viewer, "user-viewer"),
        &audit,
        "sess-a",
        EventReadQuery::default(),
    )
    .await
    .expect_err("viewer must be denied raw-read");
    // Should be an Auth error variant.
    assert!(format!("{viewer_err:?}").contains("Auth"));

    // ---------- User role is denied at the policy gate ----------
    let user_err = visibility::query_raw(
        &pool,
        &ctx(Role::User, "user-user"),
        &audit,
        "sess-a",
        EventReadQuery::default(),
    )
    .await
    .expect_err("user role must be denied raw-read");
    assert!(format!("{user_err:?}").contains("Auth"));
}
