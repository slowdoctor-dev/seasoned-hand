//! Sandbox handle-cache rehydration.
//! refs: /specs/phase-1/stories/story-1.2.md
//! refs: /specs/phase-1/architecture.md §6 row "Sandbox handle cache (DEBT #18)"
//! refs: /specs/phase-0/DEBT.md #18

/// Outcome of a single rehydration pass.
///
/// `restored` counts containers whose corresponding `sessions` row was in a
/// live state ({IDLE, RUNNING, SUSPENDED, VERIFYING}) and were re-registered
/// in the handle cache. `orphans` counts containers whose session was absent
/// or in a terminal state ({FINISHED, ERROR}) — they are left running for
/// Phase 0 DEBT #16 to clean up. `errors` collects per-container failures
/// (DB lookup or inspect failure) without aborting the whole pass.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct RehydrateReport {
    pub restored: usize,
    pub orphans: usize,
    pub errors: Vec<String>,
}

pub(super) const SANDBOX_CONTAINER_PREFIX: &str = "seasoned-hand-sandbox-";

/// Extract the session id suffix from a container name as Docker reports it
/// (with or without the leading `/`). Returns `None` when the name does not
/// match our `seasoned-hand-sandbox-*` pattern.
pub(super) fn extract_session_id_from_name(name: &str) -> Option<&str> {
    name.trim_start_matches('/')
        .strip_prefix(SANDBOX_CONTAINER_PREFIX)
        .filter(|s| !s.is_empty())
}

