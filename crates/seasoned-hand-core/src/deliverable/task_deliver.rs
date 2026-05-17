//! `task_deliver` LLM tool — Worker-mode entry point that hands a
//! real-employee artifact back through the channel framework.
//!
//! Pipeline (architecture §2.3 / §8):
//! 1. Validate `target_filename` extension via [`DeliverableFormat::from_filename`].
//! 2. Look up the originating task via `sessions.task_id`.
//! 3. Write the LLM-authored source to
//!    `/workspace/.deliverables/.source/<deliverable_id>.<src_ext>`.
//! 4. Render via [`RendererDispatcher`] (story 2.6).
//! 5. On failure: ONE retry via "simplify content" LLM call against
//!    the planner slot. Re-attempt the render.
//! 6. If second attempt fails: fall back to writing the source as
//!    `.md` (raw) and persist with `format = "md"`. Emit
//!    `Misc{kind:"deliverable_format_fallback"}` so the operator can
//!    diagnose.
//! 7. Persist the [`Deliverable`] row via [`DeliverableStore::insert`].
//!    Provenance manifest is the schema-version-only stub — story
//!    2.15 lands the full builder.
//! 8. Emit `Misc{kind:"deliverable"}` so the DeliveryRouter picks it
//!    up (story 2.5 already routes on this event).
//!
//! Mask: Worker-mode only. Initializer / Verifier modes get the tool
//! masked via [`crate::dispatch::mask::DefaultMaskPolicy`].
//!
//! refs: /specs/phase-2/architecture.md §2.3, §8
//! refs: /specs/phase-2/stories/story-2.14.md

use std::sync::Arc;

use async_trait::async_trait;
use rusqlite::OptionalExtension;
use serde_json::{Value, json};
use uuid::Uuid;

use super::renderer::{RenderError, RendererDispatcher};
use super::store::{DeliverableError, DeliverableStore, NewDeliverable};
use crate::checkpoint::CheckpointStore;
use crate::db::DbPool;
use crate::delivery::store::DeliveryEventStore;
use crate::events::{EventStore, EventType, NewEvent, sqlite::SqliteEventStore};
use crate::intake::store::IntakeEventStore;
use crate::llm::{ChatCompletionRequest, LlmClient, Message, Role};
use crate::project::brief::DeliverableFormat;
use crate::project::{ProjectStore, TaskStore};
use crate::provenance::{
    BuildDeps, INLINE_THRESHOLD_BYTES, ManifestInputs, build_manifest, persist_or_spill,
};
use crate::router::{SlotName, SlotRouter};
use crate::tools::{Tool, ToolContext, ToolError, ToolErrorPayload, ToolOutput};
use crate::verifier::VerificationStore;

pub const TOOL_NAME: &str = "task_deliver";

/// Max characters of `stderr` injected into the simplify prompt — keeps
/// the LLM input small and avoids leaking renderer internals into the
/// LLM's context.
const STDERR_PREVIEW_CHARS: usize = 200;

/// Per-LLM simplify token budget. Markdown / JSON content is usually
/// small; 4 k tokens covers most realistic deliverables without
/// risking provider-side timeouts.
const SIMPLIFY_MAX_TOKENS: u32 = 4096;

/// Tool dependencies threaded in at registration time (`AppState::new`
/// in production, an in-test fixture in unit tests). Keeping the deps
/// inside the struct avoids widening [`ToolContext`] across every
/// existing tool / test fixture.
#[derive(Clone)]
pub struct TaskDeliverDeps {
    pub deliverables: Arc<DeliverableStore>,
    pub renderer: Arc<RendererDispatcher>,
    /// Pool used to look up `sessions.task_id` for the originating
    /// task. The store layer doesn't expose this query because Phase 2
    /// has no other reader yet; if a second caller appears, lift this
    /// into a `SessionStore::task_id_for(...)` helper.
    pub db: DbPool,
    /// Optional — when `Some`, the renderer-failure retry path is
    /// enabled (planner-slot LLM call to simplify the content). When
    /// `None`, renderer failure short-circuits straight to the `.md`
    /// fallback. Tests typically wire `Some(stub_llm)` to exercise the
    /// retry without touching a real LLM provider.
    pub planner_llm: Option<Arc<dyn SimplifyLlm>>,
    /// Story 2.15: borrowed handles the provenance builder needs at
    /// deliverable-persist time. Kept as a single sub-struct so future
    /// renderer-side fields don't keep growing `TaskDeliverDeps` itself.
    pub provenance: ProvenanceDeps,
}

