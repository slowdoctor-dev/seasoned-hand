//! Project entity + store (V006 `projects` table).
//!
//! refs: /specs/phase-2/architecture.md §2.1, §3 V006
//! refs: /specs/phase-2/stories/story-2.2.md

use rusqlite::params;
use thiserror::Error;
use uuid::Uuid;

use crate::db::DbPool;
use crate::time::now_micros;

// Canonical home for the wire shape is `seasoned-hand-dto` (ADR-016 / story 6.3)
// so the backend and the wasm UI share one definition. The DB-string mapping
// (`as_db_str` / `from_db_str`) lives there too; `from_db_str` returns a
// dto-level error that `From` lifts into `ProjectError` below.
pub use seasoned_hand_dto::{Project, ProjectStatus};

impl From<seasoned_hand_dto::EnumParseError> for ProjectError {
    fn from(e: seasoned_hand_dto::EnumParseError) -> Self {
        ProjectError::UnknownStatus(e.value)
    }
}

#[derive(Debug, Clone)]
pub struct NewProject {
    pub tenant_id: Option<String>,
    pub title: String,
    pub description: Option<String>,
}

/// Field-level patch payload. Each `Some(_)` is applied; `None` leaves the
/// existing column unchanged. Status flips go through
/// [`ProjectStore::set_status`] instead so callers can't accidentally
/// archive on rename.
#[derive(Debug, Clone, Default)]
pub struct ProjectPatch {
    pub title: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Error)]
pub enum ProjectError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("project not found: {0}")]
    NotFound(String),
    #[error("unknown project status in DB: {0}")]
    UnknownStatus(String),
}

#[derive(Clone)]
pub struct ProjectStore {
    pool: DbPool,
}

impl ProjectStore {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    pub async fn insert(&self, new: NewProject) -> Result<String, ProjectError> {
        let id = Uuid::new_v4().to_string();
        let now = now_micros();
        let tenant_id = new
            .tenant_id
            .unwrap_or_else(|| "legacy-default".to_string());
        let id_clone = id.clone();
        let res: Result<usize, rusqlite::Error> = self
            .pool
            .with_conn(move |conn| {
                conn.execute(
                    "INSERT INTO projects (\
                       id, tenant_id, title, description, status, created_at, updated_at\
                     ) VALUES (?, ?, ?, ?, ?, ?, ?)",
                    params![
                        id_clone,
                        tenant_id,
                        new.title,
                        new.description,
                        ProjectStatus::Active.as_db_str(),
                        now,
                        now,
                    ],
                )
            })
            .await;
        res?;
        Ok(id)
    }

    pub async fn get(&self, id: &str) -> Result<Project, ProjectError> {
        let id_owned = id.to_string();
        let row: Option<RawRow> = self
            .pool
            .with_conn(move |conn| -> rusqlite::Result<Option<RawRow>> {
                let mut stmt = conn.prepare(
                    "SELECT id, tenant_id, title, description, status, created_at, updated_at \
                       FROM projects WHERE id = ?",
                )?;
                let mut rows = stmt.query_map(params![id_owned], RawRow::from_row)?;
                match rows.next() {
                    Some(row) => Ok(Some(row?)),
                    None => Ok(None),
                }
            })
            .await?;
        match row {
            Some(r) => r.into_project(),
            None => Err(ProjectError::NotFound(id.to_string())),
        }
    }

