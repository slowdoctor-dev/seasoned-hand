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
        let err =
            normalize_workspace_relative_path(bad).expect_err(&format!("{bad} should be rejected"));
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
fn is_safe_session_id_accepts_and_rejects() {
    // Phase 4 security hardening iter-2 F1: canonical session-id validator
    // shared between intake (`is_safe_session_id` wrapper) and the sandbox
    // layer's `require_safe_session_id` guard.
    for good in &[
        "a1b2c3d4-1234-5678-9abc-def012345678",
        "session-abc",
        "x",
        // Length-64 boundary.
        "0123456789012345678901234567890123456789012345678901234567890123",
    ] {
        assert!(is_safe_session_id(good), "{good:?} must be accepted");
    }
    for bad in &[
        "",
        "../etc/passwd",
        "..",
        "with space",
        "has/slash",
        "has\\back",
        "$(whoami)",
        "x;rm",
        "session.with.dots",
        "session_underscore",
        // Length-65 just over boundary.
        "01234567890123456789012345678901234567890123456789012345678901234",
    ] {
        assert!(!is_safe_session_id(bad), "{bad:?} must be rejected");
    }
}

#[test]
fn require_safe_session_id_returns_invalid_workspace() {
    // The sandbox-layer guard returns the same error variant as the
    // path-normalizer so callers can pattern-match consistently.
    let err =
        require_safe_session_id("../etc").expect_err("path-traversal session id must be rejected");
    assert!(
        matches!(err, SandboxError::InvalidWorkspace(_)),
        "wrong variant: {err:?}",
    );
    assert!(require_safe_session_id("a1b2c3d4").is_ok());
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

// SEC-IT4-M2: untrusted code in the sandbox can plant a symlink in the
// bind-mounted workspace. The host-side read/write helpers must not follow it
// out of the workspace root.
#[cfg(unix)]
async fn client_with_workspace_and_secret() -> (tempfile::TempDir, SandboxClient, std::path::PathBuf)
{
    let tmp = tempfile::tempdir().unwrap();
    // A host file that lives OUTSIDE any session workspace.
    let secret = tmp.path().join("host-secret.txt");
    tokio::fs::write(&secret, b"TOP SECRET HOST FILE")
        .await
        .unwrap();
    // The workspace root is a subdir; the session workspace is root/s1.
    let root = tmp.path().join("ws-root");
    let ws = root.join("s1");
    tokio::fs::create_dir_all(&ws).await.unwrap();
    let client = SandboxClient::new("ghcr.io/agent-infra/sandbox:1.0.0.152", &root).unwrap();
    client
        .insert_handle_for_test(SandboxHandle {
            session_id: "s1".into(),
            container_id: "c1".into(),
            api_url: "http://127.0.0.1:1".into(),
            novnc_url: "http://127.0.0.1:2".into(),
            ttyd_url: "ws://127.0.0.1:3".into(),
            workspace_host_path: ws,
        })
        .await;
    (tmp, client, secret)
}

#[cfg(unix)]
#[tokio::test]
async fn read_workspace_file_rejects_symlink_escape() {
    let (tmp, client, secret) = client_with_workspace_and_secret().await;
    let ws = tmp.path().join("ws-root").join("s1");
    // Sandbox plants `s1/leak -> ../../host-secret.txt`.
    tokio::fs::symlink(&secret, ws.join("leak")).await.unwrap();

    let err = client
        .read_workspace_file("s1", "leak")
        .await
        .expect_err("symlink escape must be rejected, not followed");
    match err {
        SandboxError::InvalidWorkspace(msg) => assert!(msg.contains("escapes root")),
        other => panic!("expected InvalidWorkspace, got {other:?}"),
    }

    // A normal in-workspace file still reads fine.
    tokio::fs::write(ws.join("ok.txt"), b"hello").await.unwrap();
    let got = client.read_workspace_file("s1", "ok.txt").await.unwrap();
    assert_eq!(got, b"hello");
}

#[cfg(unix)]
#[tokio::test]
async fn write_workspace_file_refuses_to_write_through_symlink() {
    let (tmp, client, secret) = client_with_workspace_and_secret().await;
    let ws = tmp.path().join("ws-root").join("s1");
    // Sandbox plants `s1/leak -> ../../host-secret.txt`, then the control
    // plane is induced to write to "leak".
    tokio::fs::symlink(&secret, ws.join("leak")).await.unwrap();

    let err = client
        .write_workspace_file("s1", "leak", b"OVERWRITTEN")
        .await
        .expect_err("writing through a symlink must be rejected");
    match err {
        SandboxError::InvalidWorkspace(msg) => assert!(msg.contains("symlink")),
        other => panic!("expected InvalidWorkspace, got {other:?}"),
    }
    // The host file outside the workspace is untouched.
    let still = tokio::fs::read(&secret).await.unwrap();
    assert_eq!(still, b"TOP SECRET HOST FILE");

    // A normal in-workspace write still works.
    client
        .write_workspace_file("s1", "ok.txt", b"hi")
        .await
        .unwrap();
    assert_eq!(tokio::fs::read(ws.join("ok.txt")).await.unwrap(), b"hi");
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
