//! SQLite persistence.
//! refs: /specs/phase-0/architecture.md §3

use std::path::Path;
use std::sync::Arc;

use rusqlite::Connection;
use thiserror::Error;
use tokio::sync::Mutex;

mod migrations {
    refinery::embed_migrations!("../../migrations");
}

#[derive(Debug, Error)]
pub enum DbError {
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("migration error: {0}")]
    Migration(#[from] refinery::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("WAL not enabled: returned {0}")]
    WalNotEnabled(String),
}

#[derive(Clone)]
pub struct DbPool {
    inner: Arc<Mutex<Connection>>,
}

impl DbPool {
    pub async fn with_conn<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut Connection) -> R + Send,
        R: Send,
    {
        let mut conn = self.inner.lock().await;
        f(&mut conn)
    }
}

/// Opens a SQLite connection at the given URL (`sqlite:./path`, a bare
/// path, or `:memory:`), sets WAL mode and foreign keys, then runs the
/// embedded migrations.
pub async fn open(database_url: &str) -> Result<DbPool, DbError> {
    let mut conn = if database_url == ":memory:" || database_url == "sqlite::memory:" {
        Connection::open_in_memory()?
    } else {
        let path = database_url.strip_prefix("sqlite:").unwrap_or(database_url);
        if let Some(parent) = Path::new(path).parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)?;
        }
        Connection::open(path)?
    };

    set_pragmas(&conn, is_in_memory(database_url))?;
    run_migrations(&mut conn)?;

    Ok(DbPool {
        inner: Arc::new(Mutex::new(conn)),
    })
}

fn is_in_memory(url: &str) -> bool {
    url == ":memory:" || url == "sqlite::memory:"
}

fn set_pragmas(conn: &Connection, in_memory: bool) -> Result<(), DbError> {
    let mode: String = conn.query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))?;
    let normalized = mode.to_lowercase();
    if !in_memory && normalized != "wal" {
        return Err(DbError::WalNotEnabled(mode));
    }
    conn.execute_batch("PRAGMA foreign_keys = ON;")?;
    Ok(())
}

pub fn run_migrations(conn: &mut Connection) -> Result<(), DbError> {
    migrations::migrations::runner().run(conn)?;
    Ok(())
}

#[cfg(test)]
mod tests;
