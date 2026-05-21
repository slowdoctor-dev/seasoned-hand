use std::collections::{HashMap, HashSet};
use std::time::Duration;

use rusqlite::params;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::db::DbPool;
use crate::events::{EventStore, EventType, NewEvent, sqlite::SqliteEventStore};
use crate::time::now_micros;

/// Default cadence when `SH_USER_COST_INTERVAL_SEC` is unset.
pub const DEFAULT_USER_COST_INTERVAL_SECS: u64 = 3600;
pub const DEFAULT_USER_COST_RECONCILE_INTERVAL_SECS: u64 = 24 * 3600;

/// Sentinel constants — match V013 bootstrap inserts so unattributable
/// sessions still produce a ledger row instead of silently dropping.
const SENTINEL_TENANT: &str = "legacy-default";
const SENTINEL_ORG: &str = "org-legacy-default";
const SENTINEL_USER: &str = "user-legacy-admin";

/// Nearline writer for `user_cost_ledger`. One instance is spawned by the
/// server alongside curator retention (1h default cadence; configurable
/// via `SH_USER_COST_INTERVAL_SEC`).
#[derive(Clone)]
pub struct NearlineWriter {
    db: DbPool,
}

#[derive(Clone)]
pub struct ReconciliationJob {
    db: DbPool,
    events: std::sync::Arc<SqliteEventStore>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlushReport {
    /// Number of distinct `(tenant_id, user_id, month_yyyymm)` ledger
    /// rows touched by this flush — useful for ops dashboards and the
    /// reconciliation job (story 5.13).
    pub rows_upserted: usize,
    /// High watermark of `events.id` observed at scan time. Stored on
    /// each touched row so reconciliation can detect drift.
    pub high_watermark_event_id: Option<i64>,
}

#[derive(Debug, Error)]
pub enum NearlineWriterError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
}