/// Borrowed-handle bundle threaded into `build_manifest` from
/// `task_deliver`. All fields are `Arc` so the dispatcher can clone
/// `TaskDeliverDeps` cheaply across the per-iteration tool catalog.
#[derive(Clone)]
pub struct ProvenanceDeps {
    pub task_store: Arc<TaskStore>,
    pub project_store: Arc<ProjectStore>,
    pub intake_store: Arc<IntakeEventStore>,
    pub delivery_store: Arc<DeliveryEventStore>,
    pub events: Arc<SqliteEventStore>,
    pub verifications: Arc<VerificationStore>,
    pub checkpoints: Arc<CheckpointStore>,
}

/// LLM seam for the simplify-and-retry path. Production wraps
/// [`LlmClient`] against the planner slot; tests substitute a
/// recording impl to assert the prompt shape + return canned content.
#[async_trait]
pub trait SimplifyLlm: Send + Sync {
    /// Return the simplified content, or `None` if the LLM declined /
    /// errored. `None` is treated by the caller as "skip the retry,
    /// fall back to `.md` directly".
    async fn simplify(
        &self,
        failed_content: &str,
        target_format: &str,
        stderr_preview: &str,
    ) -> Option<String>;
}

/// Production `SimplifyLlm` impl that calls the planner slot with a
/// small renderer-simplify system prompt. Built in
/// [`TaskDeliverDeps::from_app_state`] (or whatever AppState builder
/// wires it).
pub struct PlannerSimplifyLlm {
    pub llm: LlmClient,
    pub model: String,
}

impl PlannerSimplifyLlm {
    pub fn from_router(router: &SlotRouter) -> Self {
        let slot = router.resolve(SlotName::Planner);
        Self {
            llm: LlmClient::new(slot.base_url.clone(), slot.api_key.clone()),
            model: slot.model.clone(),
        }
    }
}

#[async_trait]
impl SimplifyLlm for PlannerSimplifyLlm {
    async fn simplify(
        &self,
        failed_content: &str,
        target_format: &str,
        stderr_preview: &str,
    ) -> Option<String> {
        let system = format!(
            "You are an assistant that rewrites a document so a renderer can produce \
             a `{target_format}` file from it. Remove complex tables, images, and fancy \
             formatting while preserving the meaning. Return ONLY the rewritten content \
             with no preamble or explanation."
        );
        let user = format!(
            "The renderer failed with stderr: {stderr_preview}\n\n\
             Original content:\n```\n{failed_content}\n```"
        );
        let resp = self
            .llm
            .chat_completion(ChatCompletionRequest {
                model: self.model.clone(),
                messages: vec![
                    Message {
                        role: Role::System,
                        content: Some(system),
                        name: None,
                        tool_calls: None,
                        tool_call_id: None,
                    },
                    Message {
                        role: Role::User,
                        content: Some(user),
                        name: None,
                        tool_calls: None,
                        tool_call_id: None,
                    },
                ],
                tools: None,
                tool_choice: None,
                temperature: Some(0.0),
                max_tokens: Some(SIMPLIFY_MAX_TOKENS),
                top_p: None,
            })
            .await
            .ok()?;
        resp.choices
            .first()
            .and_then(|c| c.message.content.clone())
            .filter(|s| !s.trim().is_empty())
    }
}

pub struct TaskDeliver {
    deps: TaskDeliverDeps,
}

impl TaskDeliver {
    pub fn new(deps: TaskDeliverDeps) -> Self {
        Self { deps }
    }
}

