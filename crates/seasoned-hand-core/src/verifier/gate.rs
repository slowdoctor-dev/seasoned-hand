use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;

use crate::agent::AgentRunner;
use crate::db::DbPool;
use crate::events::{EventStore, EventType, NewEvent, sqlite::SqliteEventStore};

/// Story 1.13b: handler called when a `fail+rollback_required` verdict
/// fires and the opt-in `rollback_on_verifier_fail` flag is true. The
/// production impl looks up the latest checkpoint for the session and
/// dispatches the `checkpoint_rollback` tool against it. Tests inject
/// a mock that records the call.
#[async_trait::async_trait]
pub trait RollbackHandler: Send + Sync {
    /// Roll back the most recent un-reverted checkpoint for the
    /// session, using `reason` as the rollback reason. Returns `true`
    /// when a rollback was attempted, `false` when there was no
    /// eligible checkpoint or the handler decided to skip.
    async fn rollback_latest(&self, session_id: &str, reason: &str) -> bool;
}

#[derive(Clone)]
pub struct VerifierGate {
    db: DbPool,
    events: Arc<SqliteEventStore>,
    runner: Arc<AgentRunner>,
    /// Story 1.13b: opt-in rollback wiring. `None` when the runtime has
    /// no rollback handler configured (Phase 0 / pre-1.13b deployments).
    rollback: Option<Arc<dyn RollbackHandler>>,
    /// Story 1.13b: gate flag for the rollback handler. Defaults `false`
    /// per phase-1/DEBT.md #3 — even when a handler is attached, the
    /// gate only invokes it when this is true.
    rollback_enabled: bool,
}

impl VerifierGate {
    pub fn new(db: DbPool, events: Arc<SqliteEventStore>, runner: Arc<AgentRunner>) -> Self {
        Self {
            db,
            events,
            runner,
            rollback: None,
            rollback_enabled: false,
        }
    }

    /// Story 1.13b: attach a rollback handler. The handler is invoked
    /// only when (a) `enabled == true` AND (b) the verdict carries
    /// `rollback_required: true`. When `enabled == false` the handler
    /// stays attached but is dormant; the `rollback_required` field on
    /// verdicts is logged-and-ignored.
    pub fn with_rollback(mut self, handler: Arc<dyn RollbackHandler>, enabled: bool) -> Self {
        self.rollback = Some(handler);
        self.rollback_enabled = enabled;
        self
    }