#[derive(Debug, Error)]
pub enum ReconciliationError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("event store: {0}")]
    Event(#[from] crate::events::EventError),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DriftFinding {
    pub tenant_id: String,
    pub user_id: String,
    pub month_yyyymm: String,
    pub expected_cost_cents: i64,
    pub observed_cost_cents: i64,
    pub delta_pct: f64,
    pub expected_session_count: i64,
    pub observed_session_count: i64,
    pub expected_tool_calls: i64,
    pub observed_tool_calls: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReconciliationReport {
    pub rows_checked: usize,
    pub drifted_rows: usize,
    pub drifts: Vec<DriftFinding>,
}

#[derive(Debug, Clone)]
struct Bucket {
    tenant_id: String,
    organization_id: String,
    user_id: String,
    month_yyyymm: String,
    session_count: i64,
    tool_calls: i64,
    cost_cents: i64,
}

impl NearlineWriter {
    pub fn new(db: DbPool) -> Self {
        Self { db }
    }

    /// Recompute monthly cost rollups for every `(tenant, user, month)`
    /// bucket that has at least one finalized session. Idempotent —
    /// running twice in a row converges to the same value because each
    /// UPSERT replaces (not increments) the totals.
    pub async fn flush(&self) -> Result<FlushReport, NearlineWriterError> {
        let now = now_micros();
        let report = self
            .db
            .with_conn(move |conn| {
                // Compute the event watermark up-front so every row this
                // flush touches gets the same value.
                let high_watermark: Option<i64> = conn
                    .query_row("SELECT MAX(id) FROM events", [], |r| r.get(0))
                    .unwrap_or(None);

                // Aggregate per bucket. `sessions` has no `tenant_id`
                // column (it predates V013); resolve via task → project,
                // falling back to the sentinel when both joins miss.
                //
                // `users.organization_memberships.is_primary=1` resolves
                // the actor's billing org — for legacy sessions without a
                // user, the sentinel admin's primary membership applies.
                let mut stmt = conn.prepare(
                    "WITH session_buckets AS (
                       SELECT
                         COALESCE(p.tenant_id, t.tenant_id, ?) AS tenant_id,
                         COALESCE(s.user_id, ?) AS user_id,
                         strftime('%Y%m',
                                  CAST(s.updated_at AS INTEGER) / 1000000,
                                  'unixepoch') AS month_yyyymm,
                         s.cost_cents,
                         s.tool_calls
                       FROM sessions s
                       LEFT JOIN tasks t ON t.id = s.task_id
                       LEFT JOIN projects p
                         ON p.id = COALESCE(t.project_id, s.project_id)
                       WHERE s.state IN ('FINISHED','ERROR')
                     )
                     SELECT
                       b.tenant_id,
                       COALESCE(m.organization_id, ?) AS organization_id,
                       b.user_id,
                       b.month_yyyymm,
                       COUNT(*) AS session_count,
                       COALESCE(SUM(b.tool_calls), 0) AS tool_calls,
                       COALESCE(SUM(b.cost_cents), 0) AS cost_cents
                     FROM session_buckets b
                     LEFT JOIN organization_memberships m
                       ON m.user_id = b.user_id AND m.is_primary = 1
                     GROUP BY b.tenant_id, organization_id,
                              b.user_id, b.month_yyyymm",
                )?;
                let buckets: Vec<Bucket> = stmt
                    .query_map(params![SENTINEL_TENANT, SENTINEL_USER, SENTINEL_ORG], |r| {
                        Ok(Bucket {
                            tenant_id: r.get(0)?,
                            organization_id: r.get(1)?,
                            user_id: r.get(2)?,
                            month_yyyymm: r.get(3)?,
                            session_count: r.get(4)?,
                            tool_calls: r.get(5)?,
                            cost_cents: r.get(6)?,
                        })
                    })?
                    .collect::<Result<Vec<_>, _>>()?;
                drop(stmt);

                let tx = conn.transaction()?;
                for b in &buckets {
                    let row_id = format!("ucl-{}", Uuid::new_v4());
                    // UPSERT key: (tenant_id, user_id, month_yyyymm).
                    // Replace totals on conflict so re-running this
                    // flush is idempotent. `created_at` is only set on
                    // first insert via `excluded.created_at` semantics —
                    // re-runs preserve the original creation timestamp.
                    tx.execute(
                        "INSERT INTO user_cost_ledger
                           (id, tenant_id, organization_id, user_id,
                            month_yyyymm, session_count, tool_calls, cost_cents,
                            source_low_watermark_event_id,
                            source_high_watermark_event_id,
                            reconciled_at, created_at, updated_at)
                         VALUES (?, ?, ?, ?, ?, ?, ?, ?, NULL, ?, ?, ?, ?)
                         ON CONFLICT(tenant_id, user_id, month_yyyymm) DO UPDATE SET
                           organization_id = excluded.organization_id,
                           session_count   = excluded.session_count,
                           tool_calls      = excluded.tool_calls,
                           cost_cents      = excluded.cost_cents,
                           source_high_watermark_event_id =
                             excluded.source_high_watermark_event_id,
                           reconciled_at   = excluded.reconciled_at,
                           updated_at      = excluded.updated_at",
                        params![
                            row_id,
                            b.tenant_id,
                            b.organization_id,
                            b.user_id,
                            b.month_yyyymm,
                            b.session_count,
                            b.tool_calls,
                            b.cost_cents,
                            high_watermark,
                            now,
                            now,
                            now,
                        ],
                    )?;
                }
                tx.commit()?;

                Ok::<FlushReport, rusqlite::Error>(FlushReport {
                    rows_upserted: buckets.len(),
                    high_watermark_event_id: high_watermark,
                })
            })
            .await?;
        Ok(report)
    }

    /// Drive [`flush`](Self::flush) on a fixed cadence until `shutdown`
    /// fires. Errors are logged but never propagated — the writer is
    /// designed to keep ticking through transient DB hiccups so the
    /// monthly rollups stay live for ops.
    pub async fn run(self, interval: Duration, shutdown: CancellationToken) {
        let mut ticker = tokio::time::interval(interval);
        // First tick fires immediately; consume so we don't double-flush
        // right after spawn.
        ticker.tick().await;
        loop {
            tokio::select! {
                _ = shutdown.cancelled() => break,
                _ = ticker.tick() => {
                    match self.flush().await {
                        Ok(report) => tracing::debug!(
                            rows_upserted = report.rows_upserted,
                            high_watermark = ?report.high_watermark_event_id,
                            "user_cost_ledger flush ok",
                        ),
                        Err(err) => tracing::warn!(error = %err, "user_cost_ledger flush failed"),
                    }
                }
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ReconcileKey {
    tenant_id: String,
    user_id: String,
    month_yyyymm: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReconcileMetrics {
    cost_cents: i64,
    session_count: i64,
    tool_calls: i64,
}

impl ReconciliationJob {
    pub fn new(db: DbPool, events: std::sync::Arc<SqliteEventStore>) -> Self {
        Self { db, events }
    }

    pub async fn run(
        &self,
        month_yyyymm: &str,
    ) -> Result<ReconciliationReport, ReconciliationError> {
        let month = month_yyyymm.to_string();
        let expected = self.expected_for_month(&month).await?;
        let observed = self.observed_for_month(&month).await?;

        let mut keys: HashSet<ReconcileKey> = expected.keys().cloned().collect();
        keys.extend(observed.keys().cloned());

        let rows_checked = keys.len();
        let mut drifts = Vec::new();
        for key in keys {
            let expected_metrics = expected.get(&key).cloned().unwrap_or(ReconcileMetrics {
                cost_cents: 0,
                session_count: 0,
                tool_calls: 0,
            });
            let observed_metrics = observed.get(&key).cloned().unwrap_or(ReconcileMetrics {
                cost_cents: 0,
                session_count: 0,
                tool_calls: 0,
            });
            let delta_pct = delta_pct(expected_metrics.cost_cents, observed_metrics.cost_cents);
            if is_drift(
                expected_metrics.cost_cents,
                observed_metrics.cost_cents,
                delta_pct,
            ) {
                let finding = DriftFinding {
                    tenant_id: key.tenant_id.clone(),
                    user_id: key.user_id.clone(),
                    month_yyyymm: key.month_yyyymm.clone(),
                    expected_cost_cents: expected_metrics.cost_cents,
                    observed_cost_cents: observed_metrics.cost_cents,
                    delta_pct,
                    expected_session_count: expected_metrics.session_count,
                    observed_session_count: observed_metrics.session_count,
                    expected_tool_calls: expected_metrics.tool_calls,
                    observed_tool_calls: observed_metrics.tool_calls,
                };
                self.emit_drift_event(&finding).await?;
                drifts.push(finding);
            }
        }
        Ok(ReconciliationReport {
            rows_checked,
            drifted_rows: drifts.len(),
            drifts,
        })
    }

    pub async fn run_daily(self, interval: Duration, shutdown: CancellationToken) {
        let mut ticker = tokio::time::interval(interval);
        ticker.tick().await;
        loop {
            tokio::select! {
                _ = shutdown.cancelled() => break,
                _ = ticker.tick() => {
                    match self.current_and_previous_months().await {
                        Ok((current, previous)) => {
                            for month in [previous, current] {
                                match self.run(&month).await {
                                    Ok(report) => tracing::debug!(
                                        month_yyyymm = %month,
                                        rows_checked = report.rows_checked,
                                        drifted_rows = report.drifted_rows,
                                        "user_cost reconciliation run ok",
                                    ),
                                    Err(error) => tracing::warn!(
                                        month_yyyymm = %month,
                                        %error,
                                        "user_cost reconciliation run failed",
                                    ),
                                }
                            }
                        }
                        Err(error) => tracing::warn!(%error, "user_cost reconciliation month-resolve failed"),
                    }
                }
            }
        }
    }

    async fn current_and_previous_months(&self) -> Result<(String, String), ReconciliationError> {
        let now = now_micros();
        let tuple = self
            .db
            .with_conn(move |conn| {
                conn.query_row(
                    "SELECT
                        strftime('%Y%m', CAST(? AS INTEGER) / 1000000, 'unixepoch'),
                        strftime('%Y%m', CAST(? AS INTEGER) / 1000000, 'unixepoch', 'start of month', '-1 month')",
                    params![now, now],
                    |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
                )
            })
            .await?;
        Ok(tuple)
    }

    async fn expected_for_month(
        &self,
        month_yyyymm: &str,
    ) -> Result<HashMap<ReconcileKey, ReconcileMetrics>, ReconciliationError> {
        let month = month_yyyymm.to_string();
        let rows = self
            .db
            .with_conn(move |conn| {
                let mut stmt = conn.prepare(
                    "WITH session_buckets AS (
                       SELECT
                         COALESCE(p.tenant_id, t.tenant_id, ?) AS tenant_id,
                         COALESCE(s.user_id, ?) AS user_id,
                         strftime('%Y%m',
                                  CAST(s.updated_at AS INTEGER) / 1000000,
                                  'unixepoch') AS month_yyyymm,
                         s.cost_cents,
                         s.tool_calls
                       FROM sessions s
                       LEFT JOIN tasks t ON t.id = s.task_id
                       LEFT JOIN projects p
                         ON p.id = COALESCE(t.project_id, s.project_id)
                       WHERE s.state IN ('FINISHED','ERROR')
                     )
                     SELECT tenant_id, user_id, month_yyyymm,
                            COUNT(*) AS session_count,
                            COALESCE(SUM(tool_calls), 0) AS tool_calls,
                            COALESCE(SUM(cost_cents), 0) AS cost_cents
                     FROM session_buckets
                     WHERE month_yyyymm = ?
                     GROUP BY tenant_id, user_id, month_yyyymm",
                )?;
                let rows = stmt
                    .query_map(params![SENTINEL_TENANT, SENTINEL_USER, month], |r| {
                        Ok((
                            ReconcileKey {
                                tenant_id: r.get(0)?,
                                user_id: r.get(1)?,
                                month_yyyymm: r.get(2)?,
                            },
                            ReconcileMetrics {
                                session_count: r.get(3)?,
                                tool_calls: r.get(4)?,
                                cost_cents: r.get(5)?,
                            },
                        ))
                    })?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok::<Vec<(ReconcileKey, ReconcileMetrics)>, rusqlite::Error>(rows)
            })
            .await?;
        Ok(rows.into_iter().collect())
    }

    async fn observed_for_month(
        &self,
        month_yyyymm: &str,
    ) -> Result<HashMap<ReconcileKey, ReconcileMetrics>, ReconciliationError> {
        let month = month_yyyymm.to_string();
        let rows = self
            .db
            .with_conn(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT tenant_id, user_id, month_yyyymm, session_count, tool_calls, cost_cents
                     FROM user_cost_ledger
                     WHERE month_yyyymm = ?",
                )?;
                let rows = stmt
                    .query_map(params![month], |r| {
                        Ok((
                            ReconcileKey {
                                tenant_id: r.get(0)?,
                                user_id: r.get(1)?,
                                month_yyyymm: r.get(2)?,
                            },
                            ReconcileMetrics {
                                session_count: r.get(3)?,
                                tool_calls: r.get(4)?,
                                cost_cents: r.get(5)?,
                            },
                        ))
                    })?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok::<Vec<(ReconcileKey, ReconcileMetrics)>, rusqlite::Error>(rows)
            })
            .await?;
        Ok(rows.into_iter().collect())
    }

    async fn emit_drift_event(&self, finding: &DriftFinding) -> Result<(), ReconciliationError> {
        let session_id = format!("audit:{}", finding.tenant_id);
        self.ensure_session(&session_id).await?;
        self.events
            .append(NewEvent {
                session_id,
                event_type: EventType::Misc,
                source: "user_cost_reconciliation".to_string(),
                data: serde_json::json!({
                    "kind": "user_cost_reconciliation_drift",
                    "tenant_id": finding.tenant_id,
                    "user_id": finding.user_id,
                    "month_yyyymm": finding.month_yyyymm,
                    "expected_cost_cents": finding.expected_cost_cents,
                    "observed_cost_cents": finding.observed_cost_cents,
                    "delta_pct": finding.delta_pct,
                    "expected_session_count": finding.expected_session_count,
                    "observed_session_count": finding.observed_session_count,
                    "expected_tool_calls": finding.expected_tool_calls,
                    "observed_tool_calls": finding.observed_tool_calls,
                }),
            })
            .await?;
        Ok(())
    }

    async fn ensure_session(&self, session_id: &str) -> Result<(), ReconciliationError> {
        let session_id = session_id.to_string();
        let now = now_micros();
        self.db
            .with_conn(move |conn| {
                conn.execute(
                    "INSERT OR IGNORE INTO sessions (id, created_at, updated_at, state)
                     VALUES (?, ?, ?, 'IDLE')",
                    params![session_id, now, now],
                )?;
                Ok::<(), rusqlite::Error>(())
            })
            .await?;
        Ok(())
    }
}

/// Relative drift between the expected (recomputed-from-source) and the
/// observed (ledger) cost. Returns a fraction in [0.0, 1.0+).
///
/// Hardening P5-HARD-IT1-L3: when `expected == 0`, any nonzero `observed`
/// — positive OR negative — yields 1.0 (treated as full drift). This is
/// deliberate: a ledger row showing cost where the source shows none
/// (including an impossible negative cost, which has no DB CHECK) is a
/// reconciliation discrepancy that MUST surface as a drift finding so an
/// operator investigates. Over-reporting here is the safe direction.
fn delta_pct(expected: i64, observed: i64) -> f64 {
    if expected == 0 {
        if observed == 0 { 0.0 } else { 1.0 }
    } else {
        ((expected - observed).abs() as f64) / (expected as f64)
    }
}

fn is_drift(expected: i64, observed: i64, delta_pct: f64) -> bool {
    (expected == 0 && observed > 0) || delta_pct > 0.005
}
