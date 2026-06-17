//! Tenant-safe event projection (Phase 5 story 5.14).
//!
//! V013 created `tenant_event_view` — a redacted, tenant-tagged projection
//! of the canonical `events` table. This module owns the write-time hook
//! that runs on every `SqliteEventStore::append`, redacts the payload
//! via [`crate::verifier::extraction::redact_pii`], resolves the
//! `tenant_id` from the session's task/project chain, and inserts the
//! projection row.
//!
//! Quarantine semantics (architecture §7): if any projection step fails
//! (tenant resolution, serialization, insert), we skip the projection
//! row but the canonical `events.append` still succeeds. A separate
//! `Misc{kind:"tenant_event_projection_failed"}` event is queued for
//! post-commit emission so operators can see the gap in the projection
//! without losing the original event.
//!
//! refs: /specs/phase-5/architecture.md §7 (tenant projection), §7.1
//!       (write-time redaction)
//! refs: /specs/phase-5/stories/story-5.14.md
//! closes: SECURITY_REVIEW DEBT #S-1

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::audit::{AuditAction, AuditLogger, AuditRecord, AuditWriteError};
use crate::auth::{Action, AuthContext, AuthError, AuthResource, Role, authorize};
use crate::db::DbPool;
use crate::events::Event;
use crate::events::session_search::searchable_text_for_event;
use crate::verifier::extraction::redact_pii;

/// Sentinel tenant for events tied to sessions with no derivable
/// tenant chain. Matches the V013 bootstrap so projection rows never
/// vanish silently.
pub const SENTINEL_TENANT: &str = "legacy-default";

/// Default visibility for events whose `event_type + source` do not
/// match a more restrictive rule. Aligns with architecture §7's RBAC:
/// users see their own org's 'user' + 'viewer' rows; viewers see
/// 'viewer' only; admins see all.
pub const DEFAULT_VISIBILITY: &str = "user";

/// Source prefix on Misc events synthesized BY the projection hook
/// itself (post-commit quarantine emissions). The hook skips events
/// whose source starts with this prefix to avoid infinite recursion.
pub const PROJECTION_INTERNAL_SOURCE: &str = "tenant_event_projection";

/// The projection values `apply` materialized for an event, carried out
/// on [`ProjectionOutcome::Inserted`] so the search-index hook can reuse
/// them instead of re-`SELECT`ing them back from `tenant_event_view` on
/// the per-event append hot path (perf review iter-3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchProjection {
    pub tenant_id: String,
    pub visibility_level: &'static str,
    /// The redacted, search-indexable text (same value written to
    /// `tenant_event_view.searchable_text`).
    pub searchable_text: String,
}

/// Outcome of one projection attempt — surfaced from `apply` so the
/// caller can decide whether to queue a quarantine Misc event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectionOutcome {
    /// Row inserted into tenant_event_view; carries the materialized
    /// projection so the caller can index it for search without a re-read.
    Inserted(SearchProjection),
    /// Projection intentionally skipped (e.g. an event synthesized by
    /// the projection hook itself — see [`PROJECTION_INTERNAL_SOURCE`]).
    /// Not an error condition.
    Skipped,
    /// Projection step failed. Caller should emit a
    /// `Misc{kind:"tenant_event_projection_failed"}` event so the gap
    /// is visible. Carries a short reason string for the Misc payload.
    Failed { reason: String },
}

