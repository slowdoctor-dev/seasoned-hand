//! Built-in tools.
//! Phase 3 ships real SOP / glossary / playbook lookup surfaces.
//!
//! refs: /specs/phase-0/architecture.md §4.3

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use rusqlite::OptionalExtension;
use serde_json::{Map, Value, json};

use super::{Tool, ToolContext, ToolError, ToolErrorPayload, ToolOutput};
use crate::agent::init::feature_list::{FeatureList, FeatureStatus};
use crate::agent::init::progress;
use crate::events::{EventStore, EventType, NewEvent};
use crate::plan::{Phase, PhaseStatus, PlanMutationSource};

pub(super) async fn sandbox_post_raw(url: &str, body: Value) -> Result<ToolOutput, ToolError> {
    let client = reqwest::Client::new();
    let resp = client
        .post(url)
        .json(&body)
        .send()
        .await
        .map_err(|e| ToolError::Backend(e.to_string()))?;
    let status = resp.status();
    let parsed: Value = resp.json().await.unwrap_or(Value::Null);
    if status.is_success() {
        Ok(ToolOutput {
            ok: true,
            output: parsed,
            file_ref: None,
            error: None,
        })
    } else {
        Ok(ToolOutput {
            ok: false,
            output: parsed,
            file_ref: None,
            error: Some(ToolErrorPayload {
                kind: "sandbox_http".into(),
                message: format!("HTTP {}", status.as_u16()),
            }),
        })
    }
}

async fn sandbox_post(ctx: &ToolContext, path: &str, body: Value) -> Result<ToolOutput, ToolError> {
    let handle = ctx.sandbox.get(&ctx.session_id).await.ok_or_else(|| {
        ToolError::Backend(format!("sandbox not ready for session {}", ctx.session_id))
    })?;
    let url = format!("{}{}", handle.api_url, path);
    sandbox_post_raw(&url, body).await
}

pub(super) async fn sandbox_get_raw(url: &str) -> Result<ToolOutput, ToolError> {
    let client = reqwest::Client::new();
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| ToolError::Backend(e.to_string()))?;
    let status = resp.status();
    let parsed: Value = resp.json().await.unwrap_or(Value::Null);
    if status.is_success() {
        Ok(ToolOutput {
            ok: true,
            output: parsed,
            file_ref: None,
            error: None,
        })
    } else {
        Ok(ToolOutput {
            ok: false,
            output: parsed,
            file_ref: None,
            error: Some(ToolErrorPayload {
                kind: "sandbox_http".into(),
                message: format!("HTTP {}", status.as_u16()),
            }),
        })
    }
}

async fn sandbox_get(ctx: &ToolContext, path: &str) -> Result<ToolOutput, ToolError> {
    let handle = ctx.sandbox.get(&ctx.session_id).await.ok_or_else(|| {
        ToolError::Backend(format!("sandbox not ready for session {}", ctx.session_id))
    })?;
    let url = format!("{}{}", handle.api_url, path);
    sandbox_get_raw(&url).await
}

async fn browser_action(
    ctx: &ToolContext,
    action: &str,
    body: Value,
) -> Result<ToolOutput, ToolError> {
    let mut object = match body {
        Value::Object(map) => map,
        _ => Map::new(),
    };
    object.insert("action_type".into(), Value::String(action.to_string()));
    sandbox_post(ctx, "/v1/browser/actions", Value::Object(object)).await
}

fn require_str(args: &Value, key: &str) -> Result<String, ToolError> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| ToolError::InvalidArgs(format!("missing '{key}' string")))
}

pub struct MessageNotifyUser;

#[async_trait]
impl Tool for MessageNotifyUser {
    fn name(&self) -> &'static str {
        "message_notify_user"
    }
    fn description(&self) -> &'static str {
        "Send an informational message to the user. Fire-and-forget; the agent loop does not wait for a reply."
    }
    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "content": { "type": "string", "description": "Message body shown to the user." },
                "final": { "type": "boolean", "description": "When true, signal task completion and trigger verifier flow." }
            },
            "required": ["content"],
            "additionalProperties": false,
        })
    }
    async fn invoke(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let content = require_str(&args, "content")?;
        let event = ctx
            .events
            .append(NewEvent {
                session_id: ctx.session_id.clone(),
                event_type: EventType::Message,
                source: format!("tool:{}", self.name()),
                data: json!({"role": "assistant", "content": content, "ui": "notify"}),
            })
            .await
            .map_err(|e| ToolError::Backend(e.to_string()))?;
        Ok(ToolOutput {
            ok: true,
            output: json!({"event_id": event.id, "final": args.get("final").and_then(Value::as_bool).unwrap_or(false)}),
            file_ref: None,
            error: None,
        })
    }
}

pub struct MessageAskUser;

#[async_trait]
impl Tool for MessageAskUser {
    fn name(&self) -> &'static str {
        "message_ask_user"
    }
    fn description(&self) -> &'static str {
        "Ask the user a question and block the agent loop until a reply arrives via the WebSocket user_response command."
    }
    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "content": { "type": "string", "description": "Question shown to the user." }
            },
            "required": ["content"],
            "additionalProperties": false,
        })
    }
    async fn invoke(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let content = require_str(&args, "content")?;
        let event = ctx
            .events
            .append(NewEvent {
                session_id: ctx.session_id.clone(),
                event_type: EventType::Message,
                source: format!("tool:{}", self.name()),
                data: json!({"role": "assistant", "content": content, "ui": "ask"}),
            })
            .await
            .map_err(|e| ToolError::Backend(e.to_string()))?;
        // Loop-blocking semantics land in story 0.14 (agent runner). Here we
        // just emit the event; the runner observes ui:"ask" and pauses.
        Ok(ToolOutput {
            ok: true,
            output: json!({"event_id": event.id, "call_id": event.id}),
            file_ref: None,
            error: None,
        })
    }
}

pub struct Idle;

#[async_trait]
impl Tool for Idle {
    fn name(&self) -> &'static str {
        "idle"
    }
    fn description(&self) -> &'static str {
        "Signal that the task is complete and the agent loop should terminate."
    }
    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "final": { "type": "boolean", "description": "Ignored; idle always implies final task completion." }
            },
            "additionalProperties": false,
        })
    }
    async fn invoke(&self, _args: Value, _ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        Ok(ToolOutput {
            ok: true,
            output: json!({"signal": "task_complete"}),
            file_ref: None,
            error: None,
        })
    }
}

