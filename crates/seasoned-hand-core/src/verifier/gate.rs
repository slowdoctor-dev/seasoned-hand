use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;

use crate::agent::AgentRunner;
use crate::db::DbPool;
use crate::events::{EventStore, EventType, NewEvent, sqlite::SqliteEventStore};

#[derive(Clone)]
pub struct VerifierGate {
    db: DbPool,
    events: Arc<SqliteEventStore>,
    runner: Arc<AgentRunner>,
}

impl VerifierGate {
    pub fn new(db: DbPool, events: Arc<SqliteEventStore>, runner: Arc<AgentRunner>) -> Self {
        Self { db, events, runner }
    }

    pub async fn run(&self, shutdown: tokio_util::sync::CancellationToken) {
        let mut cursor = 0_i64;
        while !shutdown.is_cancelled() {
            if let Ok(next) = self.poll_once(cursor).await {
                cursor = next;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }

    pub async fn poll_once(&self, after_id: i64) -> Result<i64, rusqlite::Error> {
        let rows = self
            .db
            .with_conn(move |conn| -> Result<Vec<(i64, String, String, Value)>, rusqlite::Error> {
                let mut stmt = conn.prepare(
                    "SELECT id, session_id, json_extract(data,'$.verdict') as verdict, data \
                     FROM events \
                     WHERE id > ? AND type = 'Misc' AND json_extract(data,'$.kind') = 'verifier_verdict' \
                     ORDER BY id ASC",
                )?;
                let rows = stmt.query_map(rusqlite::params![after_id], |row| {
                    let id: i64 = row.get(0)?;
                    let session_id: String = row.get(1)?;
                    let verdict: String = row.get(2)?;
                    let data_text: String = row.get(3)?;
                    let data: Value = serde_json::from_str(&data_text).unwrap_or(Value::Null);
                    Ok((id, session_id, verdict, data))
                })?;
                rows.collect()
            })
            .await?;
        let mut cursor = after_id;
        for (id, session_id, verdict, data) in rows {
            cursor = id;
            self.apply_task_complete_verdict(&session_id, &verdict, &data)
                .await;
        }
        Ok(cursor)
    }

    async fn apply_task_complete_verdict(&self, session_id: &str, verdict: &str, data: &Value) {
        let trigger = data
            .get("trigger_kind")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if trigger != "TaskComplete" {
            return;
        }

        match verdict {
            "pass" => {
                let _ = self.set_state(session_id, "FINISHED").await;
                let _ = self
                    .events
                    .append(NewEvent {
                        session_id: session_id.to_string(),
                        event_type: EventType::Misc,
                        source: "verifier_gate".into(),
                        data: serde_json::json!({"kind":"task_complete"}),
                    })
                    .await;
            }
            "fail" => {
                if data.get("suggested_plan_update").is_some()
                    && !data
                        .get("suggested_plan_update")
                        .is_some_and(Value::is_null)
                {
                    let _ = self.set_state(session_id, "RUNNING").await;
                    let _ = self.runner.resume_session(session_id).await;
                } else {
                    let _ = self.set_state(session_id, "SUSPENDED").await;
                    let _ = self
                        .events
                        .append(NewEvent {
                            session_id: session_id.to_string(),
                            event_type: EventType::Misc,
                            source: "verifier_gate".into(),
                            data: serde_json::json!({
                                "kind":"task_suspended_by_verifier",
                                "reason": data.get("reason").cloned().unwrap_or(Value::Null)
                            }),
                        })
                        .await;
                }
            }
            _ => {}
        }
    }

    async fn set_state(&self, session_id: &str, state: &str) -> Result<(), rusqlite::Error> {
        let session_id = session_id.to_string();
        let state = state.to_string();
        let changed = self
            .db
            .with_conn(move |conn| {
                conn.execute(
                    "UPDATE sessions SET state = ?, updated_at = unixepoch('subsec') * 1000000 WHERE id = ?",
                    rusqlite::params![state, session_id],
                )
            })
            .await?;
        if changed == 0 {
            return Ok(());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use serde_json::json;

    use super::*;
    use crate::agent::{AgentRunner, AgentRunnerDeps};
    use crate::cost::CostClient;
    use crate::db;
    use crate::dispatch::ToolDispatcher;
    use crate::dispatch::mask::DefaultMaskPolicy;
    use crate::events::{EventQuery, EventStore};
    use crate::plan::PlanManager;
    use crate::pubsub::RedisPool;
    use crate::router::SlotRouter;
    use crate::sandbox::SandboxClient;
    use crate::search::{SearchClient, SearchProvider};
    use crate::tools::register_builtin_tools;

    async fn fixture() -> (VerifierGate, Arc<SqliteEventStore>, DbPool, String) {
        let db = db::open(":memory:").await.unwrap();
        let session_id = "s1".to_string();
        let sid = session_id.clone();
        db.with_conn(move |conn| {
            conn.execute(
                "INSERT INTO sessions (id, created_at, updated_at, state) VALUES (?, 0, 0, 'VERIFYING')",
                rusqlite::params![sid],
            )
            .unwrap();
        })
        .await;
        let redis = RedisPool::new("redis://127.0.0.1:6").unwrap();
        let events = Arc::new(SqliteEventStore::with_redis(db.clone(), redis.clone()));
        let plan_manager = Arc::new(PlanManager::new(db.clone(), events.clone()));
        let dispatcher = Arc::new(ToolDispatcher::new(register_builtin_tools()));
        let router = Arc::new(SlotRouter::default_for_bifrost());
        let sandbox = Arc::new(
            SandboxClient::new(
                "ghcr.io/agent-infra/sandbox:1.0.0.152",
                std::env::temp_dir(),
            )
            .unwrap(),
        );
        let search = Arc::new(SearchClient::new(SearchProvider::Brave { api_key: None }));
        let llm = crate::llm::LlmClient::new("http://127.0.0.1:1/v1", None);
        let runner = Arc::new(AgentRunner::new(AgentRunnerDeps {
            llm,
            dispatcher,
            events: events.clone(),
            router,
            sandbox,
            search,
            cost: Arc::new(CostClient::new("http://127.0.0.1:1/v1")),
            sessions: db.clone(),
            plan_manager,
            mask_policy: Arc::new(DefaultMaskPolicy),
            checkpoint_labels: Arc::new(crate::checkpoint::CheckpointLabelBuffer::new()),
            redis: Arc::new(redis.clone()),
        }));

        let gate = VerifierGate::new(db.clone(), events.clone(), runner);
        (gate, events, db, session_id)
    }

    async fn insert_verdict_event(
        events: &SqliteEventStore,
        session_id: &str,
        verdict: &str,
        suggested: Option<serde_json::Value>,
    ) {
        events
            .append(NewEvent {
                session_id: session_id.to_string(),
                event_type: EventType::Misc,
                source: "verifier".into(),
                data: json!({
                    "kind":"verifier_verdict",
                    "trigger_kind":"TaskComplete",
                    "verdict": verdict,
                    "reason": "r",
                    "suggested_plan_update": suggested,
                    "verification_id": "v1",
                }),
            })
            .await
            .unwrap();
    }

    async fn state(db: &DbPool, session_id: &str) -> String {
        let sid = session_id.to_string();
        db.with_conn(move |conn| {
            conn.query_row(
                "SELECT state FROM sessions WHERE id = ?",
                rusqlite::params![sid],
                |row| row.get::<_, String>(0),
            )
            .unwrap()
        })
        .await
    }

    #[tokio::test]
    async fn verdict_pass_transitions_to_finished() {
        let (gate, events, db, session_id) = fixture().await;
        insert_verdict_event(&events, &session_id, "pass", None).await;
        let _ = gate.poll_once(0).await.unwrap();
        assert_eq!(state(&db, &session_id).await, "FINISHED");
        let rows = events
            .query(&session_id, EventQuery::default())
            .await
            .unwrap();
        assert!(rows.iter().any(
            |e| e.data.get("kind").and_then(serde_json::Value::as_str) == Some("task_complete")
        ));
    }

    #[tokio::test]
    async fn verdict_fail_without_suggestion_suspends() {
        let (gate, events, db, session_id) = fixture().await;
        insert_verdict_event(&events, &session_id, "fail", None).await;
        let _ = gate.poll_once(0).await.unwrap();
        assert_eq!(state(&db, &session_id).await, "SUSPENDED");
    }

    #[tokio::test]
    async fn verdict_fail_with_suggestion_sets_running() {
        let (gate, events, db, session_id) = fixture().await;
        insert_verdict_event(
            &events,
            &session_id,
            "fail",
            Some(json!({"phases":[{"id":1,"title":"x"}]})),
        )
        .await;
        let _ = gate.poll_once(0).await.unwrap();
        assert_eq!(state(&db, &session_id).await, "RUNNING");
    }
}
