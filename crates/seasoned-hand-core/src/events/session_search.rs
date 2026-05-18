use rusqlite::params;
use sha2::{Digest, Sha256};

use super::{Event, EventType};
use crate::events::{EventStore, NewEvent};
use crate::llm::{ChatCompletionRequest, LlmClient, Message, Role};
use crate::router::{SlotName, SlotRouter};

#[derive(Debug, Clone, Default)]
pub struct SessionSearchQuery {
    pub session_id: Option<String>,
    pub event_type: Option<EventType>,
    pub source: Option<String>,
    pub from_timestamp: Option<i64>,
    pub to_timestamp: Option<i64>,
    pub limit: Option<usize>,
}

impl SessionSearchQuery {
    pub fn effective_limit(&self) -> usize {
        self.limit.unwrap_or(20).min(100)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct EventHit {
    pub event_id: i64,
    pub session_id: String,
    pub timestamp: i64,
    pub event_type: String,
    pub source: String,
    pub snippet: String,
}

#[derive(Debug, Clone)]
pub struct SearchSummary {
    pub summary: String,
    pub degraded: bool,
}

pub fn index_event_for_search(conn: &rusqlite::Connection, event: &Event) -> rusqlite::Result<()> {
    let searchable_text = searchable_text_for_event(event);
    conn.execute(
        "INSERT INTO session_search_index (event_id, session_id, timestamp, event_type, source, searchable_text)
         VALUES (?, ?, ?, ?, ?, ?)",
        params![
            event.id,
            event.session_id,
            event.timestamp,
            event.event_type.as_str(),
            event.source,
            searchable_text
        ],
    )?;
    Ok(())
}

pub fn search_session_events(
    conn: &rusqlite::Connection,
    query: &str,
    filters: &SessionSearchQuery,
) -> rusqlite::Result<Vec<EventHit>> {
    let mut sql = String::from(
        "SELECT i.event_id, i.session_id, i.timestamp, i.event_type, i.source,
                snippet(session_search_fts, 0, '[', ']', ' … ', 16) AS snippet
         FROM session_search_fts f
         JOIN session_search_index i ON i.event_id = f.rowid
         WHERE session_search_fts MATCH ?",
    );

    let mut binds: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(query.to_string())];
    if let Some(session_id) = &filters.session_id {
        sql.push_str(" AND i.session_id = ?");
        binds.push(Box::new(session_id.clone()));
    }
    if let Some(event_type) = filters.event_type {
        sql.push_str(" AND i.event_type = ?");
        binds.push(Box::new(event_type.as_str().to_string()));
    }
    if let Some(source) = &filters.source {
        sql.push_str(" AND i.source = ?");
        binds.push(Box::new(source.clone()));
    }
    if let Some(from_timestamp) = filters.from_timestamp {
        sql.push_str(" AND i.timestamp >= ?");
        binds.push(Box::new(from_timestamp));
    }
    if let Some(to_timestamp) = filters.to_timestamp {
        sql.push_str(" AND i.timestamp <= ?");
        binds.push(Box::new(to_timestamp));
    }
    sql.push_str(" ORDER BY i.timestamp DESC, i.event_id DESC LIMIT ?");
    binds.push(Box::new(filters.effective_limit() as i64));

    let mut stmt = conn.prepare(&sql)?;
    let bind_refs: Vec<&dyn rusqlite::ToSql> = binds.iter().map(|b| b.as_ref()).collect();
    let rows = stmt.query_map(bind_refs.as_slice(), |row| {
        Ok(EventHit {
            event_id: row.get(0)?,
            session_id: row.get(1)?,
            timestamp: row.get(2)?,
            event_type: row.get(3)?,
            source: row.get(4)?,
            snippet: row.get::<_, Option<String>>(5)?.unwrap_or_default(),
        })
    })?;

    let mut hits = Vec::new();
    for row in rows {
        hits.push(row?);
    }
    Ok(hits)
}

pub async fn summarize_hits_with_fallback<S: EventStore>(
    event_store: &S,
    router: &SlotRouter,
    session_id: &str,
    query: &str,
    hits: &[EventHit],
) -> SearchSummary {
    match summarize_hits(router, query, hits).await {
        Ok(summary) => SearchSummary {
            summary,
            degraded: false,
        },
        Err(reason) => {
            let _ = event_store
                .append(NewEvent {
                    session_id: session_id.to_string(),
                    event_type: EventType::Misc,
                    source: "session_search".to_string(),
                    data: serde_json::json!({
                        "kind": "session_search_summary_degraded",
                        "session_id": session_id,
                        "query_hash": query_hash(query),
                        "reason": reason,
                    }),
                })
                .await;
            SearchSummary {
                summary: fallback_summary(query, hits),
                degraded: true,
            }
        }
    }
}

async fn summarize_hits(
    router: &SlotRouter,
    query: &str,
    hits: &[EventHit],
) -> Result<String, String> {
    let slot = router.resolve(SlotName::SessionSearch);
    let llm = LlmClient::new(slot.base_url.clone(), slot.api_key.clone());
    let context = hits
        .iter()
        .take(12)
        .map(|h| format!("- [{}] {} {}", h.event_type, h.source, h.snippet))
        .collect::<Vec<_>>()
        .join("\n");
    let req = ChatCompletionRequest {
        model: slot.model.clone(),
        messages: vec![
            Message {
                role: Role::System,
                content: Some("Summarize session search hits in 4 bullet points max.".to_string()),
                name: None,
                tool_calls: None,
                tool_call_id: None,
            },
            Message {
                role: Role::User,
                content: Some(format!("query: {query}\n\nhits:\n{context}")),
                name: None,
                tool_calls: None,
                tool_call_id: None,
            },
        ],
        tools: None,
        tool_choice: None,
        temperature: Some(0.0),
        max_tokens: Some(220),
        top_p: None,
    };
    let resp = llm
        .chat_completion(req)
        .await
        .map_err(|e| format!("llm_error:{e}"))?;
    let content = resp
        .choices
        .first()
        .and_then(|c| c.message.content.clone())
        .unwrap_or_default();
    let trimmed = content.trim().to_string();
    if trimmed.is_empty() {
        return Err("empty_summary".to_string());
    }
    Ok(trimmed)
}

fn fallback_summary(query: &str, hits: &[EventHit]) -> String {
    let total = hits.len();
    let head = hits
        .iter()
        .take(5)
        .map(|h| format!("- {}:{} {}", h.event_type, h.source, h.snippet))
        .collect::<Vec<_>>()
        .join("\n");
    format!("Summary unavailable for query '{query}'. {total} raw hits returned.\n{head}")
}

fn query_hash(query: &str) -> String {
    format!("{:x}", Sha256::digest(query.as_bytes()))
}

fn searchable_text_for_event(event: &Event) -> String {
    match event.event_type {
        EventType::Message => {
            let role = field_string(&event.data, &["role"]);
            let text = field_string(&event.data, &["text", "content", "body"]);
            join_parts(&[role, text, flatten_json_values(&event.data)])
        }
        EventType::Action => {
            let tool_name = field_string(&event.data, &["tool_name", "name", "tool"]);
            let tool_input = field_value(&event.data, &["tool_input", "input", "args"])
                .map(flatten_json_values)
                .unwrap_or_default();
            join_parts(&[tool_name, tool_input])
        }
        EventType::Observation => {
            let tool_name = field_string(&event.data, &["tool_name", "name", "tool"]);
            let tool_result = field_value(&event.data, &["tool_result", "result", "output"])
                .map(flatten_json_values)
                .unwrap_or_else(|| flatten_json_values(&event.data));
            let truncated = truncate_chars(tool_result, 4096);
            join_parts(&[tool_name, truncated])
        }
        EventType::Plan => {
            let goal = field_string(&event.data, &["goal"]);
            let phase_titles = event
                .data
                .get("phases")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.get("title").and_then(|t| t.as_str()))
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .unwrap_or_default();
            join_parts(&[goal, phase_titles, flatten_json_values(&event.data)])
        }
        EventType::Skill => {
            let kind = field_string(&event.data, &["kind"]);
            let playbook_id = field_string(&event.data, &["playbook_id"]);
            let matcher_mode = field_string(&event.data, &["matcher_mode"]);
            join_parts(&[
                kind,
                playbook_id,
                matcher_mode,
                flatten_json_values(&event.data),
            ])
        }
        EventType::Misc => {
            let kind = field_string(&event.data, &["kind"]);
            let reason = field_string(&event.data, &["reason", "error", "message"]);
            let category = field_string(&event.data, &["category"]);
            join_parts(&[kind, reason, category, flatten_json_values(&event.data)])
        }
        EventType::Knowledge | EventType::Datasource => flatten_json_values(&event.data),
    }
}