pub struct SopRead;

#[async_trait]
impl Tool for SopRead {
    fn name(&self) -> &'static str {
        "sop_read"
    }
    fn description(&self) -> &'static str {
        "Look up a Standard Operating Procedure by id or title."
    }
    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "id": { "type": "string" },
                "title": { "type": "string" }
            },
            "additionalProperties": false,
        })
    }
    async fn invoke(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let id = args.get("id").and_then(Value::as_str).map(str::to_string);
        let title = args
            .get("title")
            .and_then(Value::as_str)
            .map(str::to_string);
        if id.is_none() && title.is_none() {
            return Err(ToolError::InvalidArgs(
                "one of 'id' or 'title' is required".into(),
            ));
        }

        let row = ctx
            .events
            .with_conn(move |conn| {
                if let Some(id) = id {
                    conn.query_row(
                        "SELECT id, title, content, version, enforced FROM sops WHERE id = ?",
                        [id],
                        |r| {
                            Ok(json!({
                                "id": r.get::<_, String>(0)?,
                                "title": r.get::<_, String>(1)?,
                                "content": r.get::<_, String>(2)?,
                                "version": r.get::<_, i64>(3)?,
                                "enforced": r.get::<_, bool>(4)?,
                            }))
                        },
                    )
                    .optional()
                } else {
                    conn.query_row(
                        "SELECT id, title, content, version, enforced FROM sops WHERE title = ?",
                        [title.unwrap_or_default()],
                        |r| {
                            Ok(json!({
                                "id": r.get::<_, String>(0)?,
                                "title": r.get::<_, String>(1)?,
                                "content": r.get::<_, String>(2)?,
                                "version": r.get::<_, i64>(3)?,
                                "enforced": r.get::<_, bool>(4)?,
                            }))
                        },
                    )
                    .optional()
                }
            })
            .await
            .map_err(|e| ToolError::Backend(e.to_string()))?;

        Ok(ToolOutput {
            ok: true,
            output: row.unwrap_or(Value::Null),
            file_ref: None,
            error: None,
        })
    }
}

pub struct GlossaryLookup;

#[async_trait]
impl Tool for GlossaryLookup {
    fn name(&self) -> &'static str {
        "glossary_lookup"
    }
    fn description(&self) -> &'static str {
        "Look up an organizational term in the glossary."
    }
    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "term": { "type": "string" }
            },
            "required": ["term"],
            "additionalProperties": false,
        })
    }
    async fn invoke(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let term = require_str(&args, "term")?;
        let row = ctx
            .events
            .with_conn(move |conn| {
                conn.query_row(
                    "SELECT term, definition, category FROM glossary WHERE term = ? COLLATE NOCASE",
                    [term],
                    |r| {
                        Ok(json!({
                            "term": r.get::<_, String>(0)?,
                            "definition": r.get::<_, String>(1)?,
                            "category": r.get::<_, String>(2)?,
                        }))
                    },
                )
                .optional()
            })
            .await
            .map_err(|e| ToolError::Backend(e.to_string()))?;

        Ok(ToolOutput {
            ok: true,
            output: row.unwrap_or(Value::Null),
            file_ref: None,
            error: None,
        })
    }
}

pub struct PlaybookSearch;

#[async_trait]
impl Tool for PlaybookSearch {
    fn name(&self) -> &'static str {
        "playbook_search"
    }
    fn description(&self) -> &'static str {
        "Full-text search over learned playbooks."
    }
    fn schema(&self) -> Value {
        obj_schema(
            json!({
                "query": { "type": "string" },
                "limit": { "type": "integer", "minimum": 0 }
            }),
            &["query"],
        )
    }
    async fn invoke(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let query = require_str(&args, "query")?;
        let raw_limit = args.get("limit").and_then(Value::as_i64).unwrap_or(3);
        if raw_limit < 0 {
            return Err(ToolError::InvalidArgs("limit must be >= 0".into()));
        }
        let limit = std::cmp::min(raw_limit as usize, 3);
        if limit == 0 {
            return Ok(ToolOutput {
                ok: true,
                output: json!([]),
                file_ref: None,
                error: None,
            });
        }

        let normalized = query.split_whitespace().collect::<Vec<_>>().join(" ");
        if normalized.is_empty() {
            return Ok(ToolOutput {
                ok: true,
                output: json!([]),
                file_ref: None,
                error: None,
            });
        }
        let fts_query = normalized
            .split(' ')
            .filter(|t| !t.is_empty())
            .map(|t| format!("{t}*"))
            .collect::<Vec<_>>()
            .join(" ");

        let session_id = ctx.session_id.clone();
        let hits = ctx
            .events
            .with_conn(move |conn| -> rusqlite::Result<Value> {
                let project_id: Option<String> = conn
                    .query_row(
                        "SELECT t.project_id
                         FROM sessions s
                         JOIN tasks t ON t.id = s.task_id
                         WHERE s.id = ?",
                        [session_id],
                        |r| r.get(0),
                    )
                    .optional()?;

                let Some(project_id) = project_id else {
                    return Ok(json!([]));
                };

                let mut stmt = conn.prepare(
                    "SELECT p.id, p.title, substr(p.content, 1, 512), -bm25(playbooks_fts) AS score
                     FROM playbooks_fts
                     JOIN playbooks p ON p.rowid = playbooks_fts.rowid
                     JOIN tasks src ON src.id = p.source_task_id
                     WHERE playbooks_fts MATCH ?
                       AND src.project_id = ?
                       AND p.status = 'active'
                     ORDER BY score DESC, p.id ASC
                     LIMIT ?",
                )?;

                let rows = stmt.query_map(rusqlite::params![fts_query, project_id, limit as i64], |r| {
                    Ok(json!({
                        "playbook_id": r.get::<_, String>(0)?,
                        "title": r.get::<_, String>(1)?,
                        "content_excerpt": r.get::<_, String>(2)?,
                        "match_score": r.get::<_, f64>(3)?,
                    }))
                })?;

                let mut out = Vec::new();
                for row in rows {
                    out.push(row?);
                }
                Ok(Value::Array(out))
            })
            .await
            .map_err(|e| ToolError::Backend(e.to_string()))?;

        Ok(ToolOutput {
            ok: true,
            output: hits,
            file_ref: None,
            error: None,
        })
    }
}

