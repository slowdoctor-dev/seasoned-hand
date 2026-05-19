//! Curator telemetry retention + compaction (story 4.23, NFR-4.4 close).
//!
//! Owns the per-project compaction tail for Phase 4 Curator output:
//! - `curator_decisions` rows past the 90-day hot window collapse into per-week
//!   per-decision-type histogram rows in `curator_decisions_summary`; raw rows
//!   are then deleted in the same transaction (NFR-4.3 atomicity).
//! - `retrospective_citations` rows past the window are pruned; the parent
//!   `weekly_retrospectives` narrative is retained (it is already a summary).
//! - `curator_search_index` rows past the window are pruned; the V011
//!   `curator_search_index_ad` trigger cascades the FTS delete.
//!
//! When the project SQLite footprint exceeds the 300 MB cap, the job emits
//! a `curator_storage_cap_warning` Misc event and shortens the hot window to
//! 60 days until the next cycle observes the footprint back under cap.
//!
//! refs: /specs/phase-4/stories/story-4.23.md, NFR-4.4, NFR-4.3

use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use rusqlite::params;
use serde_json::json;

use crate::curator::CuratorWorkerError;
use crate::db::DbPool;
use crate::events::{EventStore, EventType, NewEvent, sqlite::SqliteEventStore};

pub const MICROS_PER_DAY: i64 = 86_400 * 1_000_000;
pub const DEFAULT_HOT_WINDOW_DAYS: i64 = 90;
pub const DEFAULT_ACCELERATED_WINDOW_DAYS: i64 = 60;
pub const DEFAULT_STORAGE_CAP_BYTES: u64 = 300 * 1024 * 1024;
const WEEK_MICROS: i64 = 7 * MICROS_PER_DAY;

#[derive(Debug, Clone)]
pub struct RetentionConfig {
    pub project_id: String,
    pub hot_window_days: i64,
    pub accelerated_window_days: i64,
    pub storage_cap_bytes: u64,
    pub session_id: String,
}