fn field_string(data: &serde_json::Value, keys: &[&str]) -> String {
    field_value(data, keys)
        .map(flatten_json_values)
        .unwrap_or_default()
}

fn field_value<'a>(data: &'a serde_json::Value, keys: &[&str]) -> Option<&'a serde_json::Value> {
    keys.iter().find_map(|k| data.get(*k))
}

fn flatten_json_values(value: &serde_json::Value) -> String {
    let mut parts = Vec::new();
    collect_json_strings(value, &mut parts);
    join_parts(&parts)
}

fn collect_json_strings(value: &serde_json::Value, out: &mut Vec<String>) {
    match value {
        serde_json::Value::Null => {}
        serde_json::Value::Bool(v) => out.push(v.to_string()),
        serde_json::Value::Number(v) => out.push(v.to_string()),
        serde_json::Value::String(v) => out.push(v.clone()),
        serde_json::Value::Array(items) => {
            for item in items {
                collect_json_strings(item, out);
            }
        }
        serde_json::Value::Object(map) => {
            // REVIEW iter-3 A5: index VALUES only, not object KEYS. Indexing keys
            // makes operator searches for common field names like "kind" or
            // "playbook_id" match every Skill / Misc event regardless of relevance,
            // which isn't part of the architecture §3 per-EventType shape table.
            for (_k, v) in map {
                collect_json_strings(v, out);
            }
        }
    }
}

fn join_parts(parts: &[String]) -> String {
    parts
        .iter()
        .filter_map(|p| {
            let trimmed = p.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn truncate_chars(input: String, cap: usize) -> String {
    input.chars().take(cap).collect()
}