// ===== sandbox-backed tools (story 0.9 — representative subset) =====
//
// Phase-0 story 0.9 wires 4 representative sandbox tools to prove the
// dispatcher + sandbox_post() pattern works end-to-end. The remaining
// 18 sandbox-backed tools (3 file + 4 shell + 12 browser) stay as
// StubTools — a follow-up story will use the same pattern to wire them
// once AIO Sandbox API paths are verified end-to-end. See DEBT.md.

pub struct FileRead;
#[async_trait]
impl Tool for FileRead {
    fn name(&self) -> &'static str {
        "file_read"
    }
    fn description(&self) -> &'static str {
        "Read the contents of a file in the sandbox workspace."
    }
    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": { "path": { "type": "string" } },
            "required": ["path"],
            "additionalProperties": false,
        })
    }
    async fn invoke(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let path = require_str(&args, "path")?;
        sandbox_post(ctx, "/v1/file/read", json!({"file": path})).await
    }
}

pub struct FileWrite;
#[async_trait]
impl Tool for FileWrite {
    fn name(&self) -> &'static str {
        "file_write"
    }
    fn description(&self) -> &'static str {
        "Write content to a file in the sandbox workspace, overwriting if it exists."
    }
    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string" },
                "content": { "type": "string" }
            },
            "required": ["path", "content"],
            "additionalProperties": false,
        })
    }
    async fn invoke(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let path = require_str(&args, "path")?;
        let content = require_str(&args, "content")?;
        sandbox_post(
            ctx,
            "/v1/file/write",
            json!({"file": path, "content": content}),
        )
        .await
    }
}

pub struct ShellExec;
#[async_trait]
impl Tool for ShellExec {
    fn name(&self) -> &'static str {
        "shell_exec"
    }
    fn description(&self) -> &'static str {
        "Run a shell command in the sandbox and return its output."
    }
    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "cmd": { "type": "string" },
                "cwd": { "type": "string" }
            },
            "required": ["cmd"],
            "additionalProperties": false,
        })
    }
    async fn invoke(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let cmd = require_str(&args, "cmd")?;
        let mut body = json!({"command": cmd});
        if let Some(cwd) = args.get("cwd").and_then(Value::as_str) {
            body["cwd"] = Value::String(cwd.into());
        }
        sandbox_post(ctx, "/v1/shell/exec", body).await
    }
}

pub struct InfoSearchWeb;
#[async_trait]
impl Tool for InfoSearchWeb {
    fn name(&self) -> &'static str {
        "info_search_web"
    }
    fn description(&self) -> &'static str {
        "Web search via the configured provider (Brave default)."
    }
    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": { "type": "string" },
                "max_results": { "type": "integer", "minimum": 1, "maximum": 20 }
            },
            "required": ["query"],
            "additionalProperties": false,
        })
    }
    async fn invoke(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let query = require_str(&args, "query")?;
        let max_results = args
            .get("max_results")
            .and_then(Value::as_u64)
            .map(|n| n as usize)
            .unwrap_or(10);
        match ctx.search.web_search(&query, max_results).await {
            Ok(hits) => Ok(ToolOutput {
                ok: true,
                output: serde_json::to_value(&hits).unwrap_or(Value::Null),
                file_ref: None,
                error: None,
            }),
            Err(e) => Ok(ToolOutput {
                ok: false,
                output: Value::Null,
                file_ref: None,
                error: Some(ToolErrorPayload {
                    kind: match &e {
                        crate::search::SearchError::MissingApiKey(_) => "missing_api_key".into(),
                        crate::search::SearchError::ProviderNotImplemented(_) => {
                            "not_implemented".into()
                        }
                        _ => "search_failed".into(),
                    },
                    message: e.to_string(),
                }),
            }),
        }
    }
}

pub struct FileStrReplace;
#[async_trait]
impl Tool for FileStrReplace {
    fn name(&self) -> &'static str {
        "file_str_replace"
    }
    fn description(&self) -> &'static str {
        "Replace one substring with another in a sandbox file."
    }
    fn schema(&self) -> Value {
        obj_schema(
            json!({"path":{"type":"string"},"old_str":{"type":"string"},"new_str":{"type":"string"}}),
            &["path", "old_str", "new_str"],
        )
    }
    async fn invoke(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        sandbox_post(ctx, "/v1/file/replace", json!({"file": require_str(&args, "path")?, "old_str": require_str(&args, "old_str")?, "new_str": require_str(&args, "new_str")?})).await
    }
}

pub struct FileFindInContent;
#[async_trait]
impl Tool for FileFindInContent {
    fn name(&self) -> &'static str {
        "file_find_in_content"
    }
    fn description(&self) -> &'static str {
        "Find a regex pattern within a sandbox file's contents."
    }
    fn schema(&self) -> Value {
        obj_schema(
            json!({"path":{"type":"string"},"pattern":{"type":"string"}}),
            &["path", "pattern"],
        )
    }
    async fn invoke(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        sandbox_post(
            ctx,
            "/v1/file/search",
            json!({"file": require_str(&args, "path")?, "regex": require_str(&args, "pattern")?}),
        )
        .await
    }
}

pub struct FileFindByName;
#[async_trait]
impl Tool for FileFindByName {
    fn name(&self) -> &'static str {
        "file_find_by_name"
    }
    fn description(&self) -> &'static str {
        "Find files in the sandbox workspace whose names match a glob."
    }
    fn schema(&self) -> Value {
        obj_schema(
            json!({"glob":{"type":"string"},"cwd":{"type":"string"}}),
            &["glob"],
        )
    }
    async fn invoke(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let glob = require_str(&args, "glob")?;
        let cwd = args
            .get("cwd")
            .and_then(Value::as_str)
            .unwrap_or("/workspace");
        sandbox_post(ctx, "/v1/file/find", json!({"path": cwd, "glob": glob})).await
    }
}

pub struct ShellView;
#[async_trait]
impl Tool for ShellView {
    fn name(&self) -> &'static str {
        "shell_view"
    }
    fn description(&self) -> &'static str {
        "Read accumulated stdout/stderr from a running sandbox process."
    }
    fn schema(&self) -> Value {
        obj_schema(json!({"process_id":{"type":"string"}}), &["process_id"])
    }
    async fn invoke(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        sandbox_post(
            ctx,
            "/v1/shell/view",
            json!({"id": require_str(&args, "process_id")?}),
        )
        .await
    }
}