/// Write the tenant_event_view projection for `event` using the same
/// `conn` that just inserted the canonical row. Runs inside the caller's
/// transaction so the projection lands atomically with the event.
///
/// Returns the outcome; on `Failed`, the caller is responsible for
/// queuing a quarantine Misc event AFTER the transaction commits (the
/// hook cannot recurse into [`crate::events::EventStore::append`] from
/// inside the same connection).
pub fn apply(conn: &Connection, event: &Event) -> ProjectionOutcome {
    if event.source.starts_with(PROJECTION_INTERNAL_SOURCE) {
        return ProjectionOutcome::Skipped;
    }

    let tenant_id = match resolve_tenant_id(conn, &event.session_id) {
        Ok(t) => t,
        Err(e) => {
            return ProjectionOutcome::Failed {
                reason: format!("tenant_resolution: {e}"),
            };
        }
    };

    let visibility_level = visibility_for(event);
    let redacted_data = redact_event_data(event);
    let searchable_text = searchable_text_for_event(event);
    // `searchable_text` is built from the canonical (non-redacted) event
    // payload. We re-run the redactor on it so the FTS-style searchable
    // column never indexes a value that's stripped from `redacted_data`.
    let (searchable_text, _) = redact_pii(&searchable_text);

    let insert = conn.execute(
        "INSERT INTO tenant_event_view
           (event_id, tenant_id, visibility_level, redacted_data,
            searchable_text, created_at)
         VALUES (?, ?, ?, ?, ?, ?)",
        params![
            event.id,
            tenant_id,
            visibility_level,
            redacted_data,
            searchable_text,
            event.timestamp,
        ],
    );
    match insert {
        // `params!` only borrows the values above, so they are still owned
        // here — hand them to the caller for the search index hook instead of
        // forcing a second read of the row we just wrote.
        Ok(_) => ProjectionOutcome::Inserted(SearchProjection {
            tenant_id,
            visibility_level,
            searchable_text,
        }),
        Err(e) => ProjectionOutcome::Failed {
            reason: format!("insert: {e}"),
        },
    }
}

/// Resolve the tenant for a session by walking session → task →
/// project. Returns the sentinel when none of the joins yield a row,
/// matching the V013 bootstrap so legacy sessions still project.
///
/// RESIDUAL RISK (issue #22, accepted): the `events` table itself carries no
/// `tenant_id` column. Tenancy is *derived* here through the session → task →
/// project join chain, and any session that resolves to no parent falls back to
/// the shared [`SENTINEL_TENANT`] (`legacy-default`). So two genuinely distinct
/// pre-Phase-5 tenants both bootstrapped under the sentinel are indistinguishable
/// in this projection.
///
/// This is acceptable today because (a) every post-Phase-5 row has a real parent
/// tenant, and (b) `tenant_event_view.tenant_id` is materialized at write time
/// (this fn runs once at append, not on read), so reads are not exposed to a
/// moving join result even if parent rows are later deleted. Eliminating the
/// residual fully would mean adding a populated `events.tenant_id` column with a
/// backfill migration over the append-only table — deferred until a concrete
/// multi-legacy-tenant deployment needs it.
///
/// Parent-mismatch note (issue #22 batch B review): this projection resolves a
/// session **task-first** (`COALESCE(t.tenant_id, p.tenant_id)`, project taken
/// from the task), so for a corrupt session whose `project_id` and `task_id`
/// point at *different* tenants it deterministically assigns the task's tenant
/// and exposes only the **redacted** projection to that single tenant. The
/// **raw** read guards (`server::require_session_tenant` / `list_sessions` /
/// `require_verification_tenant`) are stricter: they fail closed and make such a
/// session invisible to *every* tenant. So there is no cross-tenant *raw* leak;
/// the residual is only that the deterministic redacted projection is visible to
/// the task's tenant. Well-formed sessions (task and own-project agree, or one is
/// null) are unaffected.
fn resolve_tenant_id(conn: &Connection, session_id: &str) -> rusqlite::Result<String> {
    // The join chain mirrors `billing::user_cost::flush`'s aggregation
    // CTE: prefer the task's tenant (post-V014 NOT NULL) and fall back
    // to the project's tenant before the sentinel.
    let found: Option<Option<String>> = conn
        .query_row(
            "SELECT COALESCE(t.tenant_id, p.tenant_id)
             FROM sessions s
             LEFT JOIN tasks t ON t.id = s.task_id
             LEFT JOIN projects p ON p.id = COALESCE(t.project_id, s.project_id)
             WHERE s.id = ?",
            [session_id],
            |r| r.get::<_, Option<String>>(0),
        )
        .optional()?;
    Ok(found
        .flatten()
        .unwrap_or_else(|| SENTINEL_TENANT.to_string()))
}

/// Map an event's `(event_type, source)` to a `visibility_level` value
/// from the schema's CHECK list (`viewer|user|admin`). Conservative
/// defaults today; stories 5.15/5.16 layer richer RBAC predicates on
/// top.
fn visibility_for(event: &Event) -> &'static str {
    // Audit-emitted Misc events surface dual-write payloads — they
    // describe org-wide actions and must stay admin-restricted until
    // an authorized actor pulls them via the audit API.
    if event.source == "audit" {
        return "admin";
    }
    DEFAULT_VISIBILITY
}