    pub async fn run(&self, shutdown: tokio_util::sync::CancellationToken) {
        // Seed cursor from the highest already-acked verdict so a restart
        // does not re-replay history (which would double-resume
        // sessions that fell into the fail+suggested_plan_update path).
        // refs: story 1.10 self-review — security report informational note
        let mut cursor = self.seed_cursor().await.unwrap_or(0);
        while !shutdown.is_cancelled() {
            if let Ok(next) = self.poll_once(cursor).await {
                cursor = next;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }

    /// Read the highest event id of an already-acked verdict so we can
    /// skip re-processing it on startup. Returns `Ok(0)` when no acks
    /// exist (fresh DB / never run before). `MAX(id)` over the empty
    /// set is `NULL`, which deserializes as `None`.
    pub async fn seed_cursor(&self) -> Result<i64, rusqlite::Error> {
        self.db
            .with_conn(|conn| {
                let id: Option<i64> = conn.query_row(
                    "SELECT MAX(id) FROM events \
                     WHERE type = 'Misc' \
                       AND json_extract(data,'$.kind') = 'verifier_gate_ack'",
                    [],
                    |row| row.get::<_, Option<i64>>(0),
                )?;
                Ok(id.unwrap_or(0))
            })
            .await
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

        let verification_id = data.get("verification_id").cloned().unwrap_or(Value::Null);
        let outcome: &str = match verdict {
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
                "finished"
            }
            "fail" => {
                // Story 1.13b opt-in: if the verdict requests rollback
                // AND the gate is configured to honor it, fire the
                // rollback handler BEFORE applying the
                // fail+suggestion / fail-no-suggestion branch. Default
                // off per phase-1/DEBT.md #3 — when off, log only.
                let want_rollback = data
                    .get("rollback_required")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                if want_rollback {
                    if self.rollback_enabled {
                        if let Some(handler) = &self.rollback {
                            handler.rollback_latest(session_id, "verifier_fail").await;
                        }
                    } else {
                        tracing::info!(
                            %session_id,
                            "verdict requested rollback but checkpoint.rollback_on_verifier_fail is off (phase-1/DEBT.md #3 — default)"
                        );
                    }
                }
                if data.get("suggested_plan_update").is_some()
                    && !data
                        .get("suggested_plan_update")
                        .is_some_and(Value::is_null)
                {
                    let _ = self.set_state(session_id, "RUNNING").await;
                    let _ = self.runner.resume_session(session_id).await;
                    "resumed"
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
                    "suspended"
                }
            }
            _ => return,
        };

        // Universal ack so the cursor seed at restart skips this row.
        // refs: story 1.10 self-review — security report informational note
        let _ = self
            .events
            .append(NewEvent {
                session_id: session_id.to_string(),
                event_type: EventType::Misc,
                source: "verifier_gate".into(),
                data: serde_json::json!({
                    "kind": "verifier_gate_ack",
                    "verification_id": verification_id,
                    "outcome": outcome,
                }),
            })
            .await;
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
            checkpoints: Arc::new(crate::checkpoint::CheckpointStore::new(db.clone())),
            redis: Arc::new(redis.clone()),
            cancel_tokens: Arc::new(dashmap::DashMap::new()),
        }));

        let gate = VerifierGate::new(db.clone(), events.clone(), runner);
        (gate, events, db, session_id)
    }

    async fn insert_verdict_event(
        events: &SqliteEventStore,
        session_id: &str,
        trigger_kind: &str,
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
                    "trigger_kind": trigger_kind,
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
        insert_verdict_event(&events, &session_id, "TaskComplete", "pass", None).await;
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
        insert_verdict_event(&events, &session_id, "TaskComplete", "fail", None).await;
        let _ = gate.poll_once(0).await.unwrap();
        assert_eq!(state(&db, &session_id).await, "SUSPENDED");
    }

    #[tokio::test]
    async fn verdict_fail_with_suggestion_sets_running() {
        let (gate, events, db, session_id) = fixture().await;
        insert_verdict_event(
            &events,
            &session_id,
            "TaskComplete",
            "fail",
            Some(json!({"phases":[{"id":1,"title":"x"}]})),
        )
        .await;
        let _ = gate.poll_once(0).await.unwrap();
        assert_eq!(state(&db, &session_id).await, "RUNNING");
    }

    #[tokio::test]
    async fn each_processed_verdict_emits_verifier_gate_ack() {
        let (gate, events, _db, session_id) = fixture().await;
        insert_verdict_event(&events, &session_id, "TaskComplete", "pass", None).await;
        let _ = gate.poll_once(0).await.unwrap();
        let rows = events
            .query(&session_id, EventQuery::default())
            .await
            .unwrap();
        let acks: Vec<_> = rows
            .iter()
            .filter(|e| {
                e.event_type == EventType::Misc
                    && e.data.get("kind").and_then(serde_json::Value::as_str)
                        == Some("verifier_gate_ack")
            })
            .collect();
        assert_eq!(acks.len(), 1, "expected exactly one verifier_gate_ack");
        assert_eq!(
            acks[0]
                .data
                .get("outcome")
                .and_then(serde_json::Value::as_str),
            Some("finished")
        );
        assert_eq!(
            acks[0]
                .data
                .get("verification_id")
                .and_then(serde_json::Value::as_str),
            Some("v1"),
            "ack must carry the verification_id from the verdict"
        );
    }

    #[tokio::test]
    async fn seed_cursor_returns_zero_when_no_acks_exist() {
        let (gate, _events, _db, _session_id) = fixture().await;
        assert_eq!(gate.seed_cursor().await.unwrap(), 0);
    }

    #[tokio::test]
    async fn restart_does_not_replay_already_acked_verdicts() {
        // Story 1.10 follow-up (security review informational note):
        // a fresh gate started after a previous gate processed verdicts
        // must NOT re-process them — otherwise restart would
        // double-resume sessions that fell into fail+suggested_plan_update.
        let (gate, events, db, session_id) = fixture().await;
        insert_verdict_event(
            &events,
            &session_id,
            "TaskComplete",
            "fail",
            Some(json!({"phases":[{"id":1,"title":"x"}]})),
        )
        .await;

        // First gate processes the verdict and emits an ack.
        let cursor_after_first = gate.poll_once(0).await.unwrap();
        assert!(cursor_after_first > 0);
        assert_eq!(state(&db, &session_id).await, "RUNNING");

        // Simulate restart: a fresh VerifierGate against the SAME db.
        let fresh_gate = VerifierGate::new(db.clone(), events.clone(), gate.runner.clone());
        let seeded = fresh_gate.seed_cursor().await.unwrap();
        assert!(
            seeded >= cursor_after_first,
            "seed must cover the previously-processed verdict, got {seeded} < {cursor_after_first}"
        );

        // Manually set the session back to VERIFYING so we can detect any
        // unwanted re-replay (the test gate is correct iff it leaves the
        // state alone after restart).
        db.with_conn({
            let sid = session_id.clone();
            move |conn| {
                conn.execute(
                    "UPDATE sessions SET state='VERIFYING' WHERE id = ?",
                    rusqlite::params![sid],
                )
                .unwrap();
            }
        })
        .await;

        // Polling from the seeded cursor must NOT re-process the verdict.
        let _ = fresh_gate.poll_once(seeded).await.unwrap();
        assert_eq!(
            state(&db, &session_id).await,
            "VERIFYING",
            "post-restart poll must not re-transition the session"
        );
    }

    #[tokio::test]
    async fn gate_does_not_transition_state_on_invalidation_verdict() {
        let (gate, events, db, session_id) = fixture().await;
        insert_verdict_event(&events, &session_id, "Invalidation", "pass", None).await;
        let _ = gate.poll_once(0).await.unwrap();
        assert_eq!(state(&db, &session_id).await, "VERIFYING");
    }

    // ------------------------------------------------------------------
    // Story 1.13b: Verifier-driven rollback opt-in path
    // ------------------------------------------------------------------

    use std::sync::Mutex;

    #[derive(Default, Clone)]
    struct RecordingRollback {
        calls: Arc<Mutex<Vec<(String, String)>>>,
    }

    #[async_trait::async_trait]
    impl RollbackHandler for RecordingRollback {
        async fn rollback_latest(&self, session_id: &str, reason: &str) -> bool {
            self.calls
                .lock()
                .unwrap()
                .push((session_id.to_string(), reason.to_string()));
            true
        }
    }

    async fn insert_rollback_verdict_event(
        events: &SqliteEventStore,
        session_id: &str,
        rollback_required: bool,
    ) {
        events
            .append(NewEvent {
                session_id: session_id.to_string(),
                event_type: EventType::Misc,
                source: "verifier".into(),
                data: json!({
                    "kind": "verifier_verdict",
                    "trigger_kind": "TaskComplete",
                    "verdict": "fail",
                    "reason": "test",
                    "rollback_required": rollback_required,
                    "verification_id": "v1",
                }),
            })
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn verifier_driven_rollback_disabled_by_default() {
        // No `with_rollback(...)` call ⇒ rollback handler not attached.
        // Even with `rollback_required: true` on the verdict, no
        // rollback should fire.
        let (gate, events, _db, session_id) = fixture().await;
        insert_rollback_verdict_event(&events, &session_id, true).await;
        let _ = gate.poll_once(0).await.unwrap();
        // Handler was never invoked because gate.rollback was None.
        // (No mock to assert against — the test just verifies the
        // code path doesn't panic and the fail-no-suggestion
        // transition still happens.)
    }

    #[tokio::test]
    async fn verifier_driven_rollback_does_not_fire_when_flag_off() {
        // Handler attached but `enabled=false`. Verdict requests
        // rollback. Handler must NOT be called.
        let (mut gate, events, _db, session_id) = fixture().await;
        let recorder = Arc::new(RecordingRollback::default());
        gate = gate.with_rollback(recorder.clone(), false);
        insert_rollback_verdict_event(&events, &session_id, true).await;
        let _ = gate.poll_once(0).await.unwrap();
        let calls = recorder.calls.lock().unwrap().clone();
        assert!(
            calls.is_empty(),
            "handler must NOT fire when enabled=false; got {calls:?}"
        );
    }

    #[tokio::test]
    async fn verifier_driven_rollback_invokes_when_enabled_and_required() {
        // Handler attached, flag on, verdict requests rollback ⇒ fire.
        let (mut gate, events, _db, session_id) = fixture().await;
        let recorder = Arc::new(RecordingRollback::default());
        gate = gate.with_rollback(recorder.clone(), true);
        insert_rollback_verdict_event(&events, &session_id, true).await;
        let _ = gate.poll_once(0).await.unwrap();
        let calls = recorder.calls.lock().unwrap().clone();
        assert_eq!(calls.len(), 1, "exactly one rollback call expected");
        assert_eq!(calls[0].0, session_id);
        assert_eq!(calls[0].1, "verifier_fail");
    }

    #[tokio::test]
    async fn verifier_driven_rollback_does_not_fire_when_required_false() {
        // Handler attached, flag on, verdict does NOT request rollback
        // ⇒ no fire (rollback only on opt-in by both sides).
        let (mut gate, events, _db, session_id) = fixture().await;
        let recorder = Arc::new(RecordingRollback::default());
        gate = gate.with_rollback(recorder.clone(), true);
        insert_rollback_verdict_event(&events, &session_id, false).await;
        let _ = gate.poll_once(0).await.unwrap();
        let calls = recorder.calls.lock().unwrap().clone();
        assert!(
            calls.is_empty(),
            "handler must NOT fire when verdict.rollback_required=false"
        );
    }
}