pub struct ShellWait;
#[async_trait]
impl Tool for ShellWait {
    fn name(&self) -> &'static str {
        "shell_wait"
    }
    fn description(&self) -> &'static str {
        "Block until a sandbox process exits, then return its exit code."
    }
    fn schema(&self) -> Value {
        obj_schema(
            json!({"process_id":{"type":"string"},"timeout_secs":{"type":"integer","minimum":1}}),
            &["process_id"],
        )
    }
    async fn invoke(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let mut body = json!({"id": require_str(&args, "process_id")?});
        if let Some(timeout) = args.get("timeout_secs").and_then(Value::as_u64) {
            body["max_wait_seconds"] = Value::Number(timeout.into());
        }
        sandbox_post(ctx, "/v1/shell/wait", body).await
    }
}

pub struct ShellWriteToProcess;
#[async_trait]
impl Tool for ShellWriteToProcess {
    fn name(&self) -> &'static str {
        "shell_write_to_process"
    }
    fn description(&self) -> &'static str {
        "Write data to a running sandbox process's stdin."
    }
    fn schema(&self) -> Value {
        obj_schema(
            json!({"process_id":{"type":"string"},"data":{"type":"string"}}),
            &["process_id", "data"],
        )
    }
    async fn invoke(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        sandbox_post(ctx, "/v1/shell/write", json!({"id": require_str(&args, "process_id")?, "input": require_str(&args, "data")?, "press_enter": false})).await
    }
}

pub struct ShellKillProcess;
#[async_trait]
impl Tool for ShellKillProcess {
    fn name(&self) -> &'static str {
        "shell_kill_process"
    }
    fn description(&self) -> &'static str {
        "Send SIGTERM/SIGKILL to a running sandbox process."
    }
    fn schema(&self) -> Value {
        obj_schema(json!({"process_id":{"type":"string"}}), &["process_id"])
    }
    async fn invoke(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        sandbox_post(
            ctx,
            "/v1/shell/kill",
            json!({"id": require_str(&args, "process_id")?}),
        )
        .await
    }
}

pub struct BrowserView;
#[async_trait]
impl Tool for BrowserView {
    fn name(&self) -> &'static str {
        "browser_view"
    }
    fn description(&self) -> &'static str {
        "Take a screenshot + DOM dump of the current browser page."
    }
    fn schema(&self) -> Value {
        obj_schema(json!({}), &[])
    }
    async fn invoke(&self, _args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        // Story 1.16: this tool and the PostBrowserActionHook share one
        // canonical accessor (`SandboxClient::browser_view`) so the DOM
        // text captured for Track B is bit-identical to what the tool
        // returns directly.
        match ctx.sandbox.browser_view(&ctx.session_id).await {
            Ok(output) => Ok(ToolOutput {
                ok: true,
                output,
                file_ref: None,
                error: None,
            }),
            Err(crate::sandbox::SandboxError::HttpStatus { status, path }) => Ok(ToolOutput {
                ok: false,
                output: Value::Null,
                file_ref: None,
                error: Some(ToolErrorPayload {
                    kind: "sandbox_http".into(),
                    message: format!("HTTP {status} from {path}"),
                }),
            }),
            Err(err) => Err(ToolError::Backend(err.to_string())),
        }
    }
}

pub struct BrowserNavigate;
#[async_trait]
impl Tool for BrowserNavigate {
    fn name(&self) -> &'static str {
        "browser_navigate"
    }
    fn description(&self) -> &'static str {
        "Navigate the sandbox browser to a URL."
    }
    fn schema(&self) -> Value {
        obj_schema(json!({"url":{"type":"string","format":"uri"}}), &["url"])
    }
    async fn invoke(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        sandbox_post(
            ctx,
            "/v1/browser/page/navigate",
            json!({"url": require_str(&args, "url")?}),
        )
        .await
    }
}

pub struct BrowserRestart;
#[async_trait]
impl Tool for BrowserRestart {
    fn name(&self) -> &'static str {
        "browser_restart"
    }
    fn description(&self) -> &'static str {
        "Restart the sandbox browser session."
    }
    fn schema(&self) -> Value {
        obj_schema(json!({}), &[])
    }
    async fn invoke(&self, _args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        sandbox_post(ctx, "/v1/browser/restart", json!({})).await
    }
}

pub struct BrowserClick;
#[async_trait]
impl Tool for BrowserClick {
    fn name(&self) -> &'static str {
        "browser_click"
    }
    fn description(&self) -> &'static str {
        "Click an element matching a CSS selector."
    }
    fn schema(&self) -> Value {
        obj_schema(json!({"selector":{"type":"string"}}), &["selector"])
    }
    async fn invoke(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        sandbox_post(
            ctx,
            "/v1/browser/page/click",
            json!({"selector": require_str(&args, "selector")?}),
        )
        .await
    }
}

pub struct BrowserInput;
#[async_trait]
impl Tool for BrowserInput {
    fn name(&self) -> &'static str {
        "browser_input"
    }
    fn description(&self) -> &'static str {
        "Type text into an input matching a CSS selector."
    }
    fn schema(&self) -> Value {
        obj_schema(
            json!({"selector":{"type":"string"},"text":{"type":"string"}}),
            &["selector", "text"],
        )
    }
    async fn invoke(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        sandbox_post(ctx, "/v1/browser/page/fill", json!({"selector": require_str(&args, "selector")?, "text": require_str(&args, "text")?})).await
    }
}

pub struct BrowserMoveMouse;
#[async_trait]
impl Tool for BrowserMoveMouse {
    fn name(&self) -> &'static str {
        "browser_move_mouse"
    }
    fn description(&self) -> &'static str {
        "Move the mouse to viewport coordinates."
    }
    fn schema(&self) -> Value {
        obj_schema(
            json!({"x":{"type":"integer"},"y":{"type":"integer"}}),
            &["x", "y"],
        )
    }
    async fn invoke(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        browser_action(ctx, "MOVE_TO", json!({"x": args.get("x").and_then(Value::as_i64).ok_or_else(|| ToolError::InvalidArgs("missing 'x' integer".into()))?, "y": args.get("y").and_then(Value::as_i64).ok_or_else(|| ToolError::InvalidArgs("missing 'y' integer".into()))?})).await
    }
}