#[async_trait]
impl Tool for TaskDeliver {
    fn name(&self) -> &'static str {
        TOOL_NAME
    }

    fn description(&self) -> &'static str {
        "Hand a finished real-employee artifact back to the operator. \
         The `target_filename` extension picks the renderer (md/txt/json/csv pass-through; \
         docx/pdf/html/odt via Pandoc; pptx via python-pptx; xlsx via openpyxl). \
         `citations` is an array of `event_id`s that ground the content."
    }

    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "content": {
                    "type": "string",
                    "description": "LLM-authored source content. Markdown for prose / Pandoc \
                                    formats; JSON for pptx + xlsx (see arch §2.3)."
                },
                "target_filename": {
                    "type": "string",
                    "description": "Target deliverable filename including extension."
                },
                "citations": {
                    "type": "array",
                    "items": { "type": "integer" },
                    "description": "Optional event_id list grounding the content."
                }
            },
            "required": ["content", "target_filename"],
            "additionalProperties": false,
        })
    }

    async fn invoke(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let content = args
            .get("content")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::InvalidArgs("missing content".into()))?
            .to_string();
        let target_filename = args
            .get("target_filename")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::InvalidArgs("missing target_filename".into()))?
            .to_string();
        let citations: Option<Vec<i64>> = args.get("citations").and_then(|v| match v {
            Value::Array(items) => Some(items.iter().filter_map(Value::as_i64).collect()),
            _ => None,
        });

        let Some(target_format) = DeliverableFormat::from_filename(&target_filename) else {
            return Err(ToolError::InvalidArgs(format!(
                "unknown_format: {target_filename}"
            )));
        };
        // LLM-supplied target_filename feeds shell commands (pandoc /
        // python-pptx / openpyxl) and `workspace_host_path.join(...)`.
        // Without a strict allowlist, `..` segments escape the workspace
        // bind-mount on the host, and shell metacharacters land in the
        // renderer command line. The deliverable filename is the leaf
        // name of an artifact (`Q4-summary.docx`) — not a path or a
        // shell expression — so we enforce a strict basename-only
        // alphabet here. (REVIEW §1/B, proposed DEBT #36)
        validate_deliverable_filename(&target_filename)
            .map_err(|reason| ToolError::InvalidArgs(format!("invalid_filename: {reason}")))?;

        // Resolve task_id from sessions.task_id (set at session-row
        // insert time by WsInitializerSpawner). Missing → tool errors
        // out; we'd rather see a clear error than silently orphan a
        // Deliverable row.
        let session_id = ctx.session_id.clone();
        let task_id = lookup_task_id(&self.deps.db, &session_id)
            .await
            .map_err(|e| ToolError::Backend(format!("task_id lookup: {e}")))?
            .ok_or_else(|| {
                ToolError::Backend(format!(
                    "session {session_id} has no task_id; Deliverable cannot be persisted"
                ))
            })?;

        let deliverable_id = Uuid::new_v4().to_string();

        // First attempt — write source + render.
        let source_ext = source_ext_for(&target_format);
        let source_path = format!(".deliverables/.source/{deliverable_id}.{source_ext}");
        ctx.sandbox
            .write_workspace_file(&session_id, &source_path, content.as_bytes())
            .await
            .map_err(|e| ToolError::Backend(format!("write source: {e}")))?;

        let attempt_one = self
            .deps
            .renderer
            .render(content.as_bytes(), &target_filename, &session_id)
            .await;

        let (rendered, format_str, source_for_record, source_path_for_record, fallback_reason) =
            match attempt_one {
                Ok(artifact) => (
                    artifact,
                    format_to_str(&target_format).to_string(),
                    content.clone(),
                    source_path.clone(),
                    None,
                ),
                Err(RenderError::RendererFailed {
                    renderer,
                    exit_code,
                    stderr,
                    input_preview,
                }) => {
                    self.simplify_or_fallback(
                        ctx,
                        &session_id,
                        &target_format,
                        &target_filename,
                        &content,
                        renderer,
                        exit_code,
                        &stderr,
                        &input_preview,
                    )
                    .await?
                }
                Err(other) => {
                    return Ok(ToolOutput {
                        ok: false,
                        output: json!({}),
                        file_ref: None,
                        error: Some(ToolErrorPayload {
                            kind: "render_failed".into(),
                            message: other.to_string(),
                        }),
                    });
                }
            };

        // Story 2.15: build the full provenance manifest BEFORE
        // persisting the Deliverable row. The architecture (§2.11)
        // requires the manifest to land on the same INSERT.
        let source_sha = sha256_hex(source_for_record.as_bytes());
        let citations_vec: Vec<i64> = citations.clone().unwrap_or_default();
        let build_deps = BuildDeps {
            task_store: &self.deps.provenance.task_store,
            project_store: &self.deps.provenance.project_store,
            intake_store: &self.deps.provenance.intake_store,
            delivery_store: &self.deps.provenance.delivery_store,
            events: &self.deps.provenance.events,
            verifications: &self.deps.provenance.verifications,
            checkpoints: &self.deps.provenance.checkpoints,
            db: &self.deps.db,
        };
        let manifest = build_manifest(
            ManifestInputs {
                task_id: &task_id,
                deliverable_id: &deliverable_id,
                rendered_content_sha256: &rendered.sha256,
                source_content_sha256: Some(&source_sha),
                citations: &citations_vec,
            },
            &build_deps,
        )
        .await
        .map_err(|e| ToolError::Backend(format!("provenance: {e}")))?;
        let column = persist_or_spill(
            &manifest,
            ctx.sandbox.as_ref(),
            &session_id,
            &task_id,
            INLINE_THRESHOLD_BYTES,
        )
        .await
        .map_err(|e| ToolError::Backend(format!("provenance spill: {e}")))?;

        // DEBT #32 close-out: resolve the workspace-relative path that
        // [`RendererDispatcher`] returns (e.g. `.deliverables/foo.docx`)
        // into the absolute on-disk path via the sandbox handle's
        // `workspace_host_path` before persisting. `EmailChannel::deliver`
        // (and any future consumer that needs the rendered bytes off
        // disk) reads the column verbatim via `tokio::fs::read(...)`;
        // storing the relative form was a latent bug that only surfaced
        // once a non-FE consumer attempted I/O against the path.
        let absolute_rendered_path = ctx
            .sandbox
            .get(&session_id)
            .await
            .map(|handle| {
                handle
                    .workspace_host_path
                    .join(&rendered.workspace_path)
                    .display()
                    .to_string()
            })
            .unwrap_or_else(|| rendered.workspace_path.clone());

        // Persist Deliverable row.
        let new_row = NewDeliverable {
            task_id: task_id.clone(),
            tenant_id: None,
            format: format_str.clone(),
            source_content_path: Some(source_path_for_record.clone()),
            source_content_sha256: Some(source_sha),
            rendered_content_path: absolute_rendered_path,
            rendered_content_sha256: rendered.sha256.clone(),
            content_size: rendered.size as i64,
            citations: citations.clone(),
            provenance_manifest: column.into_column_value(),
        };
        let persisted_id = self
            .deps
            .deliverables
            .insert(new_row)
            .await
            .map_err(|e: DeliverableError| ToolError::Backend(e.to_string()))?;

        // Emit Misc{kind:"deliverable"} so the DeliveryRouter (story
        // 2.5) picks it up. Also emit the fallback Misc if we landed
        // on the .md path.
        if let Some(reason) = fallback_reason {
            let _ = ctx
                .events
                .append(NewEvent {
                    session_id: session_id.clone(),
                    event_type: EventType::Misc,
                    source: "task_deliver".into(),
                    data: json!({
                        "kind": "deliverable_format_fallback",
                        "target_format": format_to_str(&target_format),
                        "fell_back_to": "md",
                        "reason": reason,
                    }),
                })
                .await;
        }
        let _ = ctx
            .events
            .append(NewEvent {
                session_id: session_id.clone(),
                event_type: EventType::Misc,
                source: "task_deliver".into(),
                data: json!({
                    "kind": "deliverable",
                    "deliverable_id": persisted_id,
                    "format": format_str,
                    "file_ref": rendered.workspace_path,
                    "task_id": task_id,
                    "citations": citations,
                }),
            })
            .await;

        Ok(ToolOutput {
            ok: true,
            output: json!({
                "deliverable_id": persisted_id,
                "filename": target_filename,
                "format": format_str,
                "content_sha256": rendered.sha256,
                "content_size": rendered.size,
            }),
            file_ref: Some(rendered.workspace_path),
            error: None,
        })
    }
}

