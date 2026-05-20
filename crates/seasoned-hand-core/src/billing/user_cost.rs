use std::time::Duration;

use rusqlite::params;
use thiserror::Error;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::db::DbPool;
use crate::time::now_micros;

/// Default cadence when `SH_USER_COST_INTERVAL_SEC` is unset.
pub const DEFAULT_USER_COST_INTERVAL_SECS: u64 = 3600;

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