pub struct BrowserPressKey;
#[async_trait]
impl Tool for BrowserPressKey {
    fn name(&self) -> &'static str {
        "browser_press_key"
    }
    fn description(&self) -> &'static str {
        "Press a single key (e.g. Enter, Tab, ArrowDown)."
    }
    fn schema(&self) -> Value {
        obj_schema(json!({"key":{"type":"string"}}), &["key"])
    }
    async fn invoke(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        browser_action(ctx, "PRESS", json!({"key": require_str(&args, "key")?})).await
    }
}

pub struct BrowserSelectOption;
#[async_trait]
impl Tool for BrowserSelectOption {
    fn name(&self) -> &'static str {
        "browser_select_option"
    }
    fn description(&self) -> &'static str {
        "Select an option in a <select> by value."
    }
    fn schema(&self) -> Value {
        obj_schema(
            json!({"selector":{"type":"string"},"value":{"type":"string"}}),
            &["selector", "value"],
        )
    }
    async fn invoke(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        sandbox_post(ctx, "/v1/browser/page/select_option", json!({"selector": require_str(&args, "selector")?, "value": require_str(&args, "value")?})).await
    }
}

pub struct BrowserScrollUp;
#[async_trait]
impl Tool for BrowserScrollUp {
    fn name(&self) -> &'static str {
        "browser_scroll_up"
    }
    fn description(&self) -> &'static str {
        "Scroll the browser viewport up by one page."
    }
    fn schema(&self) -> Value {
        obj_schema(json!({}), &[])
    }
    async fn invoke(&self, _args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        browser_action(ctx, "SCROLL", json!({"dx": 0, "dy": -800})).await
    }
}

pub struct BrowserScrollDown;
#[async_trait]
impl Tool for BrowserScrollDown {
    fn name(&self) -> &'static str {
        "browser_scroll_down"
    }
    fn description(&self) -> &'static str {
        "Scroll the browser viewport down by one page."
    }
    fn schema(&self) -> Value {
        obj_schema(json!({}), &[])
    }
    async fn invoke(&self, _args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        browser_action(ctx, "SCROLL", json!({"dx": 0, "dy": 800})).await
    }
}

pub struct BrowserConsoleExec;
#[async_trait]
impl Tool for BrowserConsoleExec {
    fn name(&self) -> &'static str {
        "browser_console_exec"
    }
    fn description(&self) -> &'static str {
        "Run a JavaScript expression in the browser dev console."
    }
    fn schema(&self) -> Value {
        obj_schema(json!({"code":{"type":"string"}}), &["code"])
    }
    async fn invoke(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        sandbox_post(
            ctx,
            "/v1/browser/page/evaluate",
            json!({"expression": require_str(&args, "code")?}),
        )
        .await
    }
}

pub struct BrowserConsoleView;
#[async_trait]
impl Tool for BrowserConsoleView {
    fn name(&self) -> &'static str {
        "browser_console_view"
    }
    fn description(&self) -> &'static str {
        "Read the most recent dev console log lines."
    }
    fn schema(&self) -> Value {
        obj_schema(json!({}), &[])
    }
    async fn invoke(&self, _args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        sandbox_get(ctx, "/v1/browser/page/console").await
    }
}

pub struct PlanAdvance;

#[async_trait]
impl Tool for PlanAdvance {
    fn name(&self) -> &'static str {
        "plan_advance"
    }
    fn description(&self) -> &'static str {
        "Advance the current plan to the next phase. ADR-010."
    }
    fn schema(&self) -> Value {
        obj_schema(json!({}), &[])
    }
    async fn invoke(&self, _args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let plan = ctx
            .plan_manager
            .advance(&ctx.session_id)
            .await
            .map_err(|e| ToolError::Backend(e.to_string()))?;
        Ok(ToolOutput {
            ok: true,
            output: json!({"ok": true, "plan": plan}),
            file_ref: None,
            error: None,
        })
    }
}

pub struct PlanUpdate;

#[async_trait]
impl Tool for PlanUpdate {
    fn name(&self) -> &'static str {
        "plan_update"
    }
    fn description(&self) -> &'static str {
        "Replace the remaining phases of the plan with a new structured list. ADR-010."
    }
    fn schema(&self) -> Value {
        obj_schema(
            json!({"phases":{"type":"array","items":{"type":"object","properties":{"id":{"type":"integer"},"title":{"type":"string"},"capabilities":{"type":"array","items":{"type":"string"}},"status":{"type":"string","enum":["pending","active","done"]}},"required":["id","title"]}}}),
            &["phases"],
        )
    }
    async fn invoke(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let phases = parse_phases(&args)?;
        let plan = ctx
            .plan_manager
            .update(&ctx.session_id, phases, PlanMutationSource::Agent)
            .await
            .map_err(|e| ToolError::Backend(e.to_string()))?;
        Ok(ToolOutput {
            ok: true,
            output: json!({"ok": true, "plan": plan}),
            file_ref: None,
            error: None,
        })
    }
}

pub struct FeatureMarkDone;