    /// Newest-first paginated list. `cursor` is the `created_at` of the
    /// last seen row (exclusive upper bound for the next page); pass
    /// `None` for the first page. Optional `status` filter.
    pub async fn list(
        &self,
        status: Option<ProjectStatus>,
        cursor: Option<i64>,
        limit: usize,
    ) -> Result<Vec<Project>, ProjectError> {
        let limit = limit.clamp(1, 200) as i64;
        let status_str = status.map(|s| s.as_db_str().to_string());
        let raws: Vec<RawRow> = self
            .pool
            .with_conn(move |conn| -> rusqlite::Result<Vec<RawRow>> {
                let rows = match (status_str.as_deref(), cursor) {
                    (Some(s), Some(c)) => {
                        let mut stmt = conn.prepare(
                            "SELECT id, tenant_id, title, description, status, created_at, updated_at \
                               FROM projects \
                              WHERE status = ? AND created_at < ? \
                              ORDER BY created_at DESC LIMIT ?",
                        )?;
                        stmt.query_map(params![s, c, limit], RawRow::from_row)?
                            .collect::<rusqlite::Result<Vec<_>>>()?
                    }
                    (Some(s), None) => {
                        let mut stmt = conn.prepare(
                            "SELECT id, tenant_id, title, description, status, created_at, updated_at \
                               FROM projects \
                              WHERE status = ? \
                              ORDER BY created_at DESC LIMIT ?",
                        )?;
                        stmt.query_map(params![s, limit], RawRow::from_row)?
                            .collect::<rusqlite::Result<Vec<_>>>()?
                    }
                    (None, Some(c)) => {
                        let mut stmt = conn.prepare(
                            "SELECT id, tenant_id, title, description, status, created_at, updated_at \
                               FROM projects \
                              WHERE created_at < ? \
                              ORDER BY created_at DESC LIMIT ?",
                        )?;
                        stmt.query_map(params![c, limit], RawRow::from_row)?
                            .collect::<rusqlite::Result<Vec<_>>>()?
                    }
                    (None, None) => {
                        let mut stmt = conn.prepare(
                            "SELECT id, tenant_id, title, description, status, created_at, updated_at \
                               FROM projects \
                              ORDER BY created_at DESC LIMIT ?",
                        )?;
                        stmt.query_map(params![limit], RawRow::from_row)?
                            .collect::<rusqlite::Result<Vec<_>>>()?
                    }
                };
                Ok(rows)
            })
            .await?;
        raws.into_iter().map(RawRow::into_project).collect()
    }

    /// Tenant-scoped variant of [`Self::list`]. Phase 5 list endpoints
    /// must enforce `tenant_id = :ctx.tenant_id`.
    pub async fn list_by_tenant(
        &self,
        tenant_id: &str,
        status: Option<ProjectStatus>,
        cursor: Option<i64>,
        limit: usize,
    ) -> Result<Vec<Project>, ProjectError> {
        let tenant_id = tenant_id.to_string();
        let limit = limit.clamp(1, 200) as i64;
        let status_str = status.map(|s| s.as_db_str().to_string());
        let raws: Vec<RawRow> = self
            .pool
            .with_conn(move |conn| -> rusqlite::Result<Vec<RawRow>> {
                let rows = match (status_str.as_deref(), cursor) {
                    (Some(s), Some(c)) => {
                        let mut stmt = conn.prepare(
                            "SELECT id, tenant_id, title, description, status, created_at, updated_at \
                               FROM projects \
                              WHERE tenant_id = ? AND status = ? AND created_at < ? \
                              ORDER BY created_at DESC LIMIT ?",
                        )?;
                        stmt.query_map(params![tenant_id, s, c, limit], RawRow::from_row)?
                            .collect::<rusqlite::Result<Vec<_>>>()?
                    }
                    (Some(s), None) => {
                        let mut stmt = conn.prepare(
                            "SELECT id, tenant_id, title, description, status, created_at, updated_at \
                               FROM projects \
                              WHERE tenant_id = ? AND status = ? \
                              ORDER BY created_at DESC LIMIT ?",
                        )?;
                        stmt.query_map(params![tenant_id, s, limit], RawRow::from_row)?
                            .collect::<rusqlite::Result<Vec<_>>>()?
                    }
                    (None, Some(c)) => {
                        let mut stmt = conn.prepare(
                            "SELECT id, tenant_id, title, description, status, created_at, updated_at \
                               FROM projects \
                              WHERE tenant_id = ? AND created_at < ? \
                              ORDER BY created_at DESC LIMIT ?",
                        )?;
                        stmt.query_map(params![tenant_id, c, limit], RawRow::from_row)?
                            .collect::<rusqlite::Result<Vec<_>>>()?
                    }
                    (None, None) => {
                        let mut stmt = conn.prepare(
                            "SELECT id, tenant_id, title, description, status, created_at, updated_at \
                               FROM projects \
                              WHERE tenant_id = ? \
                              ORDER BY created_at DESC LIMIT ?",
                        )?;
                        stmt.query_map(params![tenant_id, limit], RawRow::from_row)?
                            .collect::<rusqlite::Result<Vec<_>>>()?
                    }
                };
                Ok(rows)
            })
            .await?;
        raws.into_iter().map(RawRow::into_project).collect()
    }