impl TaskDeliver {
    #[allow(clippy::too_many_arguments)]
    async fn simplify_or_fallback(
        &self,
        ctx: &ToolContext,
        session_id: &str,
        target_format: &DeliverableFormat,
        target_filename: &str,
        original_content: &str,
        renderer: &'static str,
        exit_code: i32,
        stderr: &str,
        _input_preview: &str,
    ) -> Result<
        (
            super::renderer::RenderedArtifact,
            String,
            String,
            String,
            Option<String>,
        ),
        ToolError,
    > {
        let stderr_preview = truncate(stderr, STDERR_PREVIEW_CHARS);

        // Step 1: planner-LLM simplify (if a simplifier is wired).
        if let Some(simplifier) = self.deps.planner_llm.clone() {
            let simplified = simplifier
                .simplify(
                    original_content,
                    format_to_str(target_format),
                    &stderr_preview,
                )
                .await;
            if let Some(new_content) = simplified {
                // Step 2: re-attempt render with the simplified content.
                let retry_id = Uuid::new_v4().to_string();
                let source_ext = source_ext_for(target_format);
                let retry_source_path = format!(".deliverables/.source/{retry_id}.{source_ext}");
                ctx.sandbox
                    .write_workspace_file(session_id, &retry_source_path, new_content.as_bytes())
                    .await
                    .map_err(|e| ToolError::Backend(format!("write simplified source: {e}")))?;
                match self
                    .deps
                    .renderer
                    .render(new_content.as_bytes(), target_filename, session_id)
                    .await
                {
                    Ok(artifact) => {
                        return Ok((
                            artifact,
                            format_to_str(target_format).to_string(),
                            new_content,
                            retry_source_path,
                            None,
                        ));
                    }
                    Err(error) => {
                        tracing::warn!(
                            target_format = format_to_str(target_format),
                            %error,
                            "task_deliver: simplify retry still failed; falling back to .md"
                        );
                    }
                }
            }
        }

        // Step 3: fall back to writing the original source as raw .md.
        let fallback_id = Uuid::new_v4().to_string();
        let fallback_filename = format!("{fallback_id}.md");
        // Raw renderer writes to /workspace/.deliverables/<filename>.md
        let artifact = super::renderer::raw::render(
            ctx.sandbox.as_ref(),
            session_id,
            original_content.as_bytes(),
            &format!(".deliverables/{fallback_filename}"),
        )
        .await
        .map_err(|e| ToolError::Backend(format!("fallback raw write: {e}")))?;
        let fallback_source_path = format!(".deliverables/.source/{fallback_id}.md");
        ctx.sandbox
            .write_workspace_file(
                session_id,
                &fallback_source_path,
                original_content.as_bytes(),
            )
            .await
            .map_err(|e| ToolError::Backend(format!("write fallback source: {e}")))?;
        Ok((
            artifact,
            "md".into(),
            original_content.to_string(),
            fallback_source_path,
            Some(format!(
                "renderer {renderer} failed (exit={exit_code}): {stderr_preview}"
            )),
        ))
    }
}