#[async_trait]
impl Tool for FeatureMarkDone {
    fn name(&self) -> &'static str {
        "feature_mark_done"
    }
    fn description(&self) -> &'static str {
        "Mark one feature as done in /workspace/feature-list.json and emit an audit Misc event."
    }
    fn schema(&self) -> Value {
        obj_schema(json!({"feature_id":{"type":"string"}}), &["feature_id"])
    }
    async fn invoke(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let feature_id = require_str(&args, "feature_id")?;
        let mut list: FeatureList = ctx
            .sandbox
            .read_workspace_file_json(&ctx.session_id, "feature-list.json")
            .await
            .map_err(|e| ToolError::Backend(e.to_string()))?;

        let now = progress::now_micros();
        let Some(feature) = list.features.iter_mut().find(|f| f.id == feature_id) else {
            return Err(ToolError::InvalidArgs(format!(
                "unknown feature_id '{feature_id}'"
            )));
        };
        feature.status = FeatureStatus::Done;
        feature.completed_at = Some(now);
        let title = feature.title.clone();
        let phase_id = feature.plan_phase_id;

        ctx.sandbox
            .write_workspace_file_json(&ctx.session_id, "feature-list.json", &list)
            .await
            .map_err(|e| ToolError::Backend(e.to_string()))?;

        ctx.events
            .append(NewEvent {
                session_id: ctx.session_id.clone(),
                event_type: EventType::Misc,
                source: format!("tool:{}", self.name()),
                data: json!({"kind":"feature_done","feature_id":feature_id,"title":title}),
            })
            .await
            .map_err(|e| ToolError::Backend(e.to_string()))?;

        let active = ctx
            .plan_manager
            .snapshot(&ctx.session_id)
            .await
            .map_err(|e| ToolError::Backend(e.to_string()))?
            .current_phase_id;
        if active != Some(phase_id) {
            ctx.events
                .append(NewEvent {
                    session_id: ctx.session_id.clone(),
                    event_type: EventType::Misc,
                    source: format!("tool:{}", self.name()),
                    data: json!({
                        "kind":"feature_done_out_of_phase",
                        "feature_id":feature_id,
                        "plan_phase_id":phase_id,
                        "active_phase_id":active
                    }),
                })
                .await
                .map_err(|e| ToolError::Backend(e.to_string()))?;
        }

        Ok(ToolOutput {
            ok: true,
            output: json!({"feature_id": feature_id, "status": "done"}),
            file_ref: None,
            error: None,
        })
    }
}

pub struct ProgressUpdate;

#[async_trait]
impl Tool for ProgressUpdate {
    fn name(&self) -> &'static str {
        "progress_update"
    }
    fn description(&self) -> &'static str {
        "Append one timestamped line to /workspace/progress.txt."
    }
    fn schema(&self) -> Value {
        obj_schema(json!({"line":{"type":"string"}}), &["line"])
    }
    async fn invoke(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let line = require_str(&args, "line")?;
        let existing = ctx
            .sandbox
            .read_workspace_file(&ctx.session_id, "progress.txt")
            .await
            .map(|b| String::from_utf8_lossy(&b).into_owned())
            .unwrap_or_default();
        let updated = progress::append_line(&existing, &line);
        ctx.sandbox
            .write_workspace_file(&ctx.session_id, "progress.txt", updated.as_bytes())
            .await
            .map_err(|e| ToolError::Backend(e.to_string()))?;
        Ok(ToolOutput {
            ok: true,
            output: json!({"ok": true}),
            file_ref: None,
            error: None,
        })
    }
}

// ===== stub tool (story 0.7) =====
//
// Used for the 28 tools whose real backends land in stories 0.8 / 0.9 /
// 0.14. Every stub returns a stable shape so the agent runtime can
// distinguish "tool not ready" from a hard failure.

pub struct StubTool {
    pub name: &'static str,
    pub description: &'static str,
    pub schema: Value,
    pub pending_story: &'static str,
}

#[async_trait]
impl Tool for StubTool {
    fn name(&self) -> &'static str {
        self.name
    }
    fn description(&self) -> &'static str {
        self.description
    }
    fn schema(&self) -> Value {
        self.schema.clone()
    }
    async fn invoke(&self, _args: Value, _ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        Ok(ToolOutput {
            ok: false,
            output: Value::Null,
            file_ref: None,
            error: Some(ToolErrorPayload {
                kind: "not_implemented".into(),
                message: format!("backend pending (see {})", self.pending_story),
            }),
        })
    }
}

fn stub(
    name: &'static str,
    description: &'static str,
    schema: Value,
    pending_story: &'static str,
) -> Arc<dyn Tool> {
    Arc::new(StubTool {
        name,
        description,
        schema,
        pending_story,
    })
}

fn obj_schema(properties: Value, required: &[&str]) -> Value {
    json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": false,
    })
}

pub fn all() -> HashMap<&'static str, Arc<dyn Tool>> {
    let mut map: HashMap<&'static str, Arc<dyn Tool>> = HashMap::new();

    // Real (5) — from story 0.6.
    map.insert("message_notify_user", Arc::new(MessageNotifyUser));
    map.insert("message_ask_user", Arc::new(MessageAskUser));
    map.insert("idle", Arc::new(Idle));
    map.insert("sop_read", Arc::new(SopRead));
    map.insert("glossary_lookup", Arc::new(GlossaryLookup));

    // ===== File (5) — backend: Sandbox =====
    map.insert("file_read", Arc::new(FileRead));
    map.insert("file_write", Arc::new(FileWrite));
    map.insert("file_str_replace", Arc::new(FileStrReplace));
    map.insert("file_find_in_content", Arc::new(FileFindInContent));
    map.insert("file_find_by_name", Arc::new(FileFindByName));

    // ===== Shell (5) — backend: Sandbox =====
    map.insert("shell_exec", Arc::new(ShellExec));
    map.insert("shell_view", Arc::new(ShellView));
    map.insert("shell_wait", Arc::new(ShellWait));
    map.insert("shell_write_to_process", Arc::new(ShellWriteToProcess));
    map.insert("shell_kill_process", Arc::new(ShellKillProcess));

    // ===== Browser (12) — backend: Sandbox Chromium =====
    map.insert("browser_view", Arc::new(BrowserView));
    map.insert("browser_navigate", Arc::new(BrowserNavigate));
    map.insert("browser_restart", Arc::new(BrowserRestart));
    map.insert("browser_click", Arc::new(BrowserClick));
    map.insert("browser_input", Arc::new(BrowserInput));
    map.insert("browser_move_mouse", Arc::new(BrowserMoveMouse));
    map.insert("browser_press_key", Arc::new(BrowserPressKey));
    map.insert("browser_select_option", Arc::new(BrowserSelectOption));
    map.insert("browser_scroll_up", Arc::new(BrowserScrollUp));
    map.insert("browser_scroll_down", Arc::new(BrowserScrollDown));
    map.insert("browser_console_exec", Arc::new(BrowserConsoleExec));
    map.insert("browser_console_view", Arc::new(BrowserConsoleView));

    // ===== Search (1) — backend: Search client (story 0.9 real) =====
    map.insert("info_search_web", Arc::new(InfoSearchWeb));

    // ===== Deploy (2) — Phase 0 stubs (architecture §4.3 — real impl Phase 1+) =====
    map.insert(
        "deploy_expose_port",
        stub(
            "deploy_expose_port",
            "Expose a sandbox port over Tailscale Funnel (deferred beyond Phase 0).",
            obj_schema(
                json!({ "port": { "type": "integer", "minimum": 1, "maximum": 65535 } }),
                &["port"],
            ),
            "Phase 1+",
        ),
    );
    map.insert(
        "deploy_apply_deployment",
        stub(
            "deploy_apply_deployment",
            "Publish the workspace as a public deployment (deferred beyond Phase 0).",
            obj_schema(
                json!({
                    "name": { "type": "string" },
                    "subdir": { "type": "string" }
                }),
                &["name"],
            ),
            "Phase 1+",
        ),
    );

    // ===== Internal (playbook_search) =====
    map.insert("playbook_search", Arc::new(PlaybookSearch));

    // ===== Plan (2) — LLM-callable per ADR-010 =====
    map.insert("plan_advance", Arc::new(PlanAdvance));
    map.insert("plan_update", Arc::new(PlanUpdate));
    map.insert("feature_mark_done", Arc::new(FeatureMarkDone));
    map.insert("progress_update", Arc::new(ProgressUpdate));

    // ===== Checkpoint (2) — story 1.13 + 1.13b =====
    map.insert("checkpoint_label", Arc::new(CheckpointLabel));
    map.insert("checkpoint_rollback", Arc::new(CheckpointRollback));

    // ===== Deliverable (1) — story 2.14 =====
    // Always registered so the LLM sees the tool surface (mask policy
    // gates its availability per AgentMode). When the production deps
    // (DeliverableStore + RendererDispatcher + planner LLM) aren't
    // wired, the placeholder reports `task_deliver_not_wired` so the
    // operator gets a clear diagnostic instead of silent failure.
    // `register_builtin_tools_with_task_deliver` overrides the slot
    // with the real handler — main.rs / AgentRunner are the canonical
    // callers.
    map.insert("task_deliver", Arc::new(TaskDeliverNotWired));

    map
}