impl RetentionConfig {
    pub fn for_project(project_id: impl Into<String>) -> Self {
        let project_id = project_id.into();
        let session_id = format!("curator-retention:{project_id}");
        Self {
            project_id,
            hot_window_days: DEFAULT_HOT_WINDOW_DAYS,
            accelerated_window_days: DEFAULT_ACCELERATED_WINDOW_DAYS,
            storage_cap_bytes: DEFAULT_STORAGE_CAP_BYTES,
            session_id,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct RetentionReport {
    pub project_id: String,
    pub effective_window_days: i64,
    pub cutoff_micros: i64,
    pub raw_decisions_pruned: u64,
    pub summary_rows_written: u64,
    pub citations_pruned: u64,
    pub search_rows_pruned: u64,
    pub storage_bytes_before: u64,
    pub storage_bytes_after: u64,
    pub cap_warning_emitted: bool,
    pub elapsed_ms: u64,
}

pub struct CuratorRetentionJob {
    db: DbPool,
    events: Arc<SqliteEventStore>,
    config: RetentionConfig,
}

impl CuratorRetentionJob {
    pub fn new(db: DbPool, events: Arc<SqliteEventStore>, config: RetentionConfig) -> Self {
        Self { db, events, config }
    }

    pub fn config(&self) -> &RetentionConfig {
        &self.config
    }

    /// Run one retention/compaction cycle. `now_micros` is injected so tests
    /// can pin the clock; production callers should pass [`current_now_micros`].
    pub async fn run_cycle(&self, now_micros: i64) -> Result<RetentionReport, CuratorWorkerError> {
        let started = Instant::now();
        self.ensure_session().await?;

        let storage_before = self.storage_bytes().await?;
        let cap_warning = storage_before > self.config.storage_cap_bytes;
        let window_days = if cap_warning {
            self.config.accelerated_window_days
        } else {
            self.config.hot_window_days
        };
        let cutoff = now_micros.saturating_sub(window_days * MICROS_PER_DAY);

        if cap_warning {
            self.events
                .append(NewEvent {
                    session_id: self.config.session_id.clone(),
                    event_type: EventType::Misc,
                    source: "curator-retention".to_string(),
                    data: json!({
                        "kind": "curator_storage_cap_warning",
                        "project_id": self.config.project_id,
                        "current_bytes": storage_before,
                        "cap_bytes": self.config.storage_cap_bytes,
                        "effective_window_days": window_days,
                    }),
                })
                .await?;
        }

        let project_id = self.config.project_id.clone();
        let counts = self
            .db
            .with_conn(
                move |conn| -> Result<CompactionCounts, CuratorWorkerError> {
                    compact_and_prune(conn, &project_id, cutoff, now_micros)
                },
            )
            .await?;

        let storage_after = self.storage_bytes().await?;
        let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);

        self.events
            .append(NewEvent {
                session_id: self.config.session_id.clone(),
                event_type: EventType::Misc,
                source: "curator-retention".to_string(),
                data: json!({
                    "kind": "curator_retention_cycle_completed",
                    "project_id": self.config.project_id,
                    "effective_window_days": window_days,
                    "raw_pruned": counts.raw_decisions_pruned,
                    "summarized_written": counts.summary_rows_written,
                    "citations_pruned": counts.citations_pruned,
                    "search_rows_pruned": counts.search_rows_pruned,
                    "storage_bytes_before": storage_before,
                    "storage_bytes_after": storage_after,
                    "cap_warning_emitted": cap_warning,
                    "elapsed_ms": elapsed_ms,
                }),
            })
            .await?;

        Ok(RetentionReport {
            project_id: self.config.project_id.clone(),
            effective_window_days: window_days,
            cutoff_micros: cutoff,
            raw_decisions_pruned: counts.raw_decisions_pruned,
            summary_rows_written: counts.summary_rows_written,
            citations_pruned: counts.citations_pruned,
            search_rows_pruned: counts.search_rows_pruned,
            storage_bytes_before: storage_before,
            storage_bytes_after: storage_after,
            cap_warning_emitted: cap_warning,
            elapsed_ms,
        })
    }

    async fn storage_bytes(&self) -> Result<u64, CuratorWorkerError> {
        self.db
            .with_conn(|conn| -> Result<u64, CuratorWorkerError> {
                let page_count: i64 = conn.query_row("PRAGMA page_count", [], |row| row.get(0))?;
                let page_size: i64 = conn.query_row("PRAGMA page_size", [], |row| row.get(0))?;
                let product = page_count.saturating_mul(page_size).max(0);
                Ok(u64::try_from(product).unwrap_or(0))
            })
            .await
    }

    async fn ensure_session(&self) -> Result<(), CuratorWorkerError> {
        let session_id = self.config.session_id.clone();
        let now = current_now_micros()?;
        self.db
            .with_conn(move |conn| -> Result<(), CuratorWorkerError> {
                conn.execute(
                    "INSERT OR IGNORE INTO sessions (id, created_at, updated_at, state)
                     VALUES (?1, ?2, ?2, 'RUNNING')",
                    params![session_id, now],
                )?;
                Ok(())
            })
            .await
    }
}

#[derive(Debug, Default)]
struct CompactionCounts {
    raw_decisions_pruned: u64,
    summary_rows_written: u64,
    citations_pruned: u64,
    search_rows_pruned: u64,
}

fn compact_and_prune(
    conn: &mut rusqlite::Connection,
    project_id: &str,
    cutoff: i64,
    now_micros: i64,
) -> Result<CompactionCounts, CuratorWorkerError> {
    let tx = conn.transaction()?;
    let mut counts = CompactionCounts::default();

    // 1) Aggregate aged curator_decisions per (week_start, decision_type).
    let aggregates: Vec<(i64, String, i64, Option<f64>)> = {
        let mut stmt = tx.prepare(
            "SELECT
                 (created_at / ?1) * ?1 AS week_start,
                 decision_type,
                 COUNT(*) AS decision_count,
                 AVG(confidence) AS mean_confidence
             FROM curator_decisions
             WHERE project_id = ?2 AND created_at < ?3
             GROUP BY week_start, decision_type
             ORDER BY week_start ASC, decision_type ASC",
        )?;
        let rows = stmt.query_map(params![WEEK_MICROS, project_id, cutoff], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, Option<f64>>(3)?,
            ))
        })?;
        rows.collect::<Result<Vec<_>, _>>()?
    };

    for (week_start, decision_type, decision_count, mean_confidence) in aggregates {
        let week_end = week_start.saturating_add(WEEK_MICROS).saturating_sub(1);
        let summary_id = format!("cds-{}", uuid::Uuid::new_v4());
        // UPSERT semantics: if a prior cycle already wrote this bucket the row
        // is preserved and the count/mean update — keeps re-runs idempotent
        // and tolerates a stragglers-arrived-late scenario.
        let affected = tx.execute(
            "INSERT INTO curator_decisions_summary (
                 id, tenant_id, project_id, week_start, week_end, decision_type,
                 decision_count, mean_confidence, created_at
             )
             VALUES (?1, NULL, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(project_id, week_start, week_end, decision_type) DO UPDATE
             SET decision_count = decision_count + excluded.decision_count,
                 mean_confidence = CASE
                     WHEN curator_decisions_summary.mean_confidence IS NULL
                          THEN excluded.mean_confidence
                     WHEN excluded.mean_confidence IS NULL
                          THEN curator_decisions_summary.mean_confidence
                     ELSE (
                         (curator_decisions_summary.mean_confidence
                              * curator_decisions_summary.decision_count
                          + excluded.mean_confidence * excluded.decision_count)
                         / (curator_decisions_summary.decision_count
                              + excluded.decision_count)
                     )
                 END",
            params![
                summary_id,
                project_id,
                week_start,
                week_end,
                decision_type,
                decision_count,
                mean_confidence,
                now_micros,
            ],
        )?;
        counts.summary_rows_written = counts.summary_rows_written.saturating_add(affected as u64);
    }

    let raw_pruned = tx.execute(
        "DELETE FROM curator_decisions WHERE project_id = ?1 AND created_at < ?2",
        params![project_id, cutoff],
    )?;
    counts.raw_decisions_pruned = raw_pruned as u64;

    // 2) Prune retrospective_citations whose parent retrospective is older than cutoff.
    let citations_pruned = tx.execute(
        "DELETE FROM retrospective_citations
         WHERE retrospective_id IN (
             SELECT id FROM weekly_retrospectives
             WHERE project_id = ?1 AND created_at < ?2
         )",
        params![project_id, cutoff],
    )?;
    counts.citations_pruned = citations_pruned as u64;

    // 3) Prune curator_search_index rows older than cutoff. FTS5 cascade is
    //    handled by the V011 `curator_search_index_ad` trigger.
    let search_pruned = tx.execute(
        "DELETE FROM curator_search_index WHERE project_id = ?1 AND created_at < ?2",
        params![project_id, cutoff],
    )?;
    counts.search_rows_pruned = search_pruned as u64;

    tx.commit()?;
    Ok(counts)
}