fn lookup_task_id(
    db: &DbPool,
    session_id: &str,
) -> impl std::future::Future<Output = Result<Option<String>, rusqlite::Error>> + Send + 'static {
    let sid = session_id.to_string();
    let db = db.clone();
    async move {
        db.with_conn(move |conn| {
            conn.query_row(
                "SELECT task_id FROM sessions WHERE id = ?",
                rusqlite::params![sid],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()
            .map(|maybe| maybe.flatten())
        })
        .await
    }
}

fn source_ext_for(format: &DeliverableFormat) -> &'static str {
    match format {
        // Markdown is the LLM's source for every Pandoc target.
        DeliverableFormat::Markdown
        | DeliverableFormat::Docx
        | DeliverableFormat::Pdf
        | DeliverableFormat::Html => "md",
        // JSON is the LLM's source for the structured formats.
        DeliverableFormat::Json | DeliverableFormat::Pptx | DeliverableFormat::Xlsx => "json",
        DeliverableFormat::Csv => "csv",
        DeliverableFormat::Code | DeliverableFormat::Url => "md",
    }
}

fn format_to_str(format: &DeliverableFormat) -> &'static str {
    match format {
        DeliverableFormat::Markdown => "md",
        DeliverableFormat::Json => "json",
        DeliverableFormat::Csv => "csv",
        DeliverableFormat::Docx => "docx",
        DeliverableFormat::Pdf => "pdf",
        DeliverableFormat::Html => "html",
        DeliverableFormat::Pptx => "pptx",
        DeliverableFormat::Xlsx => "xlsx",
        DeliverableFormat::Code => "code",
        DeliverableFormat::Url => "url",
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(bytes))
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        s.chars().take(n).collect()
    }
}