/// Story 2.14: registry builder used by `AppState::new` once the
/// production [`crate::deliverable::TaskDeliver`] dependencies are
/// resolved. Drops the placeholder + inserts the real handler.
pub fn all_with_task_deliver(
    deps: crate::deliverable::TaskDeliverDeps,
) -> HashMap<&'static str, Arc<dyn Tool>> {
    let mut map = all();
    map.insert(
        "task_deliver",
        Arc::new(crate::deliverable::TaskDeliver::new(deps)),
    );
    map
}

/// Placeholder used when the production [`crate::deliverable::TaskDeliver`]
/// hasn't been wired yet — keeps the catalog count stable (Phase 1
/// tests + spec-check expect 38 tools post-2.14) while giving the LLM
/// a clear error if it tries to invoke `task_deliver` against an
/// unwired runtime (Phase 0 / Phase 1 tests).
pub struct TaskDeliverNotWired;

#[async_trait]
impl Tool for TaskDeliverNotWired {
    fn name(&self) -> &'static str {
        "task_deliver"
    }
    fn description(&self) -> &'static str {
        "Hand a finished real-employee artifact back to the operator. \
         The `target_filename` extension picks the renderer. \
         (Phase 2 only — disabled when DeliverableStore + RendererDispatcher \
         deps aren't wired into AppState.)"
    }
    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "content": { "type": "string" },
                "target_filename": { "type": "string" },
                "citations": {
                    "type": "array",
                    "items": { "type": "integer" }
                }
            },
            "required": ["content", "target_filename"],
            "additionalProperties": false,
        })
    }
    async fn invoke(&self, _args: Value, _ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        Ok(ToolOutput {
            ok: false,
            output: json!({}),
            file_ref: None,
            error: Some(ToolErrorPayload {
                kind: "task_deliver_not_wired".into(),
                message: "task_deliver deps not registered into AppState".into(),
            }),
        })
    }
}

/// Story 1.13: attach a human-readable label to the next
/// `Plan{op:"advance"}` checkpoint. One-shot — the label is consumed by
/// the `CheckpointManager` the next time a phase advance commits.
pub struct CheckpointLabel;

#[async_trait]
impl Tool for CheckpointLabel {
    fn name(&self) -> &'static str {
        "checkpoint_label"
    }
    fn description(&self) -> &'static str {
        "Attach a short human-readable label to the next phase-advance checkpoint. \
         One-shot: applies to the very next `plan_advance` then clears."
    }
    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "label": {
                    "type": "string",
                    "description": "Label text. Max 80 characters.",
                    "maxLength": 80
                }
            },
            "required": ["label"],
            "additionalProperties": false,
        })
    }
    async fn invoke(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let label = require_str(&args, "label")?;
        if label.len() > 80 {
            return Ok(ToolOutput {
                ok: false,
                output: json!({"max": 80, "got": label.len()}),
                file_ref: None,
                error: Some(ToolErrorPayload {
                    kind: "label_too_long".into(),
                    message: format!("label is {} chars; cap is 80", label.len()),
                }),
            });
        }
        ctx.checkpoint_labels.set(&ctx.session_id, &label);
        Ok(ToolOutput {
            ok: true,
            output: json!({
                "label": label,
                "applies_to": "next_phase_advance"
            }),
            file_ref: None,
            error: None,
        })
    }
}

/// Story 1.13b: revert a prior phase checkpoint via
/// `git -C /workspace revert --no-commit <git_sha>` inside the sandbox.
/// Registered in the catalog but MASKED from every LLM-facing
/// `AgentMode` by [`crate::dispatch::mask::DefaultMaskPolicy`].
/// Architecture §2.6 explicitly chose `git revert --no-commit` over
/// `git reset --hard` so history is preserved and the agent sees a
/// follow-up Misc event describing the revert.
pub struct CheckpointRollback;

