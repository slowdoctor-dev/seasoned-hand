use std::future::Future;
use std::time::Duration;

use serde_json::json;

use crate::events::{EventStore, EventType, NewEvent, sqlite::SqliteEventStore};
use crate::sandbox::SandboxClient;
#[cfg(test)]
use crate::sandbox::SandboxError;

pub const RECITE_EVERY_N: u32 = 10;
pub const RECITE_TAIL_LINES: usize = 80;
pub const RECITE_READ_TIMEOUT: Duration = Duration::from_secs(1);
pub const PROGRESS_PATH: &str = "/workspace/progress.txt";

pub struct ReciteScheduler;

impl ReciteScheduler {
    pub fn should_fire(step: u32) -> bool {
        step > 0 && step.is_multiple_of(RECITE_EVERY_N)
    }
}

pub async fn recite_tick(sandbox: &SandboxClient, events: &SqliteEventStore, session_id: &str) {
    recite_tick_with_read(events, session_id, || {
        sandbox.read_workspace_file(session_id, PROGRESS_PATH)
    })
    .await;
}

async fn emit_skip(events: &SqliteEventStore, session_id: &str, reason: &str) {
    let _ = events
        .append(NewEvent {
            session_id: session_id.to_string(),
            event_type: EventType::Misc,
            source: "agent".into(),
            data: json!({"kind":"progress_recite_skipped","reason":reason}),
        })
        .await;
}

fn tail_last_n_lines(input: &str, n: usize) -> String {
    let lines = input.lines().collect::<Vec<_>>();
    let start = lines.len().saturating_sub(n);
    lines[start..].join("\n")
}

pub(crate) async fn recite_tick_with_read<F, Fut, E>(
    events: &SqliteEventStore,
    session_id: &str,
    read: F,
) where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<Vec<u8>, E>>,
    E: std::fmt::Display,
{
    let bytes = match tokio::time::timeout(RECITE_READ_TIMEOUT, read()).await {
        Err(_) => {
            emit_skip(events, session_id, "slow_read").await;
            return;
        }
        Ok(Err(err)) => {
            let reason = err.to_string();
            if reason.contains("No such file") || reason.contains("not found") {
                emit_skip(events, session_id, "missing_or_empty").await;
            } else {
                emit_skip(events, session_id, &reason).await;
            }
            return;
        }
        Ok(Ok(bytes)) => bytes,
    };

    if bytes.is_empty() {
        emit_skip(events, session_id, "missing_or_empty").await;
        return;
    }

    let content = String::from_utf8_lossy(&bytes);
    let tail = tail_last_n_lines(&content, RECITE_TAIL_LINES);
    if tail.trim().is_empty() {
        emit_skip(events, session_id, "missing_or_empty").await;
        return;
    }

    let _ = events
        .append(NewEvent {
            session_id: session_id.to_string(),
            event_type: EventType::Misc,
            source: "agent".into(),
            data: json!({
                "kind":"progress_recite",
                "progress_path": PROGRESS_PATH,
                "content_preview": tail,
            }),
        })
        .await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use crate::events::{EventQuery, EventStore};

    async fn event_store() -> SqliteEventStore {
        let pool = db::open(":memory:").await.unwrap();
        pool.with_conn(|conn| {
            conn.execute(
                "INSERT INTO sessions (id, created_at, updated_at, state) \
                 VALUES ('s1', 1, 1, 'RUNNING')",
                [],
            )
            .unwrap();
        })
        .await;
        SqliteEventStore::new(pool)
    }

    #[test]
    fn recite_fires_on_tenth_iteration() {
        assert!(ReciteScheduler::should_fire(10));
        assert!(ReciteScheduler::should_fire(20));
        assert!(!ReciteScheduler::should_fire(9));
        assert!(!ReciteScheduler::should_fire(11));
    }

    #[test]
    fn recite_does_not_fire_on_step_zero() {
        assert!(!ReciteScheduler::should_fire(0));
    }

    #[test]
    fn recite_truncates_to_80_lines() {
        let input = (1..=200)
            .map(|i| format!("line-{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let tail = tail_last_n_lines(&input, RECITE_TAIL_LINES);
        let lines = tail.lines().collect::<Vec<_>>();
        assert_eq!(lines.len(), 80);
        assert_eq!(lines.first().copied(), Some("line-121"));
        assert_eq!(lines.last().copied(), Some("line-200"));
    }

    #[tokio::test]
    async fn recite_skip_on_missing_file_does_not_break_loop() {
        let store = event_store().await;
        recite_tick_with_read(&store, "s1", || async {
            Err::<Vec<u8>, SandboxError>(SandboxError::NotFound("s1".into()))
        })
        .await;
        let events = store.query("s1", EventQuery::default()).await.unwrap();
        assert!(events.iter().any(|event| {
            event.data.get("kind").and_then(serde_json::Value::as_str)
                == Some("progress_recite_skipped")
        }));
    }

    #[tokio::test]
    async fn recite_skip_on_slow_read() {
        let store = event_store().await;
        recite_tick_with_read(&store, "s1", || async {
            tokio::time::sleep(Duration::from_millis(1500)).await;
            Ok::<Vec<u8>, SandboxError>(b"hello".to_vec())
        })
        .await;
        let events = store.query("s1", EventQuery::default()).await.unwrap();
        assert!(events.iter().any(|event| {
            event.data.get("kind").and_then(serde_json::Value::as_str)
                == Some("progress_recite_skipped")
                && event.data.get("reason").and_then(serde_json::Value::as_str) == Some("slow_read")
        }));
    }
}