/// Serialize the event data with PII patterns redacted. Falls back to
/// the redacted stringification when re-parsing the redacted text as
/// JSON fails (the redaction replacements never produce invalid JSON
/// in practice, but the column is TEXT so storing the raw redacted
/// string is correct either way).
fn redact_event_data(event: &Event) -> String {
    let raw = match serde_json::to_string(&event.data) {
        Ok(s) => s,
        Err(_) => return "{}".to_string(),
    };
    let (redacted, _) = redact_pii(&raw);
    redacted
}

/// Map an `AuthContext`'s effective role to the set of visibility_level
/// values it may read from `tenant_event_view`. Mirrors story 5.15's
/// `session_search::allowed_visibility_levels_for_role` so the read
/// surface and the search surface stay in lockstep.
pub fn allowed_visibility_levels(role: Role) -> &'static [&'static str] {
    match role {
        Role::Admin => &["viewer", "user", "admin"],
        Role::User => &["viewer", "user"],
        Role::Viewer => &["viewer"],
    }
}

/// One row of the tenant-visible event feed returned by [`query`].
///
/// `redacted_data` is the post-PII-scrub JSON string stored in
/// `tenant_event_view.redacted_data`; never the raw `events.data`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VisibleEventRow {
    pub event_id: i64,
    pub session_id: String,
    pub timestamp: i64,
    pub event_type: String,
    pub source: String,
    pub visibility_level: String,
    pub redacted_data: String,
}

/// One row of the admin raw-event read returned by [`query_raw`].
///
/// `data` carries the canonical `events.data` payload with NO redaction
/// applied — surfacing it requires `Action::EventRawRead` + an audit_log
/// record per-call.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RawEventRow {
    pub event_id: i64,
    pub session_id: String,
    pub timestamp: i64,
    pub event_type: String,
    pub source: String,
    pub data: String,
}

#[derive(Debug, Clone, Default)]
pub struct EventReadQuery {
    /// Strictly-greater cursor over `event_id` for monotonic pagination.
    pub after_event_id: Option<i64>,
    /// Page size; clamped to [1, 500] (default 100) at execution time.
    pub limit: Option<usize>,
}

