use bytes::Bytes;
use serde::{Deserialize, Serialize};

use super::EventError;
use crate::sandbox::SandboxClient;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EventPayloadBody {
    Inline {
        bytes: Vec<u8>,
    },
    FileRef {
        path: String,
        content_type: String,
        sha256: String,
        size: u64,
    },
}

impl EventPayloadBody {
    pub async fn body_bytes(
        &self,
        sandbox: &SandboxClient,
        session_id: &str,
    ) -> Result<Bytes, EventError> {
        match self {
            Self::Inline { bytes } => Ok(Bytes::from(bytes.clone())),
            Self::FileRef { path, .. } => {
                let body = sandbox.read_workspace_file(session_id, path).await?;
                Ok(Bytes::from(body))
            }
        }
    }
}