#[async_trait]
impl Tool for CheckpointRollback {
    fn name(&self) -> &'static str {
        "checkpoint_rollback"
    }
    fn description(&self) -> &'static str {
        "Revert a prior checkpoint via `git revert --no-commit` inside the sandbox. \
         Internal use only — masked from the LLM."
    }
    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "checkpoint_id":   { "type": "string" },
                "reason":          { "type": "string", "maxLength": 200 },
                "rolled_back_by":  { "type": "string", "default": "admin:cli" }
            },
            "required": ["checkpoint_id", "reason"],
            "additionalProperties": false,
        })
    }
    async fn invoke(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let checkpoint_id = require_str(&args, "checkpoint_id")?;
        let reason = require_str(&args, "reason")?;
        if reason.len() > 200 {
            return Ok(tool_err(
                "reason_too_long",
                json!({"max": 200, "got": reason.len()}),
            ));
        }
        let rolled_back_by = args
            .get("rolled_back_by")
            .and_then(Value::as_str)
            .unwrap_or("admin:cli")
            .to_string();

        let row = match ctx
            .checkpoints
            .get(&checkpoint_id)
            .await
            .map_err(|e| ToolError::Backend(format!("checkpoint get: {e}")))?
        {
            Some(r) => r,
            None => {
                return Ok(tool_err(
                    "checkpoint_not_found",
                    json!({"checkpoint_id": checkpoint_id}),
                ));
            }
        };

        // Codex review Finding E: validate the persisted `git_sha` is a
        // proper hex commit id (`[0-9a-f]{40}` for SHA-1, or up to 64 for
        // SHA-256) before shell interpolation. Normal checkpoint creation
        // captures the value from `git rev-parse HEAD` so this passes,
        // but the persistence layer accepts arbitrary input — a future
        // DB tamper or migration corruption must not become an injection.
        if !is_valid_git_sha(&row.git_sha) {
            return Ok(tool_err(
                "invalid_git_sha",
                json!({"checkpoint_id": checkpoint_id, "git_sha": row.git_sha}),
            ));
        }

        // Run the revert against the sandbox's /v1/shell/exec. We pass
        // through `sandbox_post`; non-zero exit codes are surfaced as
        // `revert_failed` tool errors so the caller can act.
        let revert_cmd = format!("git -C /workspace revert --no-commit {}", row.git_sha);
        let shell_out = sandbox_post(ctx, "/v1/shell/exec", json!({"command": revert_cmd})).await?;
        let exit_code = shell_out
            .output
            .get("exit_code")
            .and_then(Value::as_i64)
            .unwrap_or(0);
        if !shell_out.ok || exit_code != 0 {
            let stderr = shell_out
                .output
                .get("stderr")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            return Ok(tool_err(
                "revert_failed",
                json!({
                    "checkpoint_id": checkpoint_id,
                    "git_sha": row.git_sha,
                    "exit_code": exit_code,
                    "stderr": stderr,
                }),
            ));
        }

        let rolled_back_at = now_micros_for_rollback();
        ctx.checkpoints
            .mark_rolled_back(&checkpoint_id, rolled_back_at, &rolled_back_by)
            .await
            .map_err(|e| ToolError::Backend(format!("mark_rolled_back: {e}")))?;

        // Emit Misc so build_messages surfaces the rollback to the
        // agent's next iteration (architecture §2.6: agent must see a
        // follow-up describing the revert).
        //
        // Codex review Finding D / DEBT #67: previously this used `let _
        // = …` and silently swallowed the append failure. That risked
        // leaving git (reverted) + DB (`mark_rolled_back` succeeded) +
        // event stream (no Misc) in an incoherent 2-of-3 state. The
        // rollback is now best-effort coherent: if the append fails the
        // operator gets a `rollback_event_append_failed` tool error
        // (with the rolled_back row still committed — git + DB are
        // already past the point of no return) so they know the agent
        // loop has stale visibility and can re-emit a status by hand.
        ctx.events
            .append(crate::events::NewEvent {
                session_id: ctx.session_id.clone(),
                event_type: crate::events::EventType::Misc,
                source: "checkpoint".into(),
                data: json!({
                    "kind": "checkpoint_rollback",
                    "checkpoint_id": checkpoint_id,
                    "git_sha": row.git_sha,
                    "reason": reason,
                    "rolled_back_by": rolled_back_by,
                }),
            })
            .await
            .map_err(|e| {
                ToolError::Backend(format!(
                    "rollback_event_append_failed (git + DB already reverted; manual reconciliation needed): {e}"
                ))
            })?;

        Ok(ToolOutput {
            ok: true,
            output: json!({
                "checkpoint_id": checkpoint_id,
                "rolled_back_at": rolled_back_at,
            }),
            file_ref: None,
            error: None,
        })
    }
}

fn tool_err(kind: &str, output: Value) -> ToolOutput {
    ToolOutput {
        ok: false,
        output,
        file_ref: None,
        error: Some(ToolErrorPayload {
            kind: kind.to_string(),
            message: kind.to_string(),
        }),
    }
}

fn now_micros_for_rollback() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_micros() as i64)
        .unwrap_or(0)
}

/// Belt-and-suspenders validator for `checkpoints.git_sha` before it
/// reaches a sandbox shell command. Accepts the standard SHA-1 hex
/// length (40) and the SHA-256 length git is migrating to (64), both
/// lowercase hex per `git rev-parse HEAD` output. Codex review
/// Finding E: the persistence layer takes arbitrary `NewCheckpoint.git_sha`
/// strings; this guard stops a tampered/corrupted row from injecting
/// shell metacharacters via `git -C /workspace revert --no-commit {sha}`.
fn is_valid_git_sha(sha: &str) -> bool {
    matches!(sha.len(), 40 | 64)
        && sha
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
}

fn parse_phases(args: &Value) -> Result<Vec<Phase>, ToolError> {
    let Some(items) = args.get("phases").and_then(Value::as_array) else {
        return Err(ToolError::InvalidArgs("missing 'phases' array".into()));
    };
    let mut phases = Vec::with_capacity(items.len());
    for item in items {
        let id = item
            .get("id")
            .and_then(Value::as_u64)
            .ok_or_else(|| ToolError::InvalidArgs("phase.id must be integer".into()))?
            as u32;
        let title = item
            .get("title")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::InvalidArgs("phase.title must be string".into()))?
            .to_string();
        let capabilities = item
            .get("capabilities")
            .and_then(Value::as_array)
            .map(|caps| {
                caps.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let status = match item
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("pending")
        {
            "active" => PhaseStatus::Active,
            "done" => PhaseStatus::Done,
            _ => PhaseStatus::Pending,
        };
        phases.push(Phase {
            id,
            title,
            capabilities,
            status,
        });
    }
    Ok(phases)
}