pub fn current_now_micros() -> Result<i64, CuratorWorkerError> {
    let micros = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|err| CuratorWorkerError::Executor(err.to_string()))?
        .as_micros();
    i64::try_from(micros).map_err(|err| CuratorWorkerError::Executor(err.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use rusqlite::params;

    async fn setup() -> (DbPool, Arc<SqliteEventStore>) {
        let db = db::open(":memory:").await.expect("open db");
        let events = Arc::new(SqliteEventStore::new(db.clone()));
        (db, events)
    }

    fn iso_micros(now: i64, days_ago: i64) -> i64 {
        now - days_ago * MICROS_PER_DAY
    }

    async fn seed_decision(
        db: &DbPool,
        project_id: &str,
        cycle_id: &str,
        decision_type: &str,
        confidence: f64,
        created_at: i64,
    ) -> String {
        let id = format!("cd-test-{}", uuid::Uuid::new_v4());
        let id_for_move = id.clone();
        let pid = project_id.to_string();
        let cid = cycle_id.to_string();
        let dtype = decision_type.to_string();
        db.with_conn(move |conn| {
            conn.execute(
                "INSERT INTO curator_decisions (
                     id, tenant_id, project_id, cycle_id, decision_type, subject_kind,
                     subject_id, confidence, rationale_json, evidence_json, status,
                     failure_category, created_at
                 ) VALUES (?1, NULL, ?2, ?3, ?4, 'playbook', 'subj-x', ?5, '{}', '{}',
                           'applied', NULL, ?6)",
                params![id_for_move, pid, cid, dtype, confidence, created_at],
            )
            .expect("seed decision");
            Ok::<(), CuratorWorkerError>(())
        })
        .await
        .unwrap();
        id
    }

    async fn seed_retrospective_with_citation(
        db: &DbPool,
        project_id: &str,
        retro_created_at: i64,
    ) -> (String, String) {
        let retro_id = format!("retro-test-{}", uuid::Uuid::new_v4());
        let citation_id = format!("rc-test-{}", uuid::Uuid::new_v4());
        let retro_clone = retro_id.clone();
        let citation_clone = citation_id.clone();
        let pid = project_id.to_string();
        // The (project_id, week_start, week_end) tuple has a UNIQUE constraint;
        // disambiguate per seed call by deriving the week tuple from
        // `retro_created_at` instead of using a hardcoded (0, 0).
        let week_start = (retro_created_at / WEEK_MICROS) * WEEK_MICROS;
        let week_end = week_start.saturating_add(WEEK_MICROS).saturating_sub(1);
        db.with_conn(move |conn| {
            conn.execute(
                "INSERT INTO weekly_retrospectives (
                     id, tenant_id, project_id, week_start, week_end, content,
                     citation_coverage, generation_status, created_at
                 ) VALUES (?1, NULL, ?2, ?3, ?4, 'narrative', 1.0, 'success', ?5)",
                params![retro_clone, pid, week_start, week_end, retro_created_at],
            )
            .expect("seed retrospective");
            conn.execute(
                "INSERT INTO retrospective_citations (
                     id, tenant_id, retrospective_id, claim_index, citation_kind,
                     citation_ref, snippet
                 ) VALUES (?1, NULL, ?2, 0, 'event', 'evt-1', 'snippet')",
                params![citation_clone, retro_clone],
            )
            .expect("seed citation");
            Ok::<(), CuratorWorkerError>(())
        })
        .await
        .unwrap();
        (retro_id, citation_id)
    }

    async fn seed_search_row(db: &DbPool, project_id: &str, created_at: i64) -> i64 {
        let pid = project_id.to_string();
        db.with_conn(move |conn| {
            conn.execute(
                "INSERT INTO curator_search_index (
                     tenant_id, project_id, source_type, source_id, searchable_text,
                     created_at
                 ) VALUES (NULL, ?1, 'playbook', 'subj-x', 'hello world', ?2)",
                params![pid, created_at],
            )
            .expect("seed search row");
            Ok::<i64, CuratorWorkerError>(conn.last_insert_rowid())
        })
        .await
        .unwrap()
    }

    async fn count(db: &DbPool, sql: &str, project_id: &str) -> i64 {
        let sql = sql.to_string();
        let pid = project_id.to_string();
        db.with_conn(move |conn| -> Result<i64, CuratorWorkerError> {
            Ok(conn.query_row(&sql, params![pid], |row| row.get::<_, i64>(0))?)
        })
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn compaction_window_boundary() {
        let (db, events) = setup().await;
        let project = "proj-retention-boundary";

        // Pin clock far enough from epoch so a 91-day rewind stays positive.
        let now = 365 * MICROS_PER_DAY;

        // Just-inside the 90-day window — must survive untouched.
        let inside = iso_micros(now, 89);
        let kept = seed_decision(&db, project, "cyc-A", "merge", 0.91, inside).await;

        // Just outside the 90-day window — must be compacted.
        let outside = iso_micros(now, 91);
        let pruned = seed_decision(&db, project, "cyc-B", "merge", 0.71, outside).await;

        // Citation tied to a retrospective inside the window: kept.
        let (_, citation_inside) =
            seed_retrospective_with_citation(&db, project, iso_micros(now, 30)).await;
        // Citation tied to an aged retrospective: pruned.
        let (_, citation_outside) =
            seed_retrospective_with_citation(&db, project, iso_micros(now, 200)).await;

        // Search rows — one fresh, one aged.
        let fresh_row = seed_search_row(&db, project, iso_micros(now, 10)).await;
        let aged_row = seed_search_row(&db, project, iso_micros(now, 200)).await;

        let job =
            CuratorRetentionJob::new(db.clone(), events, RetentionConfig::for_project(project));
        let report = job.run_cycle(now).await.expect("retention cycle");

        assert_eq!(report.effective_window_days, DEFAULT_HOT_WINDOW_DAYS);
        assert_eq!(report.raw_decisions_pruned, 1);
        assert_eq!(report.summary_rows_written, 1);
        assert_eq!(report.citations_pruned, 1);
        assert_eq!(report.search_rows_pruned, 1);
        assert!(!report.cap_warning_emitted);

        // Verify exact survivors / casualties.
        let kept_alive = count(
            &db,
            "SELECT COUNT(*) FROM curator_decisions WHERE id = ?1",
            &kept,
        )
        .await;
        assert_eq!(kept_alive, 1, "row inside window must survive");
        let pruned_alive = count(
            &db,
            "SELECT COUNT(*) FROM curator_decisions WHERE id = ?1",
            &pruned,
        )
        .await;
        assert_eq!(pruned_alive, 0, "row outside window must be pruned");

        let summary_row: (String, i64, Option<f64>) = db
            .with_conn(|conn| {
                conn.query_row(
                    "SELECT decision_type, decision_count, mean_confidence
                     FROM curator_decisions_summary
                     WHERE project_id = 'proj-retention-boundary'",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
            })
            .await
            .expect("read summary");
        assert_eq!(summary_row.0, "merge");
        assert_eq!(summary_row.1, 1);
        assert!((summary_row.2.unwrap() - 0.71).abs() < 1e-6);

        let citation_inside_count = count(
            &db,
            "SELECT COUNT(*) FROM retrospective_citations WHERE id = ?1",
            &citation_inside,
        )
        .await;
        assert_eq!(citation_inside_count, 1);
        let citation_outside_count = count(
            &db,
            "SELECT COUNT(*) FROM retrospective_citations WHERE id = ?1",
            &citation_outside,
        )
        .await;
        assert_eq!(citation_outside_count, 0);

        let fresh_str = fresh_row.to_string();
        let aged_str = aged_row.to_string();
        let fresh_alive = count(
            &db,
            "SELECT COUNT(*) FROM curator_search_index WHERE row_id = CAST(?1 AS INTEGER)",
            &fresh_str,
        )
        .await;
        assert_eq!(fresh_alive, 1);
        let aged_alive = count(
            &db,
            "SELECT COUNT(*) FROM curator_search_index WHERE row_id = CAST(?1 AS INTEGER)",
            &aged_str,
        )
        .await;
        assert_eq!(aged_alive, 0);
    }

    #[tokio::test]
    async fn storage_cap_trigger() {
        let (db, events) = setup().await;
        let project = "proj-cap";

        let now = 365 * MICROS_PER_DAY;

        // Seed one row between the 60-day and 90-day boundaries — only the
        // accelerated 60-day window catches it.
        let between = iso_micros(now, 75);
        let between_id = seed_decision(&db, project, "cyc-A", "keep", 0.8, between).await;

        // Set the cap absurdly low so the in-memory DB always exceeds it.
        let mut config = RetentionConfig::for_project(project);
        config.storage_cap_bytes = 1;

        let job = CuratorRetentionJob::new(db.clone(), events.clone(), config);
        let report = job.run_cycle(now).await.expect("retention cycle");
        assert!(report.cap_warning_emitted);
        assert_eq!(
            report.effective_window_days,
            DEFAULT_ACCELERATED_WINDOW_DAYS
        );
        assert_eq!(report.raw_decisions_pruned, 1);

        // The cap-warning event must be visible in the event stream.
        let session = format!("curator-retention:{project}");
        let stored = events
            .query(&session, Default::default())
            .await
            .expect("query events");
        let kinds: Vec<String> = stored
            .iter()
            .filter_map(|e| {
                e.data
                    .get("kind")
                    .and_then(|v| v.as_str())
                    .map(String::from)
            })
            .collect();
        assert!(
            kinds.iter().any(|k| k == "curator_storage_cap_warning"),
            "kinds = {kinds:?}"
        );
        assert!(
            kinds
                .iter()
                .any(|k| k == "curator_retention_cycle_completed"),
            "kinds = {kinds:?}"
        );

        let between_alive = count(
            &db,
            "SELECT COUNT(*) FROM curator_decisions WHERE id = ?1",
            &between_id,
        )
        .await;
        assert_eq!(
            between_alive, 0,
            "60-day cutoff must prune the 75-day-old row"
        );
    }

    #[tokio::test]
    async fn idempotent_rerun() {
        let (db, events) = setup().await;
        let project = "proj-idem";

        let now = 365 * MICROS_PER_DAY;
        // Pin all three decisions inside the same calendar-week bucket so
        // the histogram collapses to a single summary row.
        let aged = iso_micros(now, 120);
        for i in 0..3 {
            seed_decision(&db, project, "cyc-A", "merge", 0.5 + (i as f64) * 0.1, aged).await;
        }
        let (_, _) = seed_retrospective_with_citation(&db, project, iso_micros(now, 200)).await;
        seed_search_row(&db, project, iso_micros(now, 200)).await;

        let job =
            CuratorRetentionJob::new(db.clone(), events, RetentionConfig::for_project(project));
        let first = job.run_cycle(now).await.expect("first cycle");
        assert_eq!(first.raw_decisions_pruned, 3);
        assert_eq!(first.summary_rows_written, 1);
        assert_eq!(first.citations_pruned, 1);
        assert_eq!(first.search_rows_pruned, 1);

        let second = job.run_cycle(now).await.expect("second cycle");
        assert_eq!(second.raw_decisions_pruned, 0);
        assert_eq!(second.summary_rows_written, 0);
        assert_eq!(second.citations_pruned, 0);
        assert_eq!(second.search_rows_pruned, 0);

        let summary_rows = count(
            &db,
            "SELECT COUNT(*) FROM curator_decisions_summary WHERE project_id = ?1",
            project,
        )
        .await;
        assert_eq!(
            summary_rows, 1,
            "summary table must remain stable across re-runs"
        );
        let summary_count: i64 = db
            .with_conn(|conn| {
                conn.query_row(
                    "SELECT decision_count FROM curator_decisions_summary
                     WHERE project_id = 'proj-idem'",
                    [],
                    |row| row.get(0),
                )
            })
            .await
            .expect("read summary count");
        assert_eq!(summary_count, 3);
    }
}