/// Whether a `sessions.state` value means "container should still be live".
/// `VERIFYING` is the Phase 1 state added by story 1.10; including it here
/// is a value-only match against the DB so it does not require the migration
/// to have landed.
pub(super) fn is_live_state(state: Option<&str>) -> bool {
    matches!(
        state,
        Some("IDLE") | Some("RUNNING") | Some("SUSPENDED") | Some("VERIFYING")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    // ------------------------------------------------------------------
    // Pure-unit tests — run on every `cargo test`, no Docker needed.
    // ------------------------------------------------------------------

    #[test]
    fn extract_session_id_from_name_strips_prefix_and_slash() {
        assert_eq!(
            extract_session_id_from_name("/seasoned-hand-sandbox-session-abc"),
            Some("session-abc")
        );
        assert_eq!(
            extract_session_id_from_name("seasoned-hand-sandbox-xyz"),
            Some("xyz")
        );
    }

    #[test]
    fn extract_session_id_from_name_rejects_non_matching() {
        assert_eq!(extract_session_id_from_name("some-other-container"), None);
        assert_eq!(
            extract_session_id_from_name("/seasoned-hand-sandbox-"),
            None
        );
        assert_eq!(extract_session_id_from_name(""), None);
    }

    #[test]
    fn is_live_state_matches_running_idle_suspended_verifying() {
        assert!(is_live_state(Some("IDLE")));
        assert!(is_live_state(Some("RUNNING")));
        assert!(is_live_state(Some("SUSPENDED")));
        assert!(is_live_state(Some("VERIFYING")));
    }

    #[test]
    fn is_live_state_rejects_finished_error_missing() {
        assert!(!is_live_state(Some("FINISHED")));
        assert!(!is_live_state(Some("ERROR")));
        assert!(!is_live_state(Some("BOGUS")));
        assert!(!is_live_state(None));
    }

    #[test]
    fn rehydrate_report_default_is_empty() {
        let r = RehydrateReport::default();
        assert_eq!(r.restored, 0);
        assert_eq!(r.orphans, 0);
        assert!(r.errors.is_empty());
    }

    // ------------------------------------------------------------------
    // Docker integration — gated behind `RUN_DOCKER_TESTS=1` like Phase 0's
    // `live_create_inspect_destroy`. Acceptance-criteria tests from story
    // 1.2.
    // ------------------------------------------------------------------

    use crate::db;
    use crate::sandbox::{SandboxClient, container_name};
    use bollard::Docker;
    use bollard::container::RemoveContainerOptions;

    fn docker_available() -> bool {
        std::env::var("RUN_DOCKER_TESTS").as_deref() == Ok("1")
    }

    async fn open_test_db() -> db::DbPool {
        db::open(":memory:")
            .await
            .expect("open in-memory sqlite for rehydrate tests")
    }

    async fn insert_session(pool: &db::DbPool, id: &str, state: &str) {
        let id = id.to_string();
        let state = state.to_string();
        pool.with_conn(move |conn| {
            conn.execute(
                "INSERT INTO sessions (id, created_at, updated_at, state) \
                 VALUES (?, 0, 0, ?)",
                rusqlite::params![id, state],
            )
            .expect("insert session row");
        })
        .await;
    }

    async fn force_remove(docker: &Docker, name: &str) {
        let _ = docker
            .remove_container(
                name,
                Some(RemoveContainerOptions {
                    force: true,
                    v: false,
                    link: false,
                }),
            )
            .await;
    }

    fn nanos_suffix() -> String {
        use std::time::{SystemTime, UNIX_EPOCH};
        format!(
            "{:x}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        )
    }

    #[tokio::test]
    #[ignore = "requires Docker (set RUN_DOCKER_TESTS=1)"]
    async fn rehydrate_with_no_containers_reports_zero() {
        if !docker_available() {
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let client = SandboxClient::new(
            "ghcr.io/agent-infra/sandbox:1.0.0.152",
            tmp.path().to_path_buf(),
        )
        .expect("client init");
        let pool = open_test_db().await;
        let report = client
            .rehydrate_from_docker(&pool)
            .await
            .expect("rehydrate ok");
        // Zero matching containers ⇒ everything zero. Other unrelated
        // containers on the host are filtered by name prefix.
        assert_eq!(report.restored, 0);
        assert_eq!(report.orphans, 0);
        assert!(report.errors.is_empty(), "errors: {:?}", report.errors);
    }

    #[tokio::test]
    #[ignore = "requires Docker (set RUN_DOCKER_TESTS=1); pulls ~1GB image"]
    async fn rehydrate_with_two_containers_one_with_session_one_without() {
        if !docker_available() {
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let client = SandboxClient::new(
            "ghcr.io/agent-infra/sandbox:1.0.0.152",
            tmp.path().to_path_buf(),
        )
        .expect("client init");
        let pool = open_test_db().await;

        let suffix = nanos_suffix();
        let live_session = format!("rehydrate-live-{suffix}");
        let orphan_session = format!("rehydrate-orphan-{suffix}");

        // Pre-clean.
        let docker = Docker::connect_with_local_defaults().expect("docker");
        force_remove(&docker, &container_name(&live_session)).await;
        force_remove(&docker, &container_name(&orphan_session)).await;

        // Create two real containers via SandboxClient::create.
        let _live = client
            .create(&live_session)
            .await
            .expect("create live container");
        let _orphan = client
            .create(&orphan_session)
            .await
            .expect("create orphan container");

        // Only the live session has a matching sessions row.
        insert_session(&pool, &live_session, "RUNNING").await;

        // Drop the in-process cache to simulate a restart: rebuild a fresh
        // client against the same Docker daemon, then rehydrate.
        drop(client);
        let client = SandboxClient::new(
            "ghcr.io/agent-infra/sandbox:1.0.0.152",
            tmp.path().to_path_buf(),
        )
        .expect("client re-init");

        let report = client
            .rehydrate_from_docker(&pool)
            .await
            .expect("rehydrate ok");
        assert_eq!(report.restored, 1, "report: {report:?}");
        assert_eq!(report.orphans, 1, "report: {report:?}");
        assert!(report.errors.is_empty(), "errors: {:?}", report.errors);
        assert!(
            client.get(&live_session).await.is_some(),
            "live session should be cached after rehydrate"
        );
        assert!(
            client.get(&orphan_session).await.is_none(),
            "orphan session should NOT be cached after rehydrate"
        );

        // Teardown.
        force_remove(&docker, &container_name(&live_session)).await;
        force_remove(&docker, &container_name(&orphan_session)).await;
    }

    #[tokio::test]
    #[ignore = "requires Docker (set RUN_DOCKER_TESTS=1); pulls ~1GB image"]
    async fn rehydrate_is_idempotent() {
        if !docker_available() {
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let client = SandboxClient::new(
            "ghcr.io/agent-infra/sandbox:1.0.0.152",
            tmp.path().to_path_buf(),
        )
        .expect("client init");
        let pool = open_test_db().await;

        let session = format!("rehydrate-idem-{}", nanos_suffix());
        let docker = Docker::connect_with_local_defaults().expect("docker");
        force_remove(&docker, &container_name(&session)).await;

        let _h = client.create(&session).await.expect("create container");
        insert_session(&pool, &session, "RUNNING").await;

        // Restart the client to clear the cache.
        drop(client);
        let client = SandboxClient::new(
            "ghcr.io/agent-infra/sandbox:1.0.0.152",
            tmp.path().to_path_buf(),
        )
        .expect("client re-init");

        let first = client
            .rehydrate_from_docker(&pool)
            .await
            .expect("first rehydrate");
        assert_eq!(first.restored, 1, "first: {first:?}");
        assert_eq!(first.orphans, 0);
        assert!(first.errors.is_empty());

        let second = client
            .rehydrate_from_docker(&pool)
            .await
            .expect("second rehydrate");
        assert_eq!(second.restored, 0, "second: {second:?}");
        assert_eq!(second.orphans, 0);
        assert!(second.errors.is_empty());

        force_remove(&docker, &container_name(&session)).await;
    }
}
