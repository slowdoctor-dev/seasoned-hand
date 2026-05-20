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

/// Outcome of one projection attempt — surfaced from `apply` so the
/// caller can decide whether to queue a quarantine Misc event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectionOutcome {
    /// Row inserted into tenant_event_view.
    Inserted,
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
        Ok(_) => ProjectionOutcome::Inserted,
        Err(e) => ProjectionOutcome::Failed {
            reason: format!("insert: {e}"),
        },
    }
}

/// Resolve the tenant for a session by walking session → task →
/// project. Returns the sentinel when none of the joins yield a row,
/// matching the V013 bootstrap so legacy sessions still project.
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

#[cfg(test)]
mod tests;
