use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::{EventError, payload::EventPayloadBody};
use crate::sandbox::SandboxClient;

pub const INLINE_CAP_BYTES: usize = 16 * 1024;

pub fn extension_for(content_type: &str) -> &'static str {
    match content_type
        .split(';')
        .next()
        .unwrap_or(content_type)
        .trim()
    {
        "text/plain" => "txt",
        "text/markdown" => "md",
        "application/json" => "json",
        "image/png" => "png",
        "image/jpeg" => "jpg",
        _ => "bin",
    }
}

pub async fn write_large_or_inline(
    sandbox: &SandboxClient,
    session_id: &str,
    body: &[u8],
    content_type: &str,
) -> Result<EventPayloadBody, EventError> {
    if body.len() <= INLINE_CAP_BYTES {
        return Ok(EventPayloadBody::Inline {
            bytes: body.to_vec(),
        });
    }

    let ext = extension_for(content_type);
    let spill_id = Uuid::new_v4().simple();
    let path = format!("/workspace/.eventfiles/{spill_id}.{ext}");
    sandbox
        .write_workspace_file(session_id, &path, body)
        .await?;

    Ok(EventPayloadBody::FileRef {
        path,
        content_type: content_type.to_string(),
        sha256: format!("{:x}", Sha256::digest(body)),
        size: body.len() as u64,
    })
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;
    use crate::sandbox::{SandboxClient, SandboxHandle};

    async fn sandbox_with_session() -> (SandboxClient, tempfile::TempDir) {
        let root = tempdir().unwrap();
        let client =
            SandboxClient::new("ghcr.io/agent-infra/sandbox:1.0.0.152", root.path()).unwrap();
        let session_root = root.path().join("s1");
        std::fs::create_dir_all(&session_root).unwrap();
        client
            .insert_handle_for_test(SandboxHandle {
                session_id: "s1".into(),
                container_id: "test".into(),
                api_url: "http://127.0.0.1:1".into(),
                novnc_url: "http://127.0.0.1:2".into(),
                ttyd_url: "ws://127.0.0.1:3".into(),
                workspace_host_path: session_root,
            })
            .await;
        (client, root)
    }

    #[tokio::test]
    async fn small_payload_stays_inline() {
        let (sandbox, _tmp) = sandbox_with_session().await;
        let bytes = vec![7_u8; 1024];
        let payload = write_large_or_inline(&sandbox, "s1", &bytes, "application/json")
            .await
            .unwrap();
        assert_eq!(payload, EventPayloadBody::Inline { bytes });
    }

    #[tokio::test]
    async fn large_payload_writes_to_eventfiles() {
        let (sandbox, _tmp) = sandbox_with_session().await;
        let bytes = vec![9_u8; 100 * 1024];
        let payload = write_large_or_inline(&sandbox, "s1", &bytes, "application/octet-stream")
            .await
            .unwrap();
        let EventPayloadBody::FileRef { path, size, .. } = payload else {
            panic!("expected file ref");
        };
        assert!(path.starts_with("/workspace/.eventfiles/"));
        assert!(path.ends_with(".bin"));
        assert_eq!(size, 100 * 1024);
        let read_back = sandbox.read_workspace_file("s1", &path).await.unwrap();
        assert_eq!(read_back, bytes);
    }

    #[test]
    fn extension_derived_from_content_type() {
        assert_eq!(extension_for("text/plain"), "txt");
        assert_eq!(extension_for("application/json; charset=utf-8"), "json");
        assert_eq!(extension_for("image/png"), "png");
        assert_eq!(extension_for("image/jpeg"), "jpg");
        assert_eq!(extension_for("application/octet-stream"), "bin");
    }

    #[tokio::test]
    async fn body_bytes_round_trips_fileref() {
        let (sandbox, _tmp) = sandbox_with_session().await;
        let bytes = vec![1_u8; 24 * 1024];
        let payload = write_large_or_inline(&sandbox, "s1", &bytes, "application/json")
            .await
            .unwrap();
        let round_trip = payload.body_bytes(&sandbox, "s1").await.unwrap();
        assert_eq!(round_trip.as_ref(), bytes.as_slice());
    }

    #[tokio::test]
    async fn eventfile_path_uses_unique_spill_id() {
        let (sandbox, _tmp) = sandbox_with_session().await;
        let bytes = vec![2_u8; 20 * 1024];
        let a = write_large_or_inline(&sandbox, "s1", &bytes, "application/octet-stream")
            .await
            .unwrap();
        let b = write_large_or_inline(&sandbox, "s1", &bytes, "application/octet-stream")
            .await
            .unwrap();

        let EventPayloadBody::FileRef { path: a_path, .. } = a else {
            panic!("expected file ref");
        };
        let EventPayloadBody::FileRef { path: b_path, .. } = b else {
            panic!("expected file ref");
        };
        assert!(a_path.starts_with("/workspace/.eventfiles/"));
        assert!(b_path.starts_with("/workspace/.eventfiles/"));
        assert!(a_path.ends_with(".bin"));
        assert!(b_path.ends_with(".bin"));
        assert_ne!(a_path, b_path);
    }

    #[tokio::test]
    async fn concurrent_large_payloads_get_unique_paths() {
        let (sandbox, _tmp) = sandbox_with_session().await;
        let bytes = vec![3_u8; 20 * 1024];
        let writes = (0..32)
            .map(|_| write_large_or_inline(&sandbox, "s1", &bytes, "application/octet-stream"));
        let payloads = futures::future::try_join_all(writes).await.unwrap();
        let mut paths = std::collections::BTreeSet::new();
        for payload in payloads {
            let EventPayloadBody::FileRef { path, .. } = payload else {
                panic!("expected file ref");
            };
            assert!(
                paths.insert(path),
                "concurrent spill writes must not reuse a predicted event id"
            );
        }
        assert_eq!(paths.len(), 32);
    }
}
