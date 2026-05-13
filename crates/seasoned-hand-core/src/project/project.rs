//! Project entity + store (V006 `projects` table).
//!
//! refs: /specs/phase-2/architecture.md §2.1, §3 V006
//! refs: /specs/phase-2/stories/story-2.2.md

use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::params;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::db::DbPool;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProjectStatus {
    Active,
    Archived,
}

impl ProjectStatus {
    pub fn as_db_str(&self) -> &'static str {
        match self {
            ProjectStatus::Active => "active",
            ProjectStatus::Archived => "archived",
        }
    }

    pub fn from_db_str(s: &str) -> Result<Self, ProjectError> {
        match s {
            "active" => Ok(ProjectStatus::Active),
            "archived" => Ok(ProjectStatus::Archived),
            other => Err(ProjectError::UnknownStatus(other.to_string())),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub tenant_id: Option<String>,
    pub title: String,
    pub description: Option<String>,
    pub status: ProjectStatus,
    pub created_at: i64,
    pub updated_at: i64,
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
                        new.tenant_id,
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

fn now_micros() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_micros() as i64)
        .unwrap_or(0)
}