    pub async fn patch(&self, id: &str, patch: ProjectPatch) -> Result<(), ProjectError> {
        if patch.title.is_none() && patch.description.is_none() {
            // Nothing to update; still confirm the row exists so callers
            // get a consistent NotFound signal.
            self.get(id).await?;
            return Ok(());
        }
        let id_owned = id.to_string();
        let now = now_micros();
        let affected: usize = self
            .pool
            .with_conn(move |conn| -> rusqlite::Result<usize> {
                match (patch.title.as_deref(), patch.description.as_deref()) {
                    (Some(title), Some(desc)) => conn.execute(
                        "UPDATE projects SET title = ?, description = ?, updated_at = ? \
                         WHERE id = ?",
                        params![title, desc, now, id_owned],
                    ),
                    (Some(title), None) => conn.execute(
                        "UPDATE projects SET title = ?, updated_at = ? WHERE id = ?",
                        params![title, now, id_owned],
                    ),
                    (None, Some(desc)) => conn.execute(
                        "UPDATE projects SET description = ?, updated_at = ? WHERE id = ?",
                        params![desc, now, id_owned],
                    ),
                    (None, None) => Ok(0),
                }
            })
            .await?;
        if affected == 0 {
            return Err(ProjectError::NotFound(id.to_string()));
        }
        Ok(())
    }

    /// Resolve the "Inbox" fallback project for `tenant_id`, creating
    /// it on first use. Backs the IntakeRouter's default-project path
    /// (story 2.5) when an incoming brief carries no explicit
    /// `metadata.project_id`. The lookup matches the canonical
    /// `"Inbox"` title scoped to `tenant_id`; on miss we INSERT with
    /// `status = active`. Two concurrent calls on the same tenant may
    /// race and create two Inbox rows — Phase 2 is single-operator so
    /// this is informational, and Phase 5 multi-tenant will add a
    /// UNIQUE(tenant_id, title) constraint when it matters.
    pub async fn find_or_create_inbox(
        &self,
        tenant_id: Option<&str>,
    ) -> Result<String, ProjectError> {
        let tid_owned = tenant_id.map(str::to_string);
        let existing: Option<String> = self
            .pool
            .with_conn({
                let tid = tid_owned.clone();
                move |conn| -> rusqlite::Result<Option<String>> {
                    let mut rows = match tid.as_deref() {
                        Some(t) => {
                            let mut stmt = conn.prepare(
                                "SELECT id FROM projects \
                                  WHERE tenant_id = ? AND title = 'Inbox' \
                                  ORDER BY created_at ASC LIMIT 1",
                            )?;
                            stmt.query_map(params![t], |row| row.get::<_, String>(0))?
                                .collect::<rusqlite::Result<Vec<_>>>()?
                        }
                        None => {
                            let mut stmt = conn.prepare(
                                "SELECT id FROM projects \
                                  WHERE tenant_id IS NULL AND title = 'Inbox' \
                                  ORDER BY created_at ASC LIMIT 1",
                            )?;
                            stmt.query_map([], |row| row.get::<_, String>(0))?
                                .collect::<rusqlite::Result<Vec<_>>>()?
                        }
                    };
                    Ok(rows.pop())
                }
            })
            .await?;
        if let Some(id) = existing {
            return Ok(id);
        }
        self.insert(NewProject {
            tenant_id: tid_owned,
            title: "Inbox".into(),
            description: Some("Default fallback for briefs without a project".into()),
        })
        .await
    }

    pub async fn set_status(&self, id: &str, status: ProjectStatus) -> Result<(), ProjectError> {
        let id_owned = id.to_string();
        let now = now_micros();
        let status_str = status.as_db_str().to_string();
        let affected: usize = self
            .pool
            .with_conn(move |conn| {
                conn.execute(
                    "UPDATE projects SET status = ?, updated_at = ? WHERE id = ?",
                    params![status_str, now, id_owned],
                )
            })
            .await?;
        if affected == 0 {
            return Err(ProjectError::NotFound(id.to_string()));
        }
        Ok(())
    }
}

struct RawRow {
    id: String,
    tenant_id: Option<String>,
    title: String,
    description: Option<String>,
    status: String,
    created_at: i64,
    updated_at: i64,
}

impl RawRow {
    fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            tenant_id: row.get(1)?,
            title: row.get(2)?,
            description: row.get(3)?,
            status: row.get(4)?,
            created_at: row.get(5)?,
            updated_at: row.get(6)?,
        })
    }

    fn into_project(self) -> Result<Project, ProjectError> {
        let status = ProjectStatus::from_db_str(&self.status)?;
        Ok(Project {
            id: self.id,
            tenant_id: self.tenant_id,
            title: self.title,
            description: self.description,
            status,
            created_at: self.created_at,
            updated_at: self.updated_at,
        })
    }
}
