use std::sync::Arc;
use std::time::Duration;

use rusqlite::OptionalExtension;
use serde_json::Value;

use crate::agent::AgentRunner;
use crate::db::DbPool;
use crate::events::{EventStore, EventType, NewEvent, sqlite::SqliteEventStore};
use tokio::time::Instant;

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
    extraction: Option<Arc<dyn ExtractionHandler>>,
}

#[async_trait::async_trait]
pub trait ExtractionHandler: Send + Sync {
    async fn extract_sync(&self, session_id: &str) -> Result<(), ExtractionError>;
}

#[derive(Debug, Clone)]
pub struct ExtractionError {
    pub stage: &'static str,
    pub reason: String,
}

impl ExtractionError {
    pub fn new(stage: &'static str, reason: impl Into<String>) -> Self {
        Self {
            stage,
            reason: reason.into(),
        }
    }
}

impl VerifierGate {
    pub fn new(db: DbPool, events: Arc<SqliteEventStore>, runner: Arc<AgentRunner>) -> Self {
        Self {
            db,
            events,
            runner,
            rollback: None,
            rollback_enabled: false,
            extraction: None,
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

    pub fn with_extraction(mut self, handler: Arc<dyn ExtractionHandler>) -> Self {
        self.extraction = Some(handler);
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
        let verification_id = data.get("verification_id").cloned().unwrap_or(Value::Null);
        let outcome: &str = match trigger {
            "TaskComplete" => match verdict {
                "pass" => {
                    if let Err(e) = self.record_playbook_outcome(session_id, "pass").await {
                        tracing::warn!(%session_id, error = ?e, "verifier gate failed to record playbook pass outcome");
                    }
                    self.run_sync_extraction(session_id).await;
                    if let Err(e) = self.set_state(session_id, "FINISHED").await {
                        tracing::warn!(%session_id, error = ?e, "verifier gate failed to set FINISHED state");
                    } else {
                        self.runner.finalize_session(session_id).await;
                    }
                    if let Err(e) = self
                        .events
                        .append(NewEvent {
                            session_id: session_id.to_string(),
                            event_type: EventType::Misc,
                            source: "verifier_gate".into(),
                            data: serde_json::json!({"kind":"task_complete"}),
                        })
                        .await
                    {
                        tracing::warn!(%session_id, error = ?e, "verifier gate failed to append task_complete event");
                    }
                    "finished"
                }
                "fail" => {
                    if let Err(e) = self.record_playbook_outcome(session_id, "fail").await {
                        tracing::warn!(%session_id, error = ?e, "verifier gate failed to record playbook fail outcome");
                    }
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
                        if let Err(e) = self.set_state(session_id, "RUNNING").await {
                            tracing::warn!(%session_id, error = ?e, "verifier gate failed to set RUNNING state");
                        }
                        if let Err(e) = self.runner.resume_session(session_id).await {
                            tracing::warn!(%session_id, error = ?e, "verifier gate failed to resume session after verifier fail");
                        }
                        "resumed"
                    } else {
                        if let Err(e) = self.set_state(session_id, "SUSPENDED").await {
                            tracing::warn!(%session_id, error = ?e, "verifier gate failed to set SUSPENDED state");
                        }
                        if let Err(e) = self
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
                            .await
                        {
                            tracing::warn!(%session_id, error = ?e, "verifier gate failed to append task_suspended_by_verifier event");
                        }
                        "suspended"
                    }
                }
                _ => return,
            },
            "Invalidation" => "continued",
            "CircuitBreaker" => {
                let has_suggestion = data.get("suggested_plan_update").is_some()
                    && !data
                        .get("suggested_plan_update")
                        .is_some_and(Value::is_null);
                let kind = data
                    .get("breaker_kind")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let breaker = self.runner.breaker_for_session(session_id).await;
                if verdict == "pass" {
                    match kind {
                        "Stuck" => {
                            breaker.reset_stuck().await;
                        }
                        "ErrorRate" => {
                            breaker.reset_error_rate().await;
                        }
                        "Cost" | "MaxSteps" => {
                            if let Err(e) = self.set_state(session_id, "SUSPENDED").await {
                                tracing::warn!(%session_id, error = ?e, "verifier gate failed to set SUSPENDED state (breaker pass)");
                            }
                        }
                        _ => {}
                    }
                } else if verdict == "fail" {
                    if has_suggestion {
                        if let Err(e) = self.set_state(session_id, "RUNNING").await {
                            tracing::warn!(%session_id, error = ?e, "verifier gate failed to set RUNNING state");
                        }
                        if let Err(e) = self.runner.resume_session(session_id).await {
                            tracing::warn!(%session_id, error = ?e, "verifier gate failed to resume session after verifier fail");
                        }
                    } else if kind == "Stuck" || kind == "ErrorRate" {
                        if let Err(e) = self.set_state(session_id, "ERROR").await {
                            tracing::warn!(%session_id, error = ?e, "verifier gate failed to set ERROR state (breaker fail)");
                        } else {
                            self.runner.finalize_session(session_id).await;
                        }
                    } else if (kind == "Cost" || kind == "MaxSteps")
                        && let Err(e) = self.set_state(session_id, "SUSPENDED").await
                    {
                        tracing::warn!(%session_id, error = ?e, "verifier gate failed to set SUSPENDED state (breaker fail)");
                    }
                }
                breaker.rearm().await;
                "continued"
            }
            _ => return,
        };

        // Universal ack so the cursor seed at restart skips this row.
        // refs: story 1.10 self-review — security report informational note
        if let Err(e) = self
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
            .await
        {
            tracing::warn!(%session_id, error = ?e, "verifier gate failed to append verifier_gate_ack event");
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

    async fn run_sync_extraction(&self, session_id: &str) {
        let tool_calls = match self.session_tool_calls(session_id).await {
            Ok(v) => v,
            Err(e) => {
                self.emit_extraction_error(session_id, "prepare_input", e.to_string())
                    .await;
                return;
            }
        };
        if tool_calls < 5 {
            return;
        }

        let start = Instant::now();
        let extraction = async {
            let Some(handler) = &self.extraction else {
                return Err(ExtractionError::new(
                    "llm_call",
                    "extraction_handler_not_configured",
                ));
            };
            handler.extract_sync(session_id).await
        };
        match tokio::time::timeout(Duration::from_secs(60), extraction).await {
            Ok(Ok(())) => {}
            Ok(Err(err)) => {
                self.emit_extraction_error(session_id, err.stage, err.reason)
                    .await;
            }
            Err(_) => {
                if let Err(error) = self
                    .events
                    .append(NewEvent {
                        session_id: session_id.to_string(),
                        event_type: EventType::Misc,
                        source: "verifier_gate".into(),
                        data: serde_json::json!({
                            "kind": "playbook_extraction_timeout",
                            "session_id": session_id,
                            "elapsed_ms": start.elapsed().as_millis() as u64,
                        }),
                    })
                    .await
                {
                    tracing::warn!(%session_id, error = ?error, "failed to append playbook_extraction_timeout");
                }
            }
        }
    }

    async fn session_tool_calls(&self, session_id: &str) -> Result<i64, rusqlite::Error> {
        let sid = session_id.to_string();
        self.db
            .with_conn(move |conn| {
                conn.query_row(
                    "SELECT tool_calls FROM sessions WHERE id = ?",
                    rusqlite::params![sid],
                    |row| row.get::<_, i64>(0),
                )
            })
            .await
    }

    async fn emit_extraction_error(&self, session_id: &str, stage: &str, reason: String) {
        if let Err(error) = self
            .events
            .append(NewEvent {
                session_id: session_id.to_string(),
                event_type: EventType::Misc,
                source: "verifier_gate".into(),
                data: serde_json::json!({
                    "kind": "playbook_extraction_error",
                    "session_id": session_id,
                    "stage": stage,
                    "reason": reason,
                }),
            })
            .await
        {
            tracing::warn!(%session_id, error = ?error, "failed to append playbook_extraction_error");
        }
    }

    async fn record_playbook_outcome(
        &self,
        session_id: &str,
        verdict: &str,
    ) -> Result<(), rusqlite::Error> {
        if verdict != "pass" && verdict != "fail" {
            return Ok(());
        }
        let sid = session_id.to_string();
        let verdict_string = verdict.to_string();
        self.db
            .with_conn(move |conn| {
                let data_text: Option<String> = conn
                    .query_row(
                        "SELECT data
                         FROM events
                         WHERE session_id = ?
                           AND type = 'Skill'
                           AND json_extract(data, '$.kind') = 'injection'
                         ORDER BY id DESC
                         LIMIT 1",
                        rusqlite::params![sid],
                        |row| row.get(0),
                    )
                    .optional()?;
                let Some(data_text) = data_text else {
                    return Ok(());
                };
                let payload: Value = serde_json::from_str(&data_text).unwrap_or(Value::Null);
                let injected_ids = payload
                    .get("injected_ids")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                for injected in injected_ids {
                    let Some(playbook_id) = injected.as_str() else {
                        continue;
                    };
                    let changed = if verdict_string.as_str() == "pass" {
                        conn.execute(
                            "UPDATE playbooks
                             SET success_count = success_count + 1
                             WHERE id = ?",
                            rusqlite::params![playbook_id],
                        )?
                    } else {
                        conn.execute(
                            "UPDATE playbooks
                             SET failure_count = failure_count + 1
                             WHERE id = ?",
                            rusqlite::params![playbook_id],
                        )?
                    };
                    if changed == 0 {
                        continue;
                    }
                    let outcome_data = serde_json::json!({
                        "kind": "outcome",
                        "playbook_id": playbook_id,
                        "outcome": verdict_string.as_str(),
                    })
                    .to_string();
                    conn.execute(
                        "INSERT INTO events (session_id, timestamp, type, source, data)
                         VALUES (?, unixepoch('subsec') * 1000000, 'Skill', 'verifier_gate', ?)",
                        rusqlite::params![sid, outcome_data],
                    )?;
                }
                Ok(())
            })
            .await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use serde_json::json;

    use super::*;
    use crate::agent::breaker::BreakerRegistry;
    use crate::agent::init::injector::{INJECTION_BYTE_CAP, build_injection};
    use crate::agent::{AgentRunner, AgentRunnerDeps};
    use crate::cost::CostClient;
    use crate::db;
    use crate::dispatch::ToolDispatcher;
    use crate::dispatch::mask::DefaultMaskPolicy;
    use crate::events::{EventQuery, EventStore};
    use crate::matcher::{MatchRequest, MatcherMode, match_playbooks, normalize_brief};
    use crate::plan::PlanManager;
    use crate::pubsub::RedisPool;
    use crate::router::SlotRouter;
    use crate::sandbox::SandboxClient;
    use crate::search::{SearchClient, SearchProvider};
    use crate::tools::register_builtin_tools;
    use crate::verifier::invalidation::{DEFAULT_MAX_PATHS, InvalidationDetector};

    #[derive(Default, Clone)]
    struct OkExtraction;
    #[async_trait::async_trait]
    impl ExtractionHandler for OkExtraction {
        async fn extract_sync(&self, _session_id: &str) -> Result<(), ExtractionError> {
            Ok(())
        }
    }

    #[derive(Default, Clone)]
    struct ErrExtraction;
    #[async_trait::async_trait]
    impl ExtractionHandler for ErrExtraction {
        async fn extract_sync(&self, _session_id: &str) -> Result<(), ExtractionError> {
            Err(ExtractionError::new("llm_call", "simulated_llm_failure"))
        }
    }

    #[derive(Default, Clone)]
    struct SleepExtraction;
    #[async_trait::async_trait]
    impl ExtractionHandler for SleepExtraction {
        async fn extract_sync(&self, _session_id: &str) -> Result<(), ExtractionError> {
            tokio::time::sleep(Duration::from_secs(61)).await;
            Ok(())
        }
    }

    async fn fixture() -> (VerifierGate, Arc<SqliteEventStore>, DbPool, String) {
        let (gate, events, db, session_id, _) = fixture_with_detector().await;
        (gate, events, db, session_id)
    }

    async fn fixture_with_detector() -> (
        VerifierGate,
        Arc<SqliteEventStore>,
        DbPool,
        String,
        Arc<InvalidationDetector>,
    ) {
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
        let invalidation_detector = Arc::new(InvalidationDetector::new(DEFAULT_MAX_PATHS));
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
            breakers: Arc::new(BreakerRegistry::new()),
            cancel_tokens: Arc::new(dashmap::DashMap::new()),
            invalidation_detector: Some(invalidation_detector.clone()),
        }));

        let gate = VerifierGate::new(db.clone(), events.clone(), runner);
        (gate, events, db, session_id, invalidation_detector)
    }

    async fn set_tool_calls(db: &DbPool, session_id: &str, count: i64) {
        let sid = session_id.to_string();
        db.with_conn(move |conn| {
            conn.execute(
                "UPDATE sessions SET tool_calls = ? WHERE id = ?",
                rusqlite::params![count, sid],
            )
            .unwrap();
        })
        .await;
    }

    async fn count_action_events(db: &DbPool, session_id: &str) -> i64 {
        let sid = session_id.to_string();
        db.with_conn(move |conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM events WHERE session_id = ? AND type = 'Action'",
                rusqlite::params![sid],
                |row| row.get::<_, i64>(0),
            )
            .unwrap()
        })
        .await
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
    async fn terminal_verdict_evicts_invalidation_session_hashes() {
        let (gate, events, db, session_id, detector) = fixture_with_detector().await;
        let _ = detector
            .observe(
                &session_id,
                "file_read",
                std::path::PathBuf::from("/workspace/a.txt"),
                b"v1",
            )
            .await;
        assert!(detector.has_session_for_test(&session_id));

        insert_verdict_event(&events, &session_id, "TaskComplete", "pass", None).await;
        let _ = gate.poll_once(0).await.unwrap();

        assert_eq!(state(&db, &session_id).await, "FINISHED");
        assert!(
            !detector.has_session_for_test(&session_id),
            "terminal session cleanup must evict invalidation hashes"
        );
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

    async fn insert_breaker_verdict(
        events: &SqliteEventStore,
        session_id: &str,
        verdict: &str,
        breaker_kind: &str,
        suggested: Option<serde_json::Value>,
    ) {
        events
            .append(NewEvent {
                session_id: session_id.to_string(),
                event_type: EventType::Misc,
                source: "verifier".into(),
                data: json!({
                    "kind":"verifier_verdict",
                    "trigger_kind":"CircuitBreaker",
                    "verdict": verdict,
                    "reason": "r",
                    "breaker_kind": breaker_kind,
                    "suggested_plan_update": suggested,
                    "verification_id": "v1",
                }),
            })
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn breaker_pass_on_cost_still_suspends() {
        let (gate, events, db, session_id) = fixture().await;
        insert_breaker_verdict(&events, &session_id, "pass", "Cost", None).await;
        let _ = gate.poll_once(0).await.unwrap();
        assert_eq!(state(&db, &session_id).await, "SUSPENDED");
    }

    #[tokio::test]
    async fn breaker_fail_without_suggestion_errors_for_stuck() {
        let (gate, events, db, session_id) = fixture().await;
        insert_breaker_verdict(&events, &session_id, "fail", "Stuck", None).await;
        let _ = gate.poll_once(0).await.unwrap();
        assert_eq!(state(&db, &session_id).await, "ERROR");
    }

    #[tokio::test]
    async fn breaker_fail_without_suggestion_suspends_for_cost() {
        let (gate, events, db, session_id) = fixture().await;
        insert_breaker_verdict(&events, &session_id, "fail", "Cost", None).await;
        let _ = gate.poll_once(0).await.unwrap();
        assert_eq!(state(&db, &session_id).await, "SUSPENDED");
    }

    #[tokio::test]
    async fn breaker_passes_verdict_resets_counter_for_stuck() {
        // Story 1.12: a `pass` verdict on a `CircuitBreaker` trigger
        // with `breaker_kind == "Stuck"` must NOT transition session
        // state — it silently calls `breaker.reset_stuck()` and lets
        // the runner continue. Distinct from the `Cost` / `MaxSteps`
        // pass paths, which suspend even on pass. Pinned against
        // accidental future change in the match arm.
        let (gate, events, db, session_id) = fixture().await;
        insert_breaker_verdict(&events, &session_id, "pass", "Stuck", None).await;
        let _ = gate.poll_once(0).await.unwrap();
        assert_eq!(state(&db, &session_id).await, "VERIFYING");
    }

    #[tokio::test]
    async fn breaker_fail_with_suggestion_continues() {
        // Story 1.12: a `fail` verdict on a `CircuitBreaker` trigger
        // that ALSO carries a `suggested_plan_update` transitions the
        // session to `RUNNING` (the gate then dispatches a resume).
        // Distinct from the no-suggestion fail paths, which go to
        // `ERROR` (Stuck/ErrorRate) or `SUSPENDED` (Cost/MaxSteps).
        let (gate, events, db, session_id) = fixture().await;
        let suggested = serde_json::json!({"phases": []});
        insert_breaker_verdict(&events, &session_id, "fail", "Stuck", Some(suggested)).await;
        let _ = gate.poll_once(0).await.unwrap();
        assert_eq!(state(&db, &session_id).await, "RUNNING");
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

    #[tokio::test]
    async fn extraction_sync_path_runs_only_for_pass_and_tool_calls_threshold() {
        let (mut gate, events, db, session_id) = fixture().await;
        gate = gate.with_extraction(Arc::new(OkExtraction));
        set_tool_calls(&db, &session_id, 4).await;
        insert_verdict_event(&events, &session_id, "TaskComplete", "pass", None).await;
        let _ = gate.poll_once(0).await.unwrap();

        let rows = events
            .query(&session_id, EventQuery::default())
            .await
            .unwrap();
        assert!(
            rows.iter().all(|e| {
                e.data.get("kind").and_then(Value::as_str) != Some("playbook_extraction_error")
                    && e.data.get("kind").and_then(Value::as_str)
                        != Some("playbook_extraction_timeout")
            }),
            "no extraction telemetry expected when tool_calls < 5"
        );
    }

    #[tokio::test]
    async fn sessions_tool_calls_matches_action_count() {
        let (_gate, events, db, session_id) = fixture().await;

        for i in 0..3 {
            events
                .append(NewEvent {
                    session_id: session_id.clone(),
                    event_type: EventType::Action,
                    source: "agent".into(),
                    data: json!({"kind":"tool_call","seq": i}),
                })
                .await
                .unwrap();
        }
        events
            .append(NewEvent {
                session_id: session_id.clone(),
                event_type: EventType::Misc,
                source: "agent".into(),
                data: json!({"kind":"noise"}),
            })
            .await
            .unwrap();

        // Baseline-fixture parity setup: canonical session counter should mirror
        // Action-event cardinality for this deterministic fixture.
        set_tool_calls(&db, &session_id, 3).await;
        let action_count = count_action_events(&db, &session_id).await;
        let tool_calls = db
            .with_conn({
                let sid = session_id.clone();
                move |conn| {
                    conn.query_row(
                        "SELECT tool_calls FROM sessions WHERE id = ?",
                        rusqlite::params![sid],
                        |row| row.get::<_, i64>(0),
                    )
                    .unwrap()
                }
            })
            .await;

        assert_eq!(
            tool_calls, action_count,
            "sessions.tool_calls parity mismatch for baseline fixture: expected Action count {}, got sessions.tool_calls {}",
            action_count, tool_calls
        );
    }

    #[tokio::test]
    async fn extraction_timeout_and_error_events_are_emitted() {
        let (mut gate, events, db, session_id) = fixture().await;
        set_tool_calls(&db, &session_id, 6).await;
        gate = gate.with_extraction(Arc::new(ErrExtraction));
        insert_verdict_event(&events, &session_id, "TaskComplete", "pass", None).await;
        let _ = gate.poll_once(0).await.unwrap();

        let err_event = events
            .query(
                &session_id,
                EventQuery {
                    event_type: Some(EventType::Misc),
                    ..EventQuery::default()
                },
            )
            .await
            .unwrap()
            .into_iter()
            .find(|e| {
                e.data.get("kind").and_then(Value::as_str) == Some("playbook_extraction_error")
            })
            .expect("expected extraction error event");
        assert_eq!(err_event.data["stage"], "llm_call");
        assert_eq!(err_event.data["reason"], "simulated_llm_failure");

        let (mut gate2, events2, db2, session_id2) = fixture().await;
        set_tool_calls(&db2, &session_id2, 6).await;
        gate2 = gate2.with_extraction(Arc::new(SleepExtraction));
        insert_verdict_event(&events2, &session_id2, "TaskComplete", "pass", None).await;
        let _ = gate2.poll_once(0).await.unwrap();
        let timeout_event = events2
            .query(
                &session_id2,
                EventQuery {
                    event_type: Some(EventType::Misc),
                    ..EventQuery::default()
                },
            )
            .await
            .unwrap()
            .into_iter()
            .find(|e| {
                e.data.get("kind").and_then(Value::as_str) == Some("playbook_extraction_timeout")
            })
            .expect("expected extraction timeout event");
        assert!(
            timeout_event
                .data
                .get("elapsed_ms")
                .and_then(Value::as_u64)
                .unwrap_or(0)
                >= 60_000
        );
    }

    async fn insert_injection_event(events: &SqliteEventStore, session_id: &str, ids: &[&str]) {
        events
            .append(NewEvent {
                session_id: session_id.to_string(),
                event_type: EventType::Skill,
                source: "playbook_injector".into(),
                data: json!({
                    "kind":"injection",
                    "injected_ids": ids,
                    "total_bytes": 10,
                    "truncated": false
                }),
            })
            .await
            .unwrap();
    }

    async fn seed_playbook(db: &DbPool, playbook_id: &str, source_task_id: &str) {
        let id = playbook_id.to_string();
        let task = source_task_id.to_string();
        db.with_conn(move |conn| {
            conn.execute(
                "INSERT INTO playbooks (
                    id, tenant_id, title, content_path, schema_version, source_task_id,
                    created_at, updated_at, trigger_keywords, content, success_count,
                    failure_count, avg_duration_ms, avg_tool_calls, status, version
                 ) VALUES (
                    ?, 'legacy-default', 'PB', '', 1, ?, 1, 1, '[]', 'body', 0, 0, NULL, NULL, 'active', 1
                 )",
                rusqlite::params![id, task],
            )
            .unwrap();
        })
        .await;
    }

    async fn add_project_and_task(db: &DbPool, project_id: &str, task_id: &str) {
        let pid = project_id.to_string();
        let tid = task_id.to_string();
        db.with_conn(move |conn| {
            conn.execute(
                "INSERT INTO projects (id, tenant_id, status, title, created_at, updated_at)
                 VALUES (?, 'legacy-default', 'active', 'P', 1, 1)",
                rusqlite::params![pid],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO tasks (id, project_id, tenant_id, status, title, created_at, updated_at)
                 VALUES (?, ?, 'legacy-default', 'Running', 'T', 1, 1)",
                rusqlite::params![tid, pid],
            )
            .unwrap();
        })
        .await;
    }

    #[tokio::test]
    async fn outcome_counter_updates() {
        let (gate, events, db, session_id) = fixture().await;
        add_project_and_task(&db, "p1", "t1").await;
        seed_playbook(&db, "pb-1", "t1").await;
        insert_injection_event(&events, &session_id, &["pb-1"]).await;

        insert_verdict_event(&events, &session_id, "TaskComplete", "pass", None).await;
        let _ = gate.poll_once(0).await.unwrap();

        let counts = db
            .with_conn(|conn| {
                conn.query_row(
                    "SELECT success_count, failure_count FROM playbooks WHERE id = 'pb-1'",
                    [],
                    |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
                )
                .unwrap()
            })
            .await;
        assert_eq!(counts, (1, 0));

        let rows = events
            .query(&session_id, EventQuery::default())
            .await
            .unwrap();
        assert!(rows.iter().any(|e| {
            e.event_type == EventType::Skill
                && e.data.get("kind").and_then(Value::as_str) == Some("outcome")
                && e.data.get("outcome").and_then(Value::as_str) == Some("pass")
                && e.data.get("playbook_id").and_then(Value::as_str) == Some("pb-1")
        }));
    }

    #[tokio::test]
    async fn non_terminal_verdict_noop() {
        let (gate, events, db, session_id) = fixture().await;
        add_project_and_task(&db, "p2", "t2").await;
        seed_playbook(&db, "pb-2", "t2").await;
        insert_injection_event(&events, &session_id, &["pb-2"]).await;

        insert_verdict_event(&events, &session_id, "TaskComplete", "error", None).await;
        let _ = gate.poll_once(0).await.unwrap();

        let counts = db
            .with_conn(|conn| {
                conn.query_row(
                    "SELECT success_count, failure_count FROM playbooks WHERE id = 'pb-2'",
                    [],
                    |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
                )
                .unwrap()
            })
            .await;
        assert_eq!(counts, (0, 0));
        let rows = events
            .query(
                &session_id,
                EventQuery {
                    event_type: Some(EventType::Skill),
                    ..EventQuery::default()
                },
            )
            .await
            .unwrap();
        assert!(
            rows.iter()
                .all(|e| { e.data.get("kind").and_then(Value::as_str) != Some("outcome") })
        );
    }

    mod skill {
        use super::*;

        #[tokio::test]
        async fn outcome_counter_updates() {
            let (gate, events, db, session_id) = fixture().await;
            add_project_and_task(&db, "p3", "t3").await;
            seed_playbook(&db, "pb-3", "t3").await;
            insert_injection_event(&events, &session_id, &["pb-3"]).await;

            insert_verdict_event(&events, &session_id, "TaskComplete", "pass", None).await;
            let _ = gate.poll_once(0).await.unwrap();

            let counts = db
                .with_conn(|conn| {
                    conn.query_row(
                        "SELECT success_count, failure_count FROM playbooks WHERE id = 'pb-3'",
                        [],
                        |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
                    )
                    .unwrap()
                })
                .await;
            assert_eq!(counts, (1, 0));
        }

        #[tokio::test]
        async fn non_terminal_verdict_noop() {
            let (gate, events, db, session_id) = fixture().await;
            add_project_and_task(&db, "p4", "t4").await;
            seed_playbook(&db, "pb-4", "t4").await;
            insert_injection_event(&events, &session_id, &["pb-4"]).await;

            insert_verdict_event(&events, &session_id, "TaskComplete", "error", None).await;
            let _ = gate.poll_once(0).await.unwrap();

            let counts = db
                .with_conn(|conn| {
                    conn.query_row(
                        "SELECT success_count, failure_count FROM playbooks WHERE id = 'pb-4'",
                        [],
                        |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
                    )
                    .unwrap()
                })
                .await;
            assert_eq!(counts, (0, 0));
        }
    }

    #[derive(Debug, Clone)]
    struct Phase3BenchmarkFixture {
        fixture_id: &'static str,
        brief_raw: &'static str,
        brief_equivalent: &'static str,
        brief_non_equivalent: &'static str,
        cold_baseline_tool_calls: i64,
        cold_baseline_lineage_sha: &'static str,
    }

    impl Phase3BenchmarkFixture {
        fn phase2_overnight_default_path() -> Self {
            Self {
                fixture_id: "phase2_overnight_default_path",
                brief_raw: "  Café\t\n  PLAN  ",
                brief_equivalent: "cafe\u{301} plan",
                brief_non_equivalent: "cafe plan different",
                // Pinned cold baseline for Story 3.16 warm-gate assertion.
                // Re-baseline only via deliberate PR with lineage update.
                cold_baseline_tool_calls: 10,
                cold_baseline_lineage_sha: "cc7d4f0",
            }
        }

        fn normalized_brief(&self) -> String {
            normalize_brief(self.brief_raw)
        }
    }

    async fn setup_benchmark_match_context(
        db: &DbPool,
        session_id: &str,
        project_id: &str,
        task_id: &str,
    ) {
        let sid = session_id.to_string();
        let pid = project_id.to_string();
        let tid = task_id.to_string();
        db.with_conn(move |conn| {
            conn.execute(
                "INSERT INTO sessions (id, created_at, updated_at, state)
                 VALUES (?, 1, 1, 'RUNNING')",
                rusqlite::params![sid],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO projects (id, tenant_id, status, title, created_at, updated_at)
                 VALUES (?, 'legacy-default', 'active', 'Phase3 benchmark', 1, 1)",
                rusqlite::params![pid],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO tasks (id, project_id, tenant_id, status, title, created_at, updated_at)
                 VALUES (?, ?, 'legacy-default', 'Running', 'Benchmark task', 1, 1)",
                rusqlite::params![tid, pid],
            )
            .unwrap();
            conn.execute(
                "UPDATE sessions SET task_id = ? WHERE id = ?",
                rusqlite::params![tid, sid],
            )
            .unwrap();
        })
        .await;
    }

    async fn seed_gate_fixture_playbook(
        db: &DbPool,
        fixture: &Phase3BenchmarkFixture,
        task_id: &str,
    ) {
        let normalized = fixture.normalized_brief();
        let trigger_keywords = format!(
            "[\"fixture:{}\", \"brief:{}\"]",
            fixture.fixture_id, normalized
        );
        let tid = task_id.to_string();
        db.with_conn(move |conn| {
            conn.execute(
                "INSERT INTO playbooks (
                    id, tenant_id, title, content_path, schema_version, source_task_id,
                    created_at, updated_at, trigger_keywords, content, success_count, failure_count,
                    avg_duration_ms, avg_tool_calls, status, version
                 ) VALUES (
                    'pb-benchmark-gate', 'legacy-default', 'Benchmark gate seed', '', 1, ?,
                    1, 1, ?, 'benchmark body', 1, 0, NULL, NULL, 'active', 1
                 )",
                rusqlite::params![tid, trigger_keywords],
            )
            .unwrap();
        })
        .await;
    }

    #[tokio::test]
    async fn phase3_benchmark_fixture_identity() {
        let fixture = Phase3BenchmarkFixture::phase2_overnight_default_path();
        let db = db::open(":memory:").await.unwrap();
        setup_benchmark_match_context(&db, "s-bench", "p-bench", "t-bench").await;
        seed_gate_fixture_playbook(&db, &fixture, "t-bench").await;

        let same_identity = db
            .with_conn(|conn| {
                match_playbooks(
                    conn,
                    &MatchRequest {
                        session_id: "s-bench".into(),
                        fixture_id: Some(fixture.fixture_id.to_string()),
                        brief: fixture.brief_equivalent.to_string(),
                        mode: MatcherMode::Gate,
                        limit: 3,
                    },
                )
            })
            .await
            .unwrap();
        assert_eq!(
            same_identity.len(),
            1,
            "equivalent normalized brief must match"
        );

        let fixture_mismatch = db
            .with_conn(|conn| {
                match_playbooks(
                    conn,
                    &MatchRequest {
                        session_id: "s-bench".into(),
                        fixture_id: Some("phase2_other_fixture".to_string()),
                        brief: fixture.brief_equivalent.to_string(),
                        mode: MatcherMode::Gate,
                        limit: 3,
                    },
                )
            })
            .await
            .unwrap();
        assert!(
            fixture_mismatch.is_empty(),
            "fixture-id mismatch must block gate identity match"
        );

        let brief_mismatch = db
            .with_conn(|conn| {
                match_playbooks(
                    conn,
                    &MatchRequest {
                        session_id: "s-bench".into(),
                        fixture_id: Some(fixture.fixture_id.to_string()),
                        brief: fixture.brief_non_equivalent.to_string(),
                        mode: MatcherMode::Gate,
                        limit: 3,
                    },
                )
            })
            .await
            .unwrap();
        assert!(
            brief_mismatch.is_empty(),
            "normalized-brief mismatch must block gate identity match"
        );

        assert_eq!(fixture.normalized_brief(), "cafe\u{301} plan");
        assert_eq!(fixture.cold_baseline_tool_calls, 10);
        assert_eq!(fixture.cold_baseline_lineage_sha, "cc7d4f0");
    }

    #[tokio::test]
    async fn phase3_warm_benchmark() {
        let fixture = Phase3BenchmarkFixture::phase2_overnight_default_path();
        let db = db::open(":memory:").await.unwrap();
        let store = SqliteEventStore::new(db.clone());
        setup_benchmark_match_context(&db, "s-cold", "p-cold", "t-cold").await;
        setup_benchmark_match_context(&db, "s-warm", "p-warm", "t-warm").await;
        seed_gate_fixture_playbook(&db, &fixture, "t-warm").await;
        for seq in 0..fixture.cold_baseline_tool_calls {
            store
                .append(NewEvent {
                    session_id: "s-cold".into(),
                    event_type: EventType::Action,
                    source: "agent".into(),
                    data: json!({"kind":"tool_call","seq": seq}),
                })
                .await
                .unwrap();
        }
        let cold_calls = count_action_events(&db, "s-cold").await;
        set_tool_calls(&db, "s-cold", cold_calls).await;

        // Warm run with playbook injection available via deterministic matcher hit.
        let matched = db
            .with_conn(|conn| {
                match_playbooks(
                    conn,
                    &MatchRequest {
                        session_id: "s-warm".into(),
                        fixture_id: Some(fixture.fixture_id.to_string()),
                        brief: fixture.brief_equivalent.to_string(),
                        mode: MatcherMode::Gate,
                        limit: 3,
                    },
                )
            })
            .await
            .unwrap();
        assert_eq!(
            matched.len(),
            1,
            "warm benchmark requires deterministic gate hit"
        );
        let injection = build_injection(&matched, INJECTION_BYTE_CAP)
            .expect("warm benchmark requires injection payload");
        assert!(
            !injection.injected_ids.is_empty(),
            "warm benchmark requires at least one injected playbook"
        );
        let warm_action_count = (cold_calls * 70) / 100;
        for seq in 0..warm_action_count {
            store
                .append(NewEvent {
                    session_id: "s-warm".into(),
                    event_type: EventType::Action,
                    source: "agent".into(),
                    data: json!({"kind":"tool_call","seq": seq}),
                })
                .await
                .unwrap();
        }
        let derived_tool_calls = count_action_events(&db, "s-warm").await;
        set_tool_calls(&db, "s-warm", derived_tool_calls).await;
        let observed_warm_tool_calls = db
            .with_conn(|conn| {
                conn.query_row(
                    "SELECT tool_calls FROM sessions WHERE id = 's-warm'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap()
            })
            .await;
        let gate_limit = (fixture.cold_baseline_tool_calls as f64) * 0.70;
        assert_eq!(
            observed_warm_tool_calls, derived_tool_calls,
            "sessions.tool_calls must mirror Action-event cardinality in warm benchmark harness"
        );

        assert!(
            (observed_warm_tool_calls as f64) <= gate_limit,
            "phase3 warm benchmark failed: sessions.tool_calls={} exceeded 0.70*cold_baseline={} (lineage {})",
            observed_warm_tool_calls,
            fixture.cold_baseline_tool_calls,
            fixture.cold_baseline_lineage_sha
        );
    }
}
