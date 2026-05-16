use std::time::Duration;

use super::*;

#[test]
fn container_name_pattern() {
    assert_eq!(
        container_name("session-abc"),
        "seasoned-hand-sandbox-session-abc"
    );
}

#[test]
fn ports_constant_match_aio_sandbox_defaults() {
    // Sanity-checks against architecture §5.2.
    assert_eq!(PORT_API, 8080);
    assert_eq!(PORT_NOVNC, 6080);
    assert_eq!(PORT_TTYD, 7681);
}

#[test]
fn normalize_workspace_relative_path_strips_prefix_and_blocks_traversal() {
    // Accept: normal workspace-relative paths.
    assert_eq!(
        normalize_workspace_relative_path("foo/bar.txt").unwrap(),
        "foo/bar.txt"
    );
    assert_eq!(
        normalize_workspace_relative_path("/workspace/foo/bar.txt").unwrap(),
        "foo/bar.txt"
    );
    assert_eq!(
        normalize_workspace_relative_path("workspace/foo/bar.txt").unwrap(),
        "foo/bar.txt"
    );
    assert_eq!(
        normalize_workspace_relative_path("/foo/bar.txt").unwrap(),
        "foo/bar.txt"
    );

    // Reject: any `..` segment, regardless of position.
    for bad in &[
        "../etc/passwd",
        "/workspace/../etc/passwd",
        "foo/../../etc",
        "foo/..",
        "..",
        "workspace/../bar",
    ] {
        let err = normalize_workspace_relative_path(bad)
            .expect_err(&format!("{bad} should be rejected"));
        assert!(
            matches!(err, SandboxError::InvalidWorkspace(_)),
            "{bad} → wrong error variant: {err:?}"
        );
    }

    // Reject: null byte (Rust's Path silently accepts; underlying OS
    // calls would truncate at the NUL).
    let err = normalize_workspace_relative_path("foo\0bar.txt")
        .expect_err("null byte should be rejected");
    assert!(matches!(err, SandboxError::InvalidWorkspace(_)));
}

#[test]
fn new_client_records_image_and_workspace() {
    let tmp = tempfile::tempdir().unwrap();
    let client = SandboxClient::new(
        "ghcr.io/agent-infra/sandbox:1.0.0.152",
        tmp.path().to_path_buf(),
    )
    .expect("client init");
    assert_eq!(client.image(), "ghcr.io/agent-infra/sandbox:1.0.0.152");
    assert_eq!(client.workspace_root(), &tmp.path().to_path_buf());
}

#[tokio::test]
async fn get_returns_none_for_unknown_session() {
    let tmp = tempfile::tempdir().unwrap();
    let client = SandboxClient::new(
        "ghcr.io/agent-infra/sandbox:1.0.0.152",
        tmp.path().to_path_buf(),
    )
    .unwrap();
    assert!(client.get("nope").await.is_none());
}

// ============================================================================
// Live lifecycle test — requires Docker, pulls the pinned image once,
// runs a full create-inspect-destroy cycle.
//
// Skip in CI without Docker. Run locally with:
//   docker info >/dev/null && cargo test -- --ignored sandbox::tests::live
// ============================================================================

#[tokio::test]
#[ignore = "requires Docker + pulls ~1GB aio-sandbox image"]
async fn live_create_inspect_destroy() {
    let tmp = tempfile::tempdir().unwrap();
    let client = SandboxClient::new(
        "ghcr.io/agent-infra/sandbox:1.0.0.152",
        tmp.path().to_path_buf(),
    )
    .expect("client");

    let session = format!("test-{}", uuid_suffix());

    // Pre-clean (in case a prior failed run left a container around).
    let _ = client.destroy(&session).await;

    let handle = client.create(&session).await.expect("create container");
    assert!(handle.container_id.len() > 8);
    assert!(handle.api_url.starts_with("http://127.0.0.1:"));
    assert!(handle.novnc_url.starts_with("http://127.0.0.1:"));
    assert!(handle.ttyd_url.starts_with("ws://127.0.0.1:"));
    assert!(handle.workspace_host_path.exists());

    // Cached lookup.
    let cached = client.get(&session).await.expect("cached handle");
    assert_eq!(cached.container_id, handle.container_id);

    // Container is reachable on the API port (TCP connect — full HTTP
    // readiness is the sandbox's own concern + story 0.9).
    let api_addr = handle.api_url.trim_start_matches("http://").to_string();
    let mut connected = false;
    for _ in 0..30 {
        if tokio::net::TcpStream::connect(&api_addr).await.is_ok() {
            connected = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    assert!(
        connected,
        "sandbox API never became reachable at {api_addr}"
    );

    client.destroy(&session).await.expect("destroy");
    assert!(client.get(&session).await.is_none());

    // Idempotent.
    client.destroy(&session).await.expect("destroy idempotent");
}

fn uuid_suffix() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!("{nanos:x}")
}