/// Reject any `target_filename` that isn't a strict basename. See the
/// call site for the threat model (REVIEW §1/B, proposed DEBT #36).
///
/// Accepts: `[A-Za-z0-9._-]+` of length 1..=120, with at least one
/// non-`.` character (no `..`, no `.`), with a non-`.` first character
/// (no hidden files), with at least one `.` to carry the extension.
///
/// Returns the reason string on rejection so callers can surface it
/// via the `invalid_filename:` error sub-code.
fn validate_deliverable_filename(name: &str) -> Result<(), &'static str> {
    if name.is_empty() {
        return Err("empty");
    }
    if name.len() > 120 {
        return Err("too_long");
    }
    if name.starts_with('.') {
        return Err("leading_dot");
    }
    if !name.contains('.') {
        return Err("missing_extension");
    }
    if name.contains("..") {
        return Err("parent_dir_segment");
    }
    for ch in name.chars() {
        let ok = ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-');
        if !ok {
            return Err("disallowed_character");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use crate::dispatch::mask::{AgentMode, DefaultMaskPolicy, ToolMaskPolicy};
    use crate::events::EventQuery;
    use crate::events::sqlite::SqliteEventStore;
    use crate::sandbox::{SandboxClient, SandboxHandle};
    use crate::tools::register_builtin_tools;
    use std::sync::Mutex;
    use tempfile::TempDir;
    use wiremock::matchers::{method, path as wm_path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// Recording SimplifyLlm so the retry-path tests can assert prompt
    /// shape + return canned simplified content.
    #[derive(Default)]
    struct RecordingSimplify {
        calls: Mutex<Vec<(String, String, String)>>,
        canned: Mutex<Option<String>>,
    }
    impl RecordingSimplify {
        fn with_canned(self, content: &str) -> Self {
            *self.canned.lock().unwrap() = Some(content.into());
            self
        }
        fn call_count(&self) -> usize {
            self.calls.lock().unwrap().len()
        }
    }
    #[async_trait]
    impl SimplifyLlm for RecordingSimplify {
        async fn simplify(
            &self,
            failed: &str,
            target_format: &str,
            stderr_preview: &str,
        ) -> Option<String> {
            self.calls.lock().unwrap().push((
                failed.into(),
                target_format.into(),
                stderr_preview.into(),
            ));
            self.canned.lock().unwrap().clone()
        }
    }

    /// Boot a fixture with a wiremock'd /v1/shell/exec returning the
    /// supplied (exit_code, stderr) for EVERY shell exec.
    async fn fixture_with_exit(
        exit_code: i32,
        stderr: &str,
        simplifier: Option<Arc<dyn SimplifyLlm>>,
    ) -> (
        TaskDeliver,
        ToolContext,
        TempDir,
        Arc<SqliteEventStore>,
        Arc<DeliverableStore>,
        Arc<SandboxClient>,
        Arc<RendererDispatcher>,
        String, // task_id
        String, // session_id
        MockServer,
    ) {
        let tmp = tempfile::tempdir().unwrap();
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(wm_path("/v1/shell/exec"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "exit_code": exit_code,
                "stdout": "",
                "stderr": stderr,
            })))
            .mount(&server)
            .await;

        let pool = db::open(":memory:").await.unwrap();
        let session_id = "sess-test".to_string();
        let projects_store = Arc::new(crate::project::ProjectStore::new(pool.clone()));
        let project_id = projects_store
            .insert(crate::project::NewProject {
                tenant_id: None,
                title: "P".into(),
                description: None,
            })
            .await
            .unwrap();
        let tasks_store = Arc::new(crate::project::TaskStore::new(pool.clone()));
        let task_id = tasks_store
            .insert(crate::project::NewTask {
                project_id,
                tenant_id: None,
                title: "T".into(),
                expected_due_at: None,
            })
            .await
            .unwrap();
        // Insert sessions row with task_id link.
        let sid = session_id.clone();
        let tid = task_id.clone();
        pool.with_conn(move |conn| {
            conn.execute(
                "INSERT INTO sessions (id, created_at, updated_at, state, task_id) \
                 VALUES (?, 0, 0, 'RUNNING', ?)",
                rusqlite::params![sid, tid],
            )
            .unwrap();
        })
        .await;

        let events = Arc::new(SqliteEventStore::new(pool.clone()));
        let sandbox =
            Arc::new(SandboxClient::new("ghcr.io/agent-infra/sandbox:test", tmp.path()).unwrap());
        sandbox
            .insert_handle_for_test(SandboxHandle {
                session_id: session_id.clone(),
                container_id: "c".into(),
                api_url: server.uri(),
                novnc_url: "http://127.0.0.1:0".into(),
                ttyd_url: "ws://127.0.0.1:0".into(),
                workspace_host_path: tmp.path().to_path_buf(),
            })
            .await;
        let renderer = Arc::new(RendererDispatcher::new(sandbox.clone()));
        let deliverables = Arc::new(DeliverableStore::new(pool.clone()));
        let intake_store = Arc::new(IntakeEventStore::new(pool.clone()));
        let delivery_store = Arc::new(DeliveryEventStore::new(pool.clone()));
        let verifications_store = Arc::new(VerificationStore::new(pool.clone()));
        let checkpoints_store = Arc::new(CheckpointStore::new(pool.clone()));
        let plan_manager = Arc::new(crate::plan::PlanManager::new(pool.clone(), events.clone()));

        let tool = TaskDeliver::new(TaskDeliverDeps {
            deliverables: deliverables.clone(),
            renderer: renderer.clone(),
            db: pool.clone(),
            planner_llm: simplifier,
            provenance: ProvenanceDeps {
                task_store: tasks_store.clone(),
                project_store: projects_store.clone(),
                intake_store: intake_store.clone(),
                delivery_store: delivery_store.clone(),
                events: events.clone(),
                verifications: verifications_store.clone(),
                checkpoints: checkpoints_store.clone(),
            },
        });

        let ctx = ToolContext {
            session_id: session_id.clone(),
            mask_mode: AgentMode::Worker,
            events: events.clone(),
            sandbox: sandbox.clone(),
            search: Arc::new(crate::search::SearchClient::new(
                crate::search::SearchProvider::Brave { api_key: None },
            )),
            plan_manager,
            checkpoint_labels: Arc::new(crate::checkpoint::CheckpointLabelBuffer::new()),
            checkpoints: Arc::new(crate::checkpoint::CheckpointStore::new(pool)),
            matcher_mode: crate::matcher::MatcherMode::Production,
        };
        (
            tool,
            ctx,
            tmp,
            events,
            deliverables,
            sandbox,
            renderer,
            task_id,
            session_id,
            server,
        )
    }

    #[tokio::test]
    async fn task_deliver_writes_source_and_renders() {
        let (tool, ctx, tmp, _events, deliverables, _sandbox, _renderer, task_id, _, _) =
            fixture_with_exit(0, "", None).await;

        let args = json!({
            "content": "# Hello world\n",
            "target_filename": "out.md",
            "citations": [1, 2],
        });
        let out = tool.invoke(args, &ctx).await.expect("ok");
        assert!(out.ok, "deliverable persisted");
        let deliverable_id = out
            .output
            .get("deliverable_id")
            .and_then(Value::as_str)
            .unwrap();
        let row = deliverables.get(deliverable_id).await.unwrap();
        assert_eq!(row.task_id, task_id);
        assert_eq!(row.format, "md");
        assert!(
            tmp.path().join(".deliverables/out.md").exists(),
            "rendered file on disk"
        );
        // DEBT #32 close-out: persisted path is absolute (resolved via
        // the sandbox handle's workspace_host_path) so downstream
        // I/O consumers (EmailChannel::deliver) can `tokio::fs::read`
        // it directly.
        assert!(
            std::path::Path::new(&row.rendered_content_path).is_absolute(),
            "rendered_content_path persisted as absolute: {}",
            row.rendered_content_path
        );
        assert!(
            row.rendered_content_path.ends_with(".deliverables/out.md"),
            "absolute path retains workspace-relative tail: {}",
            row.rendered_content_path
        );
    }

    #[tokio::test]
    async fn task_deliver_rejects_unknown_extension() {
        let (tool, ctx, _tmp, _, _, _, _, _, _, _) = fixture_with_exit(0, "", None).await;
        let err = tool
            .invoke(
                json!({"content": "x", "target_filename": "thing.rtf"}),
                &ctx,
            )
            .await
            .expect_err("rejected");
        match err {
            ToolError::InvalidArgs(msg) => assert!(msg.contains("unknown_format")),
            other => panic!("expected InvalidArgs, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn task_deliver_emits_misc_deliverable_event() {
        let (tool, ctx, _tmp, events, _, _, _, _, session_id, _) =
            fixture_with_exit(0, "", None).await;
        tool.invoke(json!({"content": "# hi", "target_filename": "x.md"}), &ctx)
            .await
            .unwrap();

        let rows = events
            .query(&session_id, EventQuery::default())
            .await
            .unwrap();
        assert!(
            rows.iter()
                .any(|e| e.data.get("kind").and_then(Value::as_str) == Some("deliverable")),
            "deliverable Misc emitted"
        );
    }

    #[test]
    fn task_deliver_masked_in_initializer_mode() {
        let policy = DefaultMaskPolicy;
        assert!(!policy.is_available("task_deliver", AgentMode::Initializer));
    }

    #[test]
    fn task_deliver_masked_in_verifier_mode() {
        let policy = DefaultMaskPolicy;
        assert!(!policy.is_available("task_deliver", AgentMode::Verifier));
        assert!(!policy.is_available("task_deliver", AgentMode::Internal));
        // Worker stays available.
        assert!(policy.is_available("task_deliver", AgentMode::Worker));
    }

    #[tokio::test]
    async fn task_deliver_retries_with_simplified_content_on_render_fail() {
        // First /v1/shell/exec returns exit=1 (pandoc fails); we plant
        // the rendered file on disk so the SECOND render's fingerprint
        // step succeeds. The mock returns exit=1 always, so the retry
        // would also "fail" — but because we wire the simplifier to
        // return content, the retry path is exercised. We use docx so
        // pandoc is the renderer.
        //
        // For this test we want the retry to SUCCEED → so we need
        // the second shell-exec to succeed. Set up a sequence: first
        // exec exit=1, subsequent exit=0. wiremock doesn't natively
        // support that; instead: first attempt fails → simplify is
        // called → second attempt also "fails" but we don't care since
        // the spec says "if second attempt fails, fall back to .md".
        //
        // So actually the simpler test: BOTH attempts fail, simplifier
        // returns canned content, and we assert (a) simplifier was
        // called once, and (b) the persisted deliverable's format is
        // "md" (the fallback path). That is covered by
        // task_deliver_falls_back_to_md_after_double_fail below — so
        // this test asserts only that simplify() was invoked when the
        // first render failed.
        let simplifier = Arc::new(RecordingSimplify::default().with_canned("# simplified\n"));
        let (tool, ctx, _tmp, _, _, _, _, _, _, _) =
            fixture_with_exit(1, "pandoc: oops", Some(simplifier.clone())).await;
        let _ = tool
            .invoke(
                json!({"content": "# big", "target_filename": "report.docx"}),
                &ctx,
            )
            .await
            .unwrap();
        assert_eq!(simplifier.call_count(), 1, "simplifier called exactly once");
    }

    #[tokio::test]
    async fn task_deliver_falls_back_to_md_after_double_fail() {
        // Both render attempts fail (every shell_exec returns exit=1);
        // simplifier returns Some content. Fallback kicks in →
        // deliverable persisted with format = "md" + fallback Misc emitted.
        let simplifier = Arc::new(RecordingSimplify::default().with_canned("# alt\n"));
        let (tool, ctx, _tmp, events, deliverables, _, _, _, session_id, _) =
            fixture_with_exit(1, "pandoc: still broken", Some(simplifier)).await;

        let out = tool
            .invoke(
                json!({"content": "# original", "target_filename": "x.docx"}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(out.ok, "deliverable persisted via fallback");
        let id = out
            .output
            .get("deliverable_id")
            .and_then(Value::as_str)
            .unwrap();
        let row = deliverables.get(id).await.unwrap();
        assert_eq!(row.format, "md", "fell back to md");

        let rows = events
            .query(&session_id, EventQuery::default())
            .await
            .unwrap();
        assert!(
            rows.iter()
                .any(|e| e.data.get("kind").and_then(Value::as_str)
                    == Some("deliverable_format_fallback")),
            "fallback Misc emitted"
        );
    }

    /// Spot-check the registry honours the production builder.
    #[test]
    fn task_deliver_registered_in_builtin_catalog() {
        let reg = register_builtin_tools();
        assert!(reg.contains_key("task_deliver"));
    }

    /// Validator accepts every test-fixture filename + a few realistic
    /// shapes; rejects path-traversal and shell metacharacters. Closes
    /// proposed DEBT #36 (REVIEW §1/B).
    #[test]
    fn validate_deliverable_filename_accepts_and_rejects() {
        // Accept: real fixtures + sane variants.
        assert!(validate_deliverable_filename("out.md").is_ok());
        assert!(validate_deliverable_filename("phase2-summary.docx").is_ok());
        assert!(validate_deliverable_filename("Q4_report.xlsx").is_ok());
        assert!(validate_deliverable_filename("notes-2026-05-16.pptx").is_ok());

        // Reject: path traversal. Any rejection is correct — the
        // important property is the input is denied. We pin specific
        // codes only where stable.
        assert!(validate_deliverable_filename("../etc/passwd.md").is_err());
        assert!(validate_deliverable_filename("..hidden.md").is_err());
        assert_eq!(
            validate_deliverable_filename("foo..bar.md"),
            Err("parent_dir_segment")
        );
        assert!(validate_deliverable_filename("/etc/passwd.md").is_err());
        assert!(validate_deliverable_filename("..").is_err());

        // Reject: shell metacharacters.
        assert_eq!(
            validate_deliverable_filename("x; rm -rf /.md"),
            Err("disallowed_character")
        );
        assert_eq!(
            validate_deliverable_filename("x$(whoami).md"),
            Err("disallowed_character")
        );
        assert_eq!(
            validate_deliverable_filename("x`id`.md"),
            Err("disallowed_character")
        );
        assert_eq!(
            validate_deliverable_filename("x|y.md"),
            Err("disallowed_character")
        );

        // Reject: path separators.
        assert_eq!(
            validate_deliverable_filename("sub/dir.md"),
            Err("disallowed_character")
        );
        assert_eq!(
            validate_deliverable_filename("sub\\dir.md"),
            Err("disallowed_character")
        );

        // Reject: control / null.
        assert_eq!(
            validate_deliverable_filename("x\0.md"),
            Err("disallowed_character")
        );

        // Reject: structural.
        assert_eq!(validate_deliverable_filename(""), Err("empty"));
        assert_eq!(
            validate_deliverable_filename("noextension"),
            Err("missing_extension")
        );
        assert_eq!(
            validate_deliverable_filename(".hidden.md"),
            Err("leading_dot")
        );
        let too_long = format!("{}.md", "a".repeat(120));
        assert_eq!(validate_deliverable_filename(&too_long), Err("too_long"));
    }
}
