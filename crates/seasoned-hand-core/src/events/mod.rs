//! Append-only event stream.
//! refs: /specs/phase-0/architecture.md §3.2, §3.4
//! refs: /specs/00-philosophy/PRINCIPLES.md #3

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::db::DbError;
use crate::sandbox::SandboxError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EventType {
    Message,
    Action,
    Observation,
    Plan,
    Knowledge,
    Datasource,
    Skill,
    Misc,
}

impl EventType {
    pub fn as_str(self) -> &'static str {
        match self {
            EventType::Message => "Message",
            EventType::Action => "Action",
            EventType::Observation => "Observation",
            EventType::Plan => "Plan",
            EventType::Knowledge => "Knowledge",
            EventType::Datasource => "Datasource",
            EventType::Skill => "Skill",
            EventType::Misc => "Misc",
        }
    }
}

impl fmt::Display for EventType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for EventType {
    type Err = EventError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "Message" => EventType::Message,
            "Action" => EventType::Action,
            "Observation" => EventType::Observation,
            "Plan" => EventType::Plan,
            "Knowledge" => EventType::Knowledge,
            "Datasource" => EventType::Datasource,
            "Skill" => EventType::Skill,
            "Misc" => EventType::Misc,
            other => return Err(EventError::UnknownType(other.to_string())),
        })
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Event {
    pub id: i64,
    pub session_id: String,
    pub timestamp: i64,
    #[serde(rename = "type")]
    pub event_type: EventType,
    pub source: String,
    pub data: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct NewEvent {
    pub session_id: String,
    pub event_type: EventType,
    pub source: String,
    pub data: serde_json::Value,
}

#[derive(Debug, Clone, Default)]
pub struct EventQuery {
    pub after_id: Option<i64>,
    pub event_type: Option<EventType>,
    pub limit: Option<usize>,
}

impl EventQuery {
    pub fn effective_limit(&self) -> usize {
        self.limit.unwrap_or(100).min(1000)
    }
}

#[derive(Debug, Error)]
pub enum EventError {
    #[error("session not found: {0}")]
    SessionNotFound(String),
    #[error("unknown event type: {0}")]
    UnknownType(String),
    #[error("db error: {0}")]
    Db(#[from] DbError),
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("system clock error: {0}")]
    Clock(String),
    #[error("sandbox error: {0}")]
    Sandbox(#[from] SandboxError),
    #[error("invalid file-ref path: {0}")]
    InvalidFileRefPath(String),
}

/// Append-only event store.
///
/// Intentionally has **only** `append` and `query`. Adding any mutating
/// method (`update`, `delete`, etc.) would violate the append-only
/// invariant in PRINCIPLES.md #3 and architecture §3.2.
#[allow(async_fn_in_trait)]
pub trait EventStore: Send + Sync {
    async fn append(&self, draft: NewEvent) -> Result<Event, EventError>;
    async fn query(&self, session_id: &str, filter: EventQuery) -> Result<Vec<Event>, EventError>;
}

pub mod payload;
pub mod sqlite;
pub mod truncation;

#[cfg(test)]
mod tests;