#[derive(Debug, Error)]
pub enum VisibilityQueryError {
    #[error("auth: {0}")]
    Auth(#[from] AuthError),
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("audit: {0}")]
    Audit(#[from] AuditWriteError),
}

/// Read tenant-visible event rows for a session. Applies the compound
/// `(tenant_id, visibility_level)` predicate from architecture §7 so
/// callers cannot read across tenant boundaries or above their role's
/// visibility scope. Uses the redacted projection (NOT raw `events.data`),
/// so PII patterns scrubbed at write time stay scrubbed at read time.
///
/// `Action`-level gating is intentionally absent here — the tenant +
/// visibility predicates ARE the gate. Callers needing a hard admin-only
/// surface (e.g. forensics) must use [`query_raw`] instead.
pub async fn query(
    db: &DbPool,
    auth: &AuthContext,
    session_id: &str,
    q: EventReadQuery,
) -> Result<Vec<VisibleEventRow>, VisibilityQueryError> {
    let tenant = auth.tenant_id.clone();
    // Story 5.16 + hardening P5-HARD-IT1-H1: gate on the EFFECTIVE role
    // (project override takes precedence over org role), matching every
    // other RBAC gate in the codebase. Using raw org_role here would let
    // a project-downgraded admin still read admin-visibility rows.
    let allowed = allowed_visibility_levels(auth.effective_role());
    let allowed_owned: Vec<String> = allowed.iter().map(|s| (*s).to_string()).collect();
    let session_id = session_id.to_string();
    let after = q.after_event_id;
    let limit = q.limit.unwrap_or(100).clamp(1, 500);

    let rows = db
        .with_conn(move |conn| {
            // Build the IN-list dynamically — the allowed set is small
            // (1..=3 entries) so inline `?` placeholders are simpler
            // than rarray. Order by event_id so cursors are monotonic.
            let placeholders = std::iter::repeat_n("?", allowed_owned.len())
                .collect::<Vec<_>>()
                .join(", ");
            let mut sql = format!(
                "SELECT t.event_id, e.session_id, e.timestamp, e.type, e.source,
                        t.visibility_level, t.redacted_data
                 FROM tenant_event_view t
                 JOIN events e ON e.id = t.event_id
                 WHERE t.tenant_id = ?
                   AND t.visibility_level IN ({placeholders})
                   AND e.session_id = ?"
            );
            let mut params_vec: Vec<rusqlite::types::Value> = vec![tenant.into()];
            for level in &allowed_owned {
                params_vec.push(level.clone().into());
            }
            params_vec.push(session_id.into());
            if let Some(after_id) = after {
                sql.push_str(" AND t.event_id > ?");
                params_vec.push(after_id.into());
            }
            sql.push_str(" ORDER BY t.event_id ASC LIMIT ?");
            params_vec.push((limit as i64).into());

            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt
                .query_map(rusqlite::params_from_iter(params_vec.iter()), |r| {
                    Ok(VisibleEventRow {
                        event_id: r.get(0)?,
                        session_id: r.get(1)?,
                        timestamp: r.get(2)?,
                        event_type: r.get(3)?,
                        source: r.get(4)?,
                        visibility_level: r.get(5)?,
                        redacted_data: r.get(6)?,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok::<Vec<VisibleEventRow>, rusqlite::Error>(rows)
        })
        .await?;
    Ok(rows)
}

/// Admin-only forensic read of raw `events.data` for a session. Gated
/// by `Action::EventRawRead` (admin role only; user + viewer denied at
/// the policy gate). Every successful read writes one `audit_log` row
/// via [`AuditLogger`] so the access is non-repudiable.
///
/// Returns the rows the caller is now authorized to see — the audit
/// row carries the count + session_id so operators can detect
/// suspicious large reads even though the data itself isn't logged.
pub async fn query_raw(
    db: &DbPool,
    auth: &AuthContext,
    audit: &AuditLogger,
    session_id: &str,
    q: EventReadQuery,
) -> Result<Vec<RawEventRow>, VisibilityQueryError> {
    authorize(
        Action::EventRawRead,
        &AuthResource {
            is_same_org: true,
            actor_can_share: true,
        },
        auth,
    )?;

    let tenant = auth.tenant_id.clone();
    let session_id_for_query = session_id.to_string();
    let after = q.after_event_id;
    let limit = q.limit.unwrap_or(100).clamp(1, 500);

    let rows = db
        .with_conn(move |conn| {
            // Resolve the session's tenant via the same chain the
            // projection hook uses. Cross-tenant reads are denied here
            // so an admin in tenant A can't pull raw rows from tenant B
            // even with `Action::EventRawRead`.
            let session_tenant: String = resolve_tenant_id(conn, &session_id_for_query)?;
            if session_tenant != tenant {
                return Ok::<Vec<RawEventRow>, rusqlite::Error>(Vec::new());
            }
            let mut sql = String::from(
                "SELECT id, session_id, timestamp, type, source, data
                 FROM events
                 WHERE session_id = ?",
            );
            let mut params_vec: Vec<rusqlite::types::Value> = vec![session_id_for_query.into()];
            if let Some(after_id) = after {
                sql.push_str(" AND id > ?");
                params_vec.push(after_id.into());
            }
            sql.push_str(" ORDER BY id ASC LIMIT ?");
            params_vec.push((limit as i64).into());

            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt
                .query_map(rusqlite::params_from_iter(params_vec.iter()), |r| {
                    Ok(RawEventRow {
                        event_id: r.get(0)?,
                        session_id: r.get(1)?,
                        timestamp: r.get(2)?,
                        event_type: r.get(3)?,
                        source: r.get(4)?,
                        data: r.get(5)?,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
        .await?;

    // Emit one audit_log row per raw-read call. The payload carries
    // session_id + row count + cursor; never the actual event data, so
    // the audit log itself doesn't leak what was just read raw.
    audit
        .record(
            auth,
            AuditRecord {
                action: AuditAction::EventRawRead,
                resource_type: "session",
                resource_id: session_id,
                target_user_id: None,
                decision: Some("allow"),
                reason: None,
                metadata: serde_json::json!({
                    "after_event_id": q.after_event_id,
                    "limit": q.limit,
                    "rows_returned": rows.len(),
                }),
            },
        )
        .await?;
    Ok(rows)
}

#[cfg(test)]
mod tests;
