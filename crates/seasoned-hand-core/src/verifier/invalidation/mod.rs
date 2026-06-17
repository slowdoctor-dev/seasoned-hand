use std::collections::{HashMap, VecDeque};
use std::path::{Component, Path, PathBuf};

use dashmap::DashMap;
use sha2::{Digest, Sha256};

use crate::verifier::InvalidationReason;

pub const DEFAULT_MAX_PATHS: usize = 10_000;

#[derive(Clone)]
struct SessionHashes {
    order: VecDeque<PathBuf>,
    map: HashMap<PathBuf, [u8; 32]>,
}

impl SessionHashes {
    fn with_capacity(capacity: usize) -> Self {
        Self {
            order: VecDeque::with_capacity(capacity.min(1024)),
            map: HashMap::new(),
        }
    }

    fn upsert(&mut self, path: PathBuf, sha: [u8; 32], capacity: usize) {
        let is_new = self.map.insert(path.clone(), sha).is_none();
        if is_new {
            self.order.push_back(path);
            while self.map.len() > capacity {
                if let Some(oldest) = self.order.pop_front() {
                    self.map.remove(&oldest);
                } else {
                    break;
                }
            }
        }
    }
}

pub struct InvalidationDetector {
    sessions: DashMap<String, tokio::sync::Mutex<SessionHashes>>,
    capacity: usize,
}

impl InvalidationDetector {
    pub fn new(capacity: usize) -> Self {
        Self {
            sessions: DashMap::new(),
            capacity,
        }
    }

    pub fn normalize_path(path: &str) -> PathBuf {
        let mut out = PathBuf::new();
        for comp in Path::new(path).components() {
            match comp {
                Component::CurDir => {}
                Component::ParentDir => {
                    out.pop();
                }
                Component::RootDir | Component::Prefix(_) | Component::Normal(_) => {
                    out.push(comp.as_os_str())
                }
            }
        }
        out
    }

    pub fn hash_hex(body: &[u8]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(body);
        let digest: [u8; 32] = hasher.finalize().into();
        digest.iter().map(|b| format!("{b:02x}")).collect()
    }

    pub async fn observe(
        &self,
        session_id: &str,
        tool_name: &str,
        path: PathBuf,
        body: &[u8],
    ) -> Option<InvalidationReason> {
        let allow_listed = matches!(tool_name, "file_write" | "file_str_replace");
        let mut hasher = Sha256::new();
        hasher.update(body);
        let sha: [u8; 32] = hasher.finalize().into();

        let entry = self
            .sessions
            .entry(session_id.to_string())
            .or_insert_with(|| {
                tokio::sync::Mutex::new(SessionHashes::with_capacity(self.capacity))
            });
        let mut guard = entry.lock().await;

        let old = guard.map.get(&path).copied();
        guard.upsert(path.clone(), sha, self.capacity);

        match old {
            Some(old_sha) if old_sha != sha && !allow_listed => {
                Some(InvalidationReason::FileMismatch {
                    path,
                    old_sha: old_sha.iter().map(|b| format!("{b:02x}")).collect(),
                    new_sha: sha.iter().map(|b| format!("{b:02x}")).collect(),
                })
            }
            _ => None,
        }
    }

    pub fn evict_session(&self, session_id: &str) {
        self.sessions.remove(session_id);
    }

    #[cfg(test)]
    async fn has_path(&self, session_id: &str, path: &Path) -> bool {
        let Some(entry) = self.sessions.get(session_id) else {
            return false;
        };
        let guard = entry.lock().await;
        guard.map.contains_key(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn hash_round_trip() {
        let a = InvalidationDetector::hash_hex(b"hello");
        let b = InvalidationDetector::hash_hex(b"hello");
        assert_eq!(a, b);
    }

    #[tokio::test]
    async fn first_observation_does_not_trigger() {
        let d = InvalidationDetector::new(DEFAULT_MAX_PATHS);
        let got = d
            .observe("s1", "file_read", PathBuf::from("/workspace/a.txt"), b"v1")
            .await;
        assert!(got.is_none());
    }

    #[tokio::test]
    async fn same_content_subsequent_observation_does_not_trigger() {
        let d = InvalidationDetector::new(DEFAULT_MAX_PATHS);
        let path = PathBuf::from("/workspace/a.txt");
        let _ = d.observe("s1", "file_read", path.clone(), b"v1").await;
        let got = d.observe("s1", "file_read", path, b"v1").await;
        assert!(got.is_none());
    }

    #[tokio::test]
    async fn different_content_via_allow_listed_tool_does_not_trigger() {
        let d = InvalidationDetector::new(DEFAULT_MAX_PATHS);
        let path = PathBuf::from("/workspace/a.txt");
        let _ = d.observe("s1", "file_read", path.clone(), b"v1").await;
        let got = d.observe("s1", "file_write", path.clone(), b"v2").await;
        assert!(got.is_none());
        let got_after = d.observe("s1", "file_read", path, b"v2").await;
        assert!(
            got_after.is_none(),
            "allow-listed write must still update stored hash"
        );
    }

    #[tokio::test]
    async fn eviction_at_capacity() {
        let d = InvalidationDetector::new(2);
        let p1 = PathBuf::from("/workspace/1.txt");
        let p2 = PathBuf::from("/workspace/2.txt");
        let p3 = PathBuf::from("/workspace/3.txt");
        let _ = d.observe("s1", "file_read", p1.clone(), b"1").await;
        let _ = d.observe("s1", "file_read", p2.clone(), b"2").await;
        let _ = d.observe("s1", "file_read", p3.clone(), b"3").await;
        assert!(!d.has_path("s1", &p1).await);
        assert!(d.has_path("s1", &p2).await);
        assert!(d.has_path("s1", &p3).await);
    }

    #[tokio::test]
    async fn evict_session_drops_all_hashes_for_session() {
        let d = InvalidationDetector::new(DEFAULT_MAX_PATHS);
        let p1 = PathBuf::from("/workspace/1.txt");
        let p2 = PathBuf::from("/workspace/2.txt");
        let _ = d.observe("s1", "file_read", p1.clone(), b"1").await;
        let _ = d.observe("s2", "file_read", p2.clone(), b"2").await;

        d.evict_session("s1");

        assert!(!d.has_path("s1", &p1).await);
        assert!(d.has_path("s2", &p2).await);
    }
}
