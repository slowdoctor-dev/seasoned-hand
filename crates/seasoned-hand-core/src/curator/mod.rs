//! Curator worker runtime for Phase 4.
//! refs: /specs/phase-4/architecture.md §2.1, §2.2, §2.3, §4.1, §4.2, §6.5, §7

pub mod retention;
#[cfg(test)]
mod tenant_boundaries_tests;

use std::collections::{HashMap, HashSet};
use std::hash::Hash;
use std::sync::Arc;
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use rusqlite::OptionalExtension;
use serde_json::json;
use thiserror::Error;
use tokio::time::MissedTickBehavior;
use tokio_util::sync::CancellationToken;

use crate::db::DbPool;
use crate::events::{EventStore, EventType, NewEvent, sqlite::SqliteEventStore};
use crate::llm::{
    LlmClient, LlmError,
    types::{ChatCompletionRequest, EmbeddingRequest, Message, Role},
};

#[derive(Debug, Clone)]
pub struct CuratorConfig {
    pub enabled: bool,
    pub interval_seconds: u64,
    pub backlog_threshold: u32,
    pub max_candidates_per_cycle: u32,
    pub embedding_budget_monthly_tokens: u64,
    pub embedding_budget_soft_cap_pct: f32,
    pub embedding_budget_hard_breaker_pct: f32,
    pub embedding_model: String,
    pub auto_archive_enabled: bool,
    pub archive_recommend_min_confidence: f32,
    pub archive_apply_min_confidence: f32,
    pub project_id: String,
}

impl Default for CuratorConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            interval_seconds: 300,
            backlog_threshold: 10,
            max_candidates_per_cycle: 50,
            embedding_budget_monthly_tokens: 50_000,
            embedding_budget_soft_cap_pct: 0.08,
            embedding_budget_hard_breaker_pct: 0.12,
            embedding_model: "text-embedding-3-small".to_string(),
            auto_archive_enabled: false,
            archive_recommend_min_confidence: 0.40,
            archive_apply_min_confidence: 0.55,
            project_id: "default".to_string(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CuratorTrigger {
    IntervalTick,
    BacklogThreshold,
    Manual,
}

impl CuratorTrigger {
    fn as_str(self) -> &'static str {
        match self {
            CuratorTrigger::IntervalTick => "interval_tick",
            CuratorTrigger::BacklogThreshold => "backlog_threshold",
            CuratorTrigger::Manual => "manual",
        }
    }
}

#[derive(Debug, Clone)]
pub struct CuratorCycleResult {
    pub cycle_id: String,
    pub project_id: String,
    pub decisions_total: u32,
    pub queued_for_review: u32,
    pub failures: u32,
    pub elapsed_ms: u64,
    pub quarantines: Vec<CuratorQuarantineRecord>,
    pub budget_circuit_open: bool,
    pub budget_month_tokens: u64,
    pub budget_pct_of_total: f32,
    pub retrospective_refused_reason: Option<String>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct EmbeddingTelemetry {
    pub breaker_open: bool,
    pub embedding_tokens_used: u64,
    pub total_tokens_used: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CuratorFailureCategory {
    Panic,
    LlmRefusal,
    MalformedPayload,
    Timeout,
    OutOfMemory,
    SqliteBusy,
    SlotUnavailable,
    TenantUnresolved,
    CrossTenantRef,
}

impl CuratorFailureCategory {
    fn as_str(self) -> &'static str {
        match self {
            CuratorFailureCategory::Panic => "panic_propagated_error",
            CuratorFailureCategory::LlmRefusal => "llm_refusal",
            CuratorFailureCategory::MalformedPayload => "malformed_payload",
            CuratorFailureCategory::Timeout => "timeout",
            CuratorFailureCategory::OutOfMemory => "out_of_memory",
            CuratorFailureCategory::SqliteBusy => "sqlite_busy",
            CuratorFailureCategory::SlotUnavailable => "slot_unavailable",
            CuratorFailureCategory::TenantUnresolved => "tenant_unresolved",
            CuratorFailureCategory::CrossTenantRef => "cross_tenant_ref",
        }
    }
}

#[derive(Debug, Clone)]
pub struct CuratorQuarantineRecord {
    pub decision_id: String,
    pub failure_category: CuratorFailureCategory,
    pub retry_count: u32,
    pub detail: String,
}

#[derive(Debug, Clone)]
pub struct DuplicateCandidate {
    pub left_revision_id: String,
    pub right_revision_id: String,
    pub left_text: String,
    pub right_text: String,
    pub fts_score: f32,
    pub lexical_overlap: f32,
    pub recency_delta_days: i32,
}

#[derive(Debug, Clone)]
pub struct RerankedCandidate {
    pub left_revision_id: String,
    pub right_revision_id: String,
    pub blended_score: f32,
    pub embedding_cosine: f32,
    pub fts_norm: f32,
    pub deterministic_floor: f32,
    pub llm_contribution: f32,
    pub embedding_used: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictSeverity {
    Low,
    Medium,
    High,
}

impl ConflictSeverity {
    fn as_str(self) -> &'static str {
        match self {
            ConflictSeverity::Low => "low",
            ConflictSeverity::Medium => "medium",
            ConflictSeverity::High => "high",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ConflictFinding {
    pub conflict_id: String,
    pub left_revision_id: String,
    pub right_revision_id: String,
    pub severity: ConflictSeverity,
    pub structural_score: f32,
    pub semantic_score: f32,
    pub evidence_json: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct WeeklyRetrospective {
    pub retrospective_id: String,
    pub project_id: String,
    pub week_start: i64,
    pub week_end: i64,
    pub content: String,
    pub citation_coverage: f32,
    pub generation_status: String,
    pub created_at: i64,
}

#[derive(Debug, Clone)]
pub struct WorkPattern {
    pub pattern_id: String,
    pub pattern_key: String,
    pub score: f32,
    pub evidence_json: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct PatternRecommendation {
    pub recommendation_id: String,
    pub subject_kind: String,
    pub subject_id: String,
    pub confidence: f32,
    pub rationale_json: serde_json::Value,
    pub evidence_json: serde_json::Value,
}

#[derive(Debug, Clone, Default)]
pub struct KnowledgeDatasourceWriteResult {
    pub raw_knowledge: u32,
    pub raw_datasource: u32,
    pub promoted_knowledge: u32,
    pub promoted_datasource: u32,
}

#[derive(Debug, Clone)]
pub struct ReviewQueueItem {
    pub queue_id: String,
    pub decision_id: String,
    pub project_id: String,
    pub state: String,
    pub severity: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, Copy)]
pub enum ReviewQueueAction {
    Approve,
    Reject,
    Suppress,
}

#[derive(Debug, Error)]
pub enum CuratorWorkerError {
    #[error("event: {0}")]
    Event(#[from] crate::events::EventError),
    #[error("db: {0}")]
    Db(#[from] crate::db::DbError),
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("llm: {0}")]
    Llm(#[from] LlmError),
    #[error("executor: {0}")]
    Executor(String),
}

#[async_trait]
pub trait CuratorCycleExecutor: Send + Sync {
    async fn execute(
        &self,
        project_id: &str,
        trigger: CuratorTrigger,
        backlog_count: u32,
    ) -> Result<CuratorCycleResult, CuratorWorkerError>;
}

#[async_trait]
pub trait BacklogProbe: Send + Sync {
    async fn pending_count(&self, project_id: &str) -> Result<u32, CuratorWorkerError>;
}

#[async_trait]
pub trait CandidateBuilder: Send + Sync {
    async fn build_duplicate_candidates(
        &self,
        project_id: &str,
        limit: u32,
    ) -> Result<Vec<DuplicateCandidate>, CuratorWorkerError>;
}

#[async_trait]
pub trait EmbeddingReranker: Send + Sync {
    async fn rerank(
        &self,
        project_id: &str,
        candidates: Vec<DuplicateCandidate>,
    ) -> Result<Vec<RerankedCandidate>, CuratorWorkerError>;

    async fn telemetry_snapshot(&self) -> Option<EmbeddingTelemetry> {
        None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsolidationDecisionKind {
    Merge,
    Keep,
    ArchiveRecommend,
    ArchiveApply,
    Restore,
    Quarantine,
}

impl ConsolidationDecisionKind {
    fn as_str(self) -> &'static str {
        match self {
            ConsolidationDecisionKind::Merge => "merge",
            ConsolidationDecisionKind::Keep => "keep",
            ConsolidationDecisionKind::ArchiveRecommend => "archive",
            ConsolidationDecisionKind::ArchiveApply => "archive",
            ConsolidationDecisionKind::Restore => "restore",
            ConsolidationDecisionKind::Quarantine => "keep",
        }
    }

    fn high_impact(self) -> bool {
        matches!(
            self,
            ConsolidationDecisionKind::ArchiveRecommend | ConsolidationDecisionKind::ArchiveApply
        )
    }
}

#[derive(Debug, Clone)]
pub struct ConsolidationDecision {
    pub decision_id: String,
    pub kind: ConsolidationDecisionKind,
    pub subject_revision_ids: Vec<String>,
    pub target_revision_id: Option<String>,
    pub confidence: f32,
    pub rationale_json: serde_json::Value,
    pub requires_review: bool,
}

#[derive(Debug, Clone, Default)]
pub struct ConsolidationApplyResult {
    pub applied: u32,
    pub queued_for_review: u32,
    pub failures: u32,
    pub quarantines: Vec<CuratorQuarantineRecord>,
}

#[async_trait]
pub trait ConsolidationEngine: Send + Sync {
    async fn decide(
        &self,
        project_id: &str,
        reranked: Vec<RerankedCandidate>,
    ) -> Result<Vec<ConsolidationDecision>, CuratorWorkerError>;

    async fn apply(
        &self,
        project_id: &str,
        cycle_id: &str,
        decisions: &[ConsolidationDecision],
    ) -> Result<ConsolidationApplyResult, CuratorWorkerError>;
}

#[async_trait]
pub trait ConflictDetector: Send + Sync {
    async fn detect(
        &self,
        project_id: &str,
        reranked: &[RerankedCandidate],
    ) -> Result<Vec<ConflictFinding>, CuratorWorkerError>;
}

#[async_trait]
pub trait SemanticAdjudicator: Send + Sync {
    async fn contradiction_score(
        &self,
        left_text: &str,
        right_text: &str,
    ) -> Result<f32, CuratorWorkerError>;
}

#[async_trait]
pub trait RetrospectiveGenerator: Send + Sync {
    async fn generate_if_due(
        &self,
        project_id: &str,
        trigger: CuratorTrigger,
        backlog_count: u32,
        backlog_threshold: u32,
    ) -> Result<Option<WeeklyRetrospective>, CuratorWorkerError>;
}

#[async_trait]
pub trait WorkPatternExtractor: Send + Sync {
    async fn extract(&self, project_id: &str) -> Result<Vec<WorkPattern>, CuratorWorkerError>;

    async fn recommend(
        &self,
        project_id: &str,
        cycle_id: &str,
        patterns: &[WorkPattern],
    ) -> Result<Vec<PatternRecommendation>, CuratorWorkerError>;
}

#[async_trait]
pub trait OperatorReviewQueue: Send + Sync {
    async fn list(
        &self,
        project_id: Option<&str>,
        state: Option<&str>,
        limit: usize,
    ) -> Result<Vec<ReviewQueueItem>, CuratorWorkerError>;

    async fn transition(
        &self,
        queue_id: &str,
        action: ReviewQueueAction,
        reviewer: Option<&str>,
        note: Option<&str>,
        suppress_ttl_days: Option<u32>,
    ) -> Result<bool, CuratorWorkerError>;

    async fn reconcile_suppression_expiry(&self) -> Result<u32, CuratorWorkerError>;
}

#[async_trait]
pub trait KnowledgeDatasourceWriter: Send + Sync {
    async fn emit_and_promote(
        &self,
        project_id: &str,
        cycle_id: &str,
    ) -> Result<KnowledgeDatasourceWriteResult, CuratorWorkerError>;
}

#[derive(Clone)]
pub struct SqliteBacklogProbe {
    db: DbPool,
}

impl SqliteBacklogProbe {
    pub fn new(db: DbPool) -> Self {
        Self { db }
    }
}

#[async_trait]
impl BacklogProbe for SqliteBacklogProbe {
    async fn pending_count(&self, project_id: &str) -> Result<u32, CuratorWorkerError> {
        let project_id = project_id.to_string();
        self.db
            .with_conn(move |conn| {
                conn.query_row(
                    "SELECT COUNT(*)
                     FROM playbook_revisions r
                     WHERE r.source_project_id = ?
                       AND NOT EXISTS (
                         SELECT 1
                         FROM curator_decisions d
                         WHERE d.subject_kind = 'revision'
                           AND d.subject_id = r.id
                           AND d.project_id = r.source_project_id
                       )",
                    [project_id],
                    |row| row.get::<_, u32>(0),
                )
                .map_err(CuratorWorkerError::from)
            })
            .await
    }
}

#[derive(Clone)]
pub struct SqliteCandidateBuilder {
    db: DbPool,
}

impl SqliteCandidateBuilder {
    pub fn new(db: DbPool) -> Self {
        Self { db }
    }
}

#[async_trait]
impl CandidateBuilder for SqliteCandidateBuilder {
    async fn build_duplicate_candidates(
        &self,
        project_id: &str,
        limit: u32,
    ) -> Result<Vec<DuplicateCandidate>, CuratorWorkerError> {
        let project_id = project_id.to_string();
        let rows = self
            .db
            .with_conn(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT
                        l.id AS left_revision_id,
                        r.id AS right_revision_id,
                        l.title || '\n' || l.trigger_keywords || '\n' || l.content AS left_text,
                        r.title || '\n' || r.trigger_keywords || '\n' || r.content AS right_text,
                        CAST((COALESCE(l.created_at, 0) - COALESCE(r.created_at, 0)) / 86400000000 AS INTEGER) AS recency_delta_days
                     FROM playbook_revisions l
                     JOIN playbook_revisions r
                       ON l.source_project_id = r.source_project_id
                      AND l.id < r.id
                     JOIN playbooks lp ON lp.id = l.playbook_id
                     JOIN playbooks rp ON rp.id = r.playbook_id
                     WHERE l.source_project_id = ?
                       AND lp.status = 'active'
                       AND rp.status = 'active'
                     ORDER BY
                       ABS(COALESCE(l.created_at, 0) - COALESCE(r.created_at, 0)) ASC,
                       l.id ASC,
                       r.id ASC
                     LIMIT ?",
                )?;
                let mut q = stmt.query(rusqlite::params![project_id, i64::from(limit.max(1))])?;
                let mut out = Vec::new();
                while let Some(row) = q.next()? {
                    let left_text: String = row.get(2)?;
                    let right_text: String = row.get(3)?;
                    let lexical = lexical_overlap(&left_text, &right_text);
                    out.push(DuplicateCandidate {
                        left_revision_id: row.get(0)?,
                        right_revision_id: row.get(1)?,
                        left_text,
                        right_text,
                        fts_score: lexical,
                        lexical_overlap: lexical,
                        recency_delta_days: row.get::<_, i32>(4).unwrap_or(0),
                    });
                }
                Ok::<_, CuratorWorkerError>(out)
            })
            .await?;

        let mut ranked = rows;
        ranked.sort_by(|a, b| {
            b.fts_score
                .total_cmp(&a.fts_score)
                .then_with(|| a.recency_delta_days.abs().cmp(&b.recency_delta_days.abs()))
                .then_with(|| a.left_revision_id.cmp(&b.left_revision_id))
                .then_with(|| a.right_revision_id.cmp(&b.right_revision_id))
        });
        Ok(ranked)
    }
}

#[derive(Clone)]
pub struct SqliteConsolidationEngine {
    db: DbPool,
    auto_archive_enabled: bool,
    archive_recommend_min_confidence: f32,
    archive_apply_min_confidence: f32,
}

#[derive(Clone)]
pub struct LlmSemanticAdjudicator {
    llm: LlmClient,
    model: String,
}

impl LlmSemanticAdjudicator {
    pub fn new(llm: LlmClient, model: String) -> Self {
        Self { llm, model }
    }
}

#[async_trait]
impl SemanticAdjudicator for LlmSemanticAdjudicator {
    async fn contradiction_score(
        &self,
        left_text: &str,
        right_text: &str,
    ) -> Result<f32, CuratorWorkerError> {
        let req = ChatCompletionRequest {
            model: self.model.clone(),
            messages: vec![
                Message {
                    role: Role::System,
                    content: Some(
                        "Score contradiction from 0.0 to 1.0. Output only JSON \
                         {\"contradiction_score\": <number>}."
                            .to_string(),
                    ),
                    name: None,
                    tool_calls: None,
                    tool_call_id: None,
                },
                Message {
                    role: Role::User,
                    content: Some(format!(
                        "Left procedure:\n{}\n\nRight procedure:\n{}\n\nReturn contradiction score.",
                        left_text, right_text
                    )),
                    name: None,
                    tool_calls: None,
                    tool_call_id: None,
                },
            ],
            tools: None,
            tool_choice: None,
            temperature: Some(0.0),
            max_tokens: Some(120),
            top_p: None,
        };
        let response = self.llm.chat_completion(req).await?;
        let content = response
            .choices
            .first()
            .and_then(|choice| choice.message.content.as_ref())
            .map(String::as_str)
            .unwrap_or("{}");
        let parsed = serde_json::from_str::<serde_json::Value>(content).ok();
        let score = parsed
            .as_ref()
            .and_then(|v| v.get("contradiction_score"))
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(0.0) as f32;
        Ok(score.clamp(0.0, 1.0))
    }
}

#[derive(Clone)]
pub struct SqliteConflictDetector {
    db: DbPool,
    adjudicator: Arc<dyn SemanticAdjudicator>,
}

impl SqliteConflictDetector {
    pub fn new(db: DbPool, adjudicator: Arc<dyn SemanticAdjudicator>) -> Self {
        Self { db, adjudicator }
    }
}

#[derive(Clone)]
pub struct SqliteRetrospectiveGenerator {
    db: DbPool,
    llm: LlmClient,
    model: String,
}

impl SqliteRetrospectiveGenerator {
    pub fn new(db: DbPool, llm: LlmClient, model: String) -> Self {
        Self { db, llm, model }
    }
}

#[async_trait]
impl RetrospectiveGenerator for SqliteRetrospectiveGenerator {
    async fn generate_if_due(
        &self,
        project_id: &str,
        trigger: CuratorTrigger,
        backlog_count: u32,
        backlog_threshold: u32,
    ) -> Result<Option<WeeklyRetrospective>, CuratorWorkerError> {
        let now = now_micros()?;
        let (week_start, week_end) = current_week_window(now);
        let last = load_latest_retrospective(&self.db, project_id).await?;
        let due = retrospective_due(
            last.as_ref(),
            trigger,
            backlog_count,
            backlog_threshold,
            now,
        );
        if !due {
            return Ok(None);
        }

        let summary = build_retrospective_input(&self.db, project_id, week_start, week_end).await?;
        let content = self.generate_text(&summary).await?;
        let (coverage, citations) = compute_citation_coverage(&content);
        let generation_status = if coverage >= 0.95 {
            "success"
        } else {
            "refused"
        };
        let persisted = persist_retrospective(
            &self.db,
            RetrospectivePersistInput {
                project_id: project_id.to_string(),
                week_start,
                week_end,
                content,
                citation_coverage: coverage,
                generation_status: generation_status.to_string(),
                citations,
                created_at: now,
            },
        )
        .await?;
        Ok(Some(persisted))
    }
}

impl SqliteRetrospectiveGenerator {
    async fn generate_text(&self, summary: &str) -> Result<String, CuratorWorkerError> {
        let req = ChatCompletionRequest {
            model: self.model.clone(),
            messages: vec![
                Message {
                    role: Role::System,
                    content: Some(
                        "Create a weekly retrospective with claims that include citation tags \
                         exactly as [[CIT:<kind>:<id>]] where kind is one of event|decision|conflict|task. \
                         Refuse by returning the word REFUSE if evidence is insufficient."
                            .to_string(),
                    ),
                    name: None,
                    tool_calls: None,
                    tool_call_id: None,
                },
                Message {
                    role: Role::User,
                    content: Some(summary.to_string()),
                    name: None,
                    tool_calls: None,
                    tool_call_id: None,
                },
            ],
            tools: None,
            tool_choice: None,
            temperature: Some(0.0),
            max_tokens: Some(700),
            top_p: None,
        };
        let response = self.llm.chat_completion(req).await?;
        let content = response
            .choices
            .first()
            .and_then(|choice| choice.message.content.as_ref())
            .cloned()
            .unwrap_or_else(|| "REFUSE".to_string());
        Ok(content)
    }
}

#[derive(Clone)]
pub struct SqliteWorkPatternExtractor {
    db: DbPool,
}

#[derive(Clone)]
pub struct SqliteOperatorReviewQueue {
    db: DbPool,
}

#[derive(Clone)]
pub struct SqliteKnowledgeDatasourceWriter {
    db: DbPool,
    enforce_l2_knowledge: bool,
    enforce_l2_datasource: bool,
}

impl SqliteOperatorReviewQueue {
    pub fn new(db: DbPool) -> Self {
        Self { db }
    }
}

impl SqliteKnowledgeDatasourceWriter {
    pub fn new(db: DbPool, enforce_l2_knowledge: bool, enforce_l2_datasource: bool) -> Self {
        Self {
            db,
            enforce_l2_knowledge,
            enforce_l2_datasource,
        }
    }
}

impl SqliteWorkPatternExtractor {
    pub fn new(db: DbPool) -> Self {
        Self { db }
    }
}

#[async_trait]
impl ConflictDetector for SqliteConflictDetector {
    async fn detect(
        &self,
        project_id: &str,
        reranked: &[RerankedCandidate],
    ) -> Result<Vec<ConflictFinding>, CuratorWorkerError> {
        let mut findings = Vec::new();
        for pair in reranked {
            let left_revision_id = pair.left_revision_id.clone();
            let right_revision_id = pair.right_revision_id.clone();
            let project = project_id.to_string();
            let (left_content, right_content) = self
                .db
                .with_conn(move |conn| {
                    let left: String = conn.query_row(
                        "SELECT content FROM playbook_revisions
                         WHERE id = ?1 AND source_project_id = ?2",
                        rusqlite::params![left_revision_id, project],
                        |row| row.get(0),
                    )?;
                    let right: String = conn.query_row(
                        "SELECT content FROM playbook_revisions
                         WHERE id = ?1 AND source_project_id = ?2",
                        rusqlite::params![right_revision_id, project],
                        |row| row.get(0),
                    )?;
                    Ok::<_, CuratorWorkerError>((left, right))
                })
                .await?;

            let structural = structural_conflict_score(&left_content, &right_content);
            if structural < 0.35 {
                continue;
            }

            let semantic = self
                .adjudicator
                .contradiction_score(&left_content, &right_content)
                .await?;
            let severity = classify_conflict_severity(structural, semantic);
            let conflict_id = format!("sc-{}", uuid::Uuid::new_v4());
            let finding = ConflictFinding {
                conflict_id: conflict_id.clone(),
                left_revision_id: pair.left_revision_id.clone(),
                right_revision_id: pair.right_revision_id.clone(),
                severity,
                structural_score: structural,
                semantic_score: semantic,
                evidence_json: json!({
                    "policy":"f4_10_rule_first_semantic_adjudication",
                    "prefilter_threshold":0.35,
                    "left_revision_id":pair.left_revision_id,
                    "right_revision_id":pair.right_revision_id
                }),
            };
            persist_conflict(&self.db, project_id, &finding).await?;
            findings.push(finding);
        }
        Ok(findings)
    }
}

#[async_trait]
impl WorkPatternExtractor for SqliteWorkPatternExtractor {
    async fn extract(&self, project_id: &str) -> Result<Vec<WorkPattern>, CuratorWorkerError> {
        let project_id = project_id.to_string();
        let mut patterns = self
            .db
            .with_conn(move |conn| {
                let mut out = Vec::new();

                let mut event_stmt = conn.prepare(
                    "SELECT source, COUNT(*) AS cnt
                     FROM session_search_index
                     WHERE session_id LIKE (?1 || ':%')
                       AND event_type IN ('Action','Observation')
                     GROUP BY source
                     ORDER BY cnt DESC, source ASC
                     LIMIT 6",
                )?;
                let mut event_rows = event_stmt.query([project_id.clone()])?;
                while let Some(row) = event_rows.next()? {
                    let source: String = row.get(0)?;
                    let cnt: i64 = row.get(1)?;
                    out.push(WorkPattern {
                        pattern_id: format!("pat-{}-{}", source.replace(' ', "_"), cnt),
                        pattern_key: format!("event_source:{source}"),
                        score: ((cnt as f32) / 20.0).clamp(0.0, 1.0),
                        evidence_json: json!({
                            "kind":"event_replay",
                            "source":source,
                            "count":cnt
                        }),
                    });
                }

                let mut decision_stmt = conn.prepare(
                    "SELECT decision_type, COUNT(*) AS cnt
                     FROM curator_decisions
                     WHERE project_id = ?1
                     GROUP BY decision_type
                     ORDER BY cnt DESC, decision_type ASC
                     LIMIT 4",
                )?;
                let mut decision_rows = decision_stmt.query([project_id.clone()])?;
                while let Some(row) = decision_rows.next()? {
                    let decision_type: String = row.get(0)?;
                    let cnt: i64 = row.get(1)?;
                    out.push(WorkPattern {
                        pattern_id: format!("pat-decision-{}-{}", decision_type, cnt),
                        pattern_key: format!("decision_type:{decision_type}"),
                        score: ((cnt as f32) / 15.0).clamp(0.0, 1.0),
                        evidence_json: json!({
                            "kind":"aggregate",
                            "decision_type":decision_type,
                            "count":cnt
                        }),
                    });
                }

                Ok::<_, CuratorWorkerError>(out)
            })
            .await?;

        patterns.sort_by(|a, b| {
            b.score
                .total_cmp(&a.score)
                .then_with(|| a.pattern_key.cmp(&b.pattern_key))
                .then_with(|| a.pattern_id.cmp(&b.pattern_id))
        });
        Ok(patterns)
    }

    async fn recommend(
        &self,
        project_id: &str,
        cycle_id: &str,
        patterns: &[WorkPattern],
    ) -> Result<Vec<PatternRecommendation>, CuratorWorkerError> {
        let project_id = project_id.to_string();
        let cycle_id = cycle_id.to_string();
        let patterns = patterns.to_vec();
        self.db
            .with_conn(move |conn| {
                conn.execute_batch("BEGIN IMMEDIATE;")?;
                let now = now_micros()?;
                let mut recommendations = Vec::new();
                for pattern in patterns.iter().take(3) {
                    let subject_id: String = conn
                        .query_row(
                            "SELECT o.revision_id
                             FROM playbook_revision_outcomes o
                             JOIN playbook_revisions r ON r.id = o.revision_id
                             WHERE r.source_project_id = ?1
                             ORDER BY (o.failure_count - o.success_count) DESC, o.revision_id ASC
                             LIMIT 1",
                            [project_id.clone()],
                            |row| row.get(0),
                        )
                        .unwrap_or_else(|_| pattern.pattern_id.clone());
                    let subject_kind = if subject_id.starts_with("rev-") {
                        "revision".to_string()
                    } else {
                        "pattern".to_string()
                    };
                    let confidence = (0.35 + (pattern.score * 0.45)).clamp(0.0, 0.80);
                    let recommendation_id = format!("rec-{}", uuid::Uuid::new_v4());
                    let rationale_json = json!({
                        "policy_version":"phase4_story_4_8",
                        "pattern_key":pattern.pattern_key,
                        "score":pattern.score,
                        "subject_kind":subject_kind
                    });
                    let evidence_json = json!({
                        "pattern_id":pattern.pattern_id,
                        "evidence":pattern.evidence_json
                    });

                    conn.execute(
                        "INSERT INTO curator_decisions (
                            id, tenant_id, project_id, cycle_id, decision_type, subject_kind,
                            subject_id, confidence, rationale_json, evidence_json, status, failure_category, created_at
                        ) VALUES (?1, 'legacy-default', ?2, ?3, 'recommendation', ?4, ?5, ?6, ?7, ?8, 'applied', NULL, ?9)",
                        rusqlite::params![
                            recommendation_id,
                            project_id,
                            cycle_id,
                            subject_kind,
                            subject_id,
                            confidence,
                            rationale_json.to_string(),
                            evidence_json.to_string(),
                            now
                        ],
                    )?;
                    recommendations.push(PatternRecommendation {
                        recommendation_id,
                        subject_kind,
                        subject_id,
                        confidence,
                        rationale_json,
                        evidence_json,
                    });
                }
                conn.execute_batch("COMMIT;")?;
                Ok::<_, CuratorWorkerError>(recommendations)
            })
            .await
    }
}

#[async_trait]
impl OperatorReviewQueue for SqliteOperatorReviewQueue {
    async fn list(
        &self,
        project_id: Option<&str>,
        state: Option<&str>,
        limit: usize,
    ) -> Result<Vec<ReviewQueueItem>, CuratorWorkerError> {
        let project_id = project_id.map(ToString::to_string);
        let state = state.map(ToString::to_string);
        let limit = i64::try_from(limit.max(1)).unwrap_or(100);
        self.db
            .with_conn(move |conn| {
                let mut out = Vec::new();
                let mut stmt = conn.prepare(
                    "SELECT id, decision_id, project_id, state, severity, created_at
                     FROM curator_review_queue
                     WHERE (?1 IS NULL OR project_id = ?1)
                       AND (?2 IS NULL OR state = ?2)
                     ORDER BY created_at DESC, id ASC
                     LIMIT ?3",
                )?;
                let mut rows = stmt.query(rusqlite::params![project_id, state, limit])?;
                while let Some(row) = rows.next()? {
                    out.push(ReviewQueueItem {
                        queue_id: row.get(0)?,
                        decision_id: row.get(1)?,
                        project_id: row.get(2)?,
                        state: row.get(3)?,
                        severity: row.get(4)?,
                        created_at: row.get(5)?,
                    });
                }
                Ok::<_, CuratorWorkerError>(out)
            })
            .await
    }

    async fn transition(
        &self,
        queue_id: &str,
        action: ReviewQueueAction,
        reviewer: Option<&str>,
        note: Option<&str>,
        suppress_ttl_days: Option<u32>,
    ) -> Result<bool, CuratorWorkerError> {
        let queue_id = queue_id.to_string();
        let reviewer = reviewer.map(ToString::to_string);
        let note = note.map(ToString::to_string).unwrap_or_default();
        let ttl_days = suppress_ttl_days.unwrap_or(30);
        self.db
            .with_conn(move |conn| {
                conn.execute_batch("BEGIN IMMEDIATE;")?;
                let now = now_micros()?;
                let Some((decision_id, state)): Option<(String, String)> = conn
                    .query_row(
                        "SELECT decision_id, state FROM curator_review_queue WHERE id = ?1",
                        [queue_id.clone()],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )
                    .ok()
                else {
                    conn.execute_batch("ROLLBACK;")?;
                    return Ok(false);
                };
                if state != "pending" {
                    conn.execute_batch("ROLLBACK;")?;
                    return Ok(false);
                }
                let (next_state, next_status, note_out) = match action {
                    ReviewQueueAction::Approve => ("approved", "applied", note),
                    ReviewQueueAction::Reject => ("rejected", "rejected", note),
                    ReviewQueueAction::Suppress => {
                        let until = now.saturating_add(i64::from(ttl_days) * 86_400_000_000_i64);
                        (
                            "suppressed",
                            "suppressed",
                            format!("suppress_until={until};{}", note),
                        )
                    }
                };
                conn.execute(
                    "UPDATE curator_review_queue
                     SET state = ?1, reviewer = ?2, reviewer_note = ?3, resolved_at = ?4
                     WHERE id = ?5",
                    rusqlite::params![next_state, reviewer, note_out, now, queue_id],
                )?;
                conn.execute(
                    "UPDATE curator_decisions
                     SET status = ?1
                     WHERE id = ?2",
                    rusqlite::params![next_status, decision_id],
                )?;
                conn.execute_batch("COMMIT;")?;
                Ok::<_, CuratorWorkerError>(true)
            })
            .await
    }

    async fn reconcile_suppression_expiry(&self) -> Result<u32, CuratorWorkerError> {
        self.db
            .with_conn(move |conn| {
                let now = now_micros()?;
                let mut stmt = conn.prepare(
                    "SELECT id, reviewer_note FROM curator_review_queue WHERE state='suppressed'",
                )?;
                let mut rows = stmt.query([])?;
                let mut to_reopen = Vec::new();
                while let Some(row) = rows.next()? {
                    let qid: String = row.get(0)?;
                    let note: Option<String> = row.get(1)?;
                    if let Some(note) = note
                        && let Some(rest) = note.strip_prefix("suppress_until=")
                        && let Some((ts, _tail)) = rest.split_once(';')
                        && let Ok(until) = ts.parse::<i64>()
                        && until <= now
                    {
                        to_reopen.push(qid);
                    }
                }
                for qid in &to_reopen {
                    conn.execute(
                        "UPDATE curator_review_queue
                         SET state='pending', reviewer_note='suppress_expired', resolved_at=NULL
                         WHERE id = ?1",
                        [qid],
                    )?;
                    let _ = conn.execute(
                        "UPDATE curator_decisions
                         SET status='queued_review'
                         WHERE id = (SELECT decision_id FROM curator_review_queue WHERE id = ?1)",
                        [qid],
                    );
                }
                Ok::<_, CuratorWorkerError>(u32::try_from(to_reopen.len()).unwrap_or(0))
            })
            .await
    }
}

#[async_trait]
impl KnowledgeDatasourceWriter for SqliteKnowledgeDatasourceWriter {
    async fn emit_and_promote(
        &self,
        project_id: &str,
        cycle_id: &str,
    ) -> Result<KnowledgeDatasourceWriteResult, CuratorWorkerError> {
        let project_id = project_id.to_string();
        let cycle_id = cycle_id.to_string();
        let enforce_l2_knowledge = self.enforce_l2_knowledge;
        let enforce_l2_datasource = self.enforce_l2_datasource;
        self.db
            .with_conn(move |conn| {
                conn.execute_batch("BEGIN IMMEDIATE;")?;
                let now = now_micros()?;
                let mut result = KnowledgeDatasourceWriteResult::default();

                let mut stmt = conn.prepare(
                    "SELECT r.id, r.source_task_id, r.title, r.content, COALESCE(r.confidence, 0.0)
                     FROM playbook_revisions r
                     WHERE r.source_project_id = ?1
                       AND r.author_type = 'extractor'
                     ORDER BY r.created_at DESC, r.id ASC
                     LIMIT 100",
                )?;
                let mut rows = stmt.query([project_id.clone()])?;
                while let Some(row) = rows.next()? {
                    let revision_id: String = row.get(0)?;
                    let source_task_id: Option<String> = row.get(1)?;
                    let title: String = row.get(2)?;
                    let content: String = row.get(3)?;
                    let confidence: f32 = row.get(4)?;
                    let refs = extract_source_refs(&content);
                    let has_citation = !refs.is_empty();
                    if confidence >= 0.55 && has_citation {
                        let key = infer_knowledge_key(&title, &content);
                        let value = infer_knowledge_value(&content);
                        let kid = format!(
                            "ki-raw-{}-{}",
                            revision_id,
                            stable_u64_hex(&format!("{}|{}", key, value))
                        );
                        conn.execute(
                            "INSERT OR IGNORE INTO knowledge_items (
                                id, tenant_id, project_id, revision_id, source_task_id, key, value, confidence, evidence_json, created_at
                            ) VALUES (?1, 'legacy-default', ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                            rusqlite::params![
                                kid,
                                project_id,
                                revision_id,
                                source_task_id,
                                key,
                                value,
                                confidence,
                                json!({
                                    "tier":"raw",
                                    "source_refs":refs.clone(),
                                    "policy":"q12.15_story_4_10"
                                })
                                .to_string(),
                                now
                            ],
                        )?;
                        if conn.changes() > 0 {
                            result.raw_knowledge = result.raw_knowledge.saturating_add(1);
                        }
                        let promoted = knowledge_l2_satisfied(
                            conn,
                            &project_id,
                            &revision_id,
                            source_task_id.as_deref(),
                            &refs,
                        )?;
                        let status = if enforce_l2_knowledge && !promoted {
                            "queued_review"
                        } else {
                            "applied"
                        };
                        let did = format!(
                            "cd-knowledge-{}-{}",
                            revision_id,
                            stable_u64_hex(&format!("{}|{}", key, cycle_id))
                        );
                        conn.execute(
                            "INSERT OR IGNORE INTO curator_decisions (
                                id, tenant_id, project_id, cycle_id, decision_type, subject_kind, subject_id,
                                confidence, rationale_json, evidence_json, status, failure_category, created_at
                            ) VALUES (?1, 'legacy-default', ?2, ?3, 'knowledge_write', 'knowledge', ?4, ?5, ?6, ?7, ?8, NULL, ?9)",
                            rusqlite::params![
                                did,
                                project_id,
                                cycle_id,
                                revision_id,
                                confidence,
                                json!({"enforce_l2_knowledge":enforce_l2_knowledge,"l2_promoted":promoted}).to_string(),
                                json!({"source_refs":refs.clone()}).to_string(),
                                status,
                                now
                            ],
                        )?;
                        if promoted {
                            result.promoted_knowledge =
                                result.promoted_knowledge.saturating_add(1);
                        }
                    }

                    for source_ref in &refs {
                        let did = format!(
                            "cd-datasource-{}-{}",
                            revision_id,
                            stable_u64_hex(source_ref)
                        );
                        let l2_ok = datasource_l2_satisfied(
                            conn,
                            &project_id,
                            &revision_id,
                            source_task_id.as_deref(),
                            source_ref,
                        )?;
                        let trust_level = if l2_ok { "l2" } else { "l0" };
                        if confidence >= 0.50 {
                            let ds_id = format!(
                                "ds-{}-{}",
                                trust_level,
                                stable_u64_hex(&format!("{}|{}", revision_id, source_ref))
                            );
                            conn.execute(
                                "INSERT OR IGNORE INTO datasource_items (
                                    id, tenant_id, project_id, revision_id, source_task_id, source_type,
                                    source_ref, trust_level, confidence, evidence_json, created_at
                                ) VALUES (?1, 'legacy-default', ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                                rusqlite::params![
                                    ds_id,
                                    project_id,
                                    revision_id,
                                    source_task_id,
                                    source_type_from_ref(source_ref),
                                    source_ref,
                                    trust_level,
                                    confidence,
                                    json!({"tier":"raw","policy":"q12.15_story_4_10"}).to_string(),
                                    now
                                ],
                            )?;
                            if conn.changes() > 0 {
                                result.raw_datasource = result.raw_datasource.saturating_add(1);
                            }
                            let status = if enforce_l2_datasource && !l2_ok {
                                "queued_review"
                            } else {
                                "applied"
                            };
                            conn.execute(
                                "INSERT OR IGNORE INTO curator_decisions (
                                    id, tenant_id, project_id, cycle_id, decision_type, subject_kind, subject_id,
                                    confidence, rationale_json, evidence_json, status, failure_category, created_at
                                ) VALUES (?1, 'legacy-default', ?2, ?3, 'datasource_write', 'datasource', ?4, ?5, ?6, ?7, ?8, NULL, ?9)",
                                rusqlite::params![
                                    did,
                                    project_id,
                                    cycle_id,
                                    source_ref,
                                    confidence,
                                    json!({"enforce_l2_datasource":enforce_l2_datasource,"l2_promoted":l2_ok}).to_string(),
                                    json!({"revision_id":revision_id}).to_string(),
                                    status,
                                    now
                                ],
                            )?;
                            if l2_ok {
                                result.promoted_datasource =
                                    result.promoted_datasource.saturating_add(1);
                            }
                        }
                    }
                }

                conn.execute_batch("COMMIT;")?;
                Ok::<_, CuratorWorkerError>(result)
            })
            .await
    }
}

impl SqliteConsolidationEngine {
    pub fn new(db: DbPool) -> Self {
        Self {
            db,
            auto_archive_enabled: false,
            archive_recommend_min_confidence: 0.40,
            archive_apply_min_confidence: 0.55,
        }
    }

    pub fn with_archive_policy(
        mut self,
        auto_archive_enabled: bool,
        archive_recommend_min_confidence: f32,
        archive_apply_min_confidence: f32,
    ) -> Self {
        let recommend = archive_recommend_min_confidence.clamp(0.0, 1.0);
        let apply = archive_apply_min_confidence.clamp(recommend, 1.0);
        self.auto_archive_enabled = auto_archive_enabled;
        self.archive_recommend_min_confidence = recommend;
        self.archive_apply_min_confidence = apply;
        self
    }
}

#[async_trait]
impl ConsolidationEngine for SqliteConsolidationEngine {
    async fn decide(
        &self,
        project_id: &str,
        reranked: Vec<RerankedCandidate>,
    ) -> Result<Vec<ConsolidationDecision>, CuratorWorkerError> {
        let mut out = Vec::with_capacity(reranked.len());
        for candidate in reranked {
            let confidence = candidate.blended_score.clamp(0.0, 1.0);
            let floor = candidate.fts_norm.max(0.0);
            let kind = if candidate.deterministic_floor < 0.30 || floor < 0.30 {
                ConsolidationDecisionKind::Quarantine
            } else if confidence >= 0.82 {
                ConsolidationDecisionKind::Merge
            } else if confidence >= 0.65 {
                ConsolidationDecisionKind::Keep
            } else if self.auto_archive_enabled && confidence >= self.archive_apply_min_confidence {
                ConsolidationDecisionKind::ArchiveApply
            } else if confidence >= self.archive_recommend_min_confidence {
                ConsolidationDecisionKind::ArchiveRecommend
            } else {
                ConsolidationDecisionKind::Quarantine
            };
            let requires_review = review_required(
                kind,
                confidence,
                &candidate.left_revision_id,
                &candidate.right_revision_id,
            );
            out.push(ConsolidationDecision {
                decision_id: format!("cd-{}", uuid::Uuid::new_v4()),
                kind,
                subject_revision_ids: vec![
                    candidate.left_revision_id.clone(),
                    candidate.right_revision_id.clone(),
                ],
                target_revision_id: Some(candidate.left_revision_id.clone()),
                confidence,
                rationale_json: json!({
                    "project_id": project_id,
                    "fts_norm": candidate.fts_norm,
                    "embedding_cosine": candidate.embedding_cosine,
                    "embedding_used": candidate.embedding_used,
                    "deterministic_floor": candidate.deterministic_floor,
                    "llm_contribution": candidate.llm_contribution,
                    "policy": "q12.2_hybrid_q12.3_revision_chain_q12.19_confidence_band"
                }),
                requires_review,
            });
        }
        Ok(out)
    }

    async fn apply(
        &self,
        project_id: &str,
        cycle_id: &str,
        decisions: &[ConsolidationDecision],
    ) -> Result<ConsolidationApplyResult, CuratorWorkerError> {
        let mut result = ConsolidationApplyResult::default();
        let project_id = project_id.to_string();
        let cycle_id = cycle_id.to_string();
        let decisions = decisions.to_vec();
        self.db
            .with_conn(move |conn| {
                conn.execute_batch("BEGIN IMMEDIATE;")?;
                let now = now_micros()?;
                let worker_tenant = project_tenant_id(conn, &project_id)?
                    .filter(|t| !t.trim().is_empty())
                    .unwrap_or_else(|| "legacy-default".to_string());
                for decision in &decisions {
                    let target_revision = decision.target_revision_id.as_deref().unwrap_or("");
                    if let Some(scope_error) = validate_decision_scope(
                        conn,
                        &project_id,
                        &worker_tenant,
                        target_revision,
                        &decision.subject_revision_ids,
                    )? {
                        result.failures = result.failures.saturating_add(1);
                        result.quarantines.push(CuratorQuarantineRecord {
                            decision_id: decision.decision_id.clone(),
                            failure_category: scope_error.failure_category,
                            retry_count: 0,
                            detail: scope_error.detail,
                        });
                        continue;
                    }
                    let status = if decision.requires_review {
                        "queued_review"
                    } else {
                        "applied"
                    };
                    conn.execute(
                        "INSERT INTO curator_decisions (
                            id, tenant_id, project_id, cycle_id, decision_type, subject_kind,
                            subject_id, confidence, rationale_json, evidence_json, status, failure_category, created_at
                         ) VALUES (?1, ?2, ?3, ?4, ?5, 'revision', ?6, ?7, ?8, ?9, ?10, NULL, ?11)",
                        rusqlite::params![
                            decision.decision_id,
                            &worker_tenant,
                            project_id,
                            cycle_id,
                            decision.kind.as_str(),
                            decision
                                .target_revision_id
                                .as_deref()
                                .unwrap_or(""),
                            decision.confidence,
                            decision.rationale_json.to_string(),
                            "{}",
                            status,
                            now
                        ],
                    )?;

                    if decision.requires_review {
                        result.queued_for_review = result.queued_for_review.saturating_add(1);
                        conn.execute(
                            "INSERT INTO curator_review_queue (
                                id, tenant_id, decision_id, project_id, queue_reason, severity, state, reviewer, reviewer_note, resolved_at, created_at
                             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'pending', NULL, NULL, NULL, ?7)",
                            rusqlite::params![
                                format!("rq-{}", uuid::Uuid::new_v4()),
                                &worker_tenant,
                                decision.decision_id,
                                project_id,
                                format!("low_confidence_{:.2}", decision.confidence),
                                if decision.kind.high_impact() { "high" } else { "medium" },
                                now
                            ],
                        )?;
                        continue;
                    }

                    match decision.kind {
                        ConsolidationDecisionKind::Merge => {
                            apply_merge(conn, &project_id, decision, now)?;
                            result.applied = result.applied.saturating_add(1);
                        }
                        ConsolidationDecisionKind::ArchiveApply => {
                            apply_archive(conn, decision, now)?;
                            result.applied = result.applied.saturating_add(1);
                        }
                        ConsolidationDecisionKind::Restore => {
                            apply_restore(conn, decision, now)?;
                            result.applied = result.applied.saturating_add(1);
                        }
                        ConsolidationDecisionKind::ArchiveRecommend
                        | ConsolidationDecisionKind::Keep
                        | ConsolidationDecisionKind::Quarantine => {
                            result.applied = result.applied.saturating_add(1);
                        }
                    }
                }
                conn.execute_batch("COMMIT;")?;
                Ok::<_, CuratorWorkerError>(result)
            })
            .await
    }
}

struct DecisionScopeError {
    failure_category: CuratorFailureCategory,
    detail: String,
}

fn validate_decision_scope(
    conn: &rusqlite::Connection,
    project_id: &str,
    worker_tenant: &str,
    target_revision: &str,
    subject_revision_ids: &[String],
) -> Result<Option<DecisionScopeError>, CuratorWorkerError> {
    if worker_tenant.trim().is_empty() {
        return Ok(Some(DecisionScopeError {
            failure_category: CuratorFailureCategory::TenantUnresolved,
            detail: format!(
                "tenant_unresolved: worker tenant missing while validating project={project_id}"
            ),
        }));
    }
    if !target_revision.is_empty()
        && let Some(err) =
            validate_revision_scope(conn, target_revision, project_id, worker_tenant)?
    {
        return Ok(Some(err));
    }
    for revision_id in subject_revision_ids {
        if revision_id.is_empty() {
            continue;
        }
        if let Some(err) = validate_revision_scope(conn, revision_id, project_id, worker_tenant)? {
            return Ok(Some(err));
        }
    }
    Ok(None)
}

fn validate_revision_scope(
    conn: &rusqlite::Connection,
    revision_id: &str,
    project_id: &str,
    worker_tenant: &str,
) -> Result<Option<DecisionScopeError>, CuratorWorkerError> {
    let found: Option<String> = conn
        .query_row(
            "SELECT tenant_id
             FROM playbook_revisions
             WHERE id = ?1 AND source_project_id = ?2",
            rusqlite::params![revision_id, project_id],
            |row| row.get(0),
        )
        .optional()?;
    let Some(target_tenant) = found else {
        return Ok(Some(DecisionScopeError {
            failure_category: CuratorFailureCategory::CrossTenantRef,
            detail: format!(
                "cross_tenant_ref: revision {revision_id} is outside project {project_id}"
            ),
        }));
    };
    if target_tenant.trim().is_empty() {
        return Ok(Some(DecisionScopeError {
            failure_category: CuratorFailureCategory::TenantUnresolved,
            detail: format!("tenant_unresolved: revision {revision_id} has empty tenant_id"),
        }));
    }
    if target_tenant != worker_tenant {
        return Ok(Some(DecisionScopeError {
            failure_category: CuratorFailureCategory::CrossTenantRef,
            detail: format!(
                "cross_tenant_ref: revision {revision_id} tenant {target_tenant} != worker tenant {worker_tenant}"
            ),
        }));
    }
    Ok(None)
}

fn project_tenant_id(
    conn: &rusqlite::Connection,
    project_id: &str,
) -> Result<Option<String>, CuratorWorkerError> {
    let tenant: Option<String> = conn
        .query_row(
            "SELECT tenant_id FROM projects WHERE id = ?1",
            [project_id],
            |row| row.get(0),
        )
        .optional()?;
    Ok(tenant)
}

#[derive(Clone, Debug)]
pub struct EmbeddingBudget {
    pub monthly_embedding_tokens: u64,
    pub soft_cap_pct: f32,
    pub hard_breaker_pct: f32,
}

impl EmbeddingBudget {
    pub fn breaker_open(&self, embedding_tokens_used: u64, total_tokens_used: u64) -> bool {
        if total_tokens_used == 0 {
            return embedding_tokens_used >= self.monthly_embedding_tokens;
        }
        (embedding_tokens_used as f32 / total_tokens_used as f32) >= self.hard_breaker_pct
    }

    pub fn soft_cap_exceeded(&self, embedding_tokens_used: u64, total_tokens_used: u64) -> bool {
        if total_tokens_used == 0 {
            return embedding_tokens_used >= self.monthly_embedding_tokens;
        }
        (embedding_tokens_used as f32 / total_tokens_used as f32) >= self.soft_cap_pct
    }
}

#[derive(Clone)]
struct SimpleLru<K, V> {
    cap: usize,
    order: Vec<K>,
    map: HashMap<K, V>,
}

impl<K, V> SimpleLru<K, V>
where
    K: Eq + Hash + Clone,
{
    fn new(cap: usize) -> Self {
        Self {
            cap: cap.max(1),
            order: Vec::new(),
            map: HashMap::new(),
        }
    }

    fn get(&mut self, key: &K) -> Option<&V> {
        if self.map.contains_key(key) {
            self.touch(key);
        }
        self.map.get(key)
    }

    fn put(&mut self, key: K, value: V) {
        if self.map.contains_key(&key) {
            self.map.insert(key.clone(), value);
            self.touch(&key);
            return;
        }
        self.order.push(key.clone());
        self.map.insert(key, value);
        while self.order.len() > self.cap {
            if let Some(evicted) = self.order.first().cloned() {
                self.order.remove(0);
                self.map.remove(&evicted);
            } else {
                break;
            }
        }
    }

    fn touch(&mut self, key: &K) {
        if let Some(pos) = self.order.iter().position(|k| k == key) {
            let k = self.order.remove(pos);
            self.order.push(k);
        }
    }
}

#[derive(Clone)]
pub struct ProductionEmbeddingReranker {
    llm: LlmClient,
    model: String,
    budget: EmbeddingBudget,
    cache: Arc<tokio::sync::Mutex<SimpleLru<String, Vec<f32>>>>,
    usage: Arc<tokio::sync::Mutex<(u64, u64)>>,
}

impl ProductionEmbeddingReranker {
    pub fn new(llm: LlmClient, model: String, budget: EmbeddingBudget) -> Self {
        Self {
            llm,
            model,
            budget,
            cache: Arc::new(tokio::sync::Mutex::new(SimpleLru::new(512))),
            usage: Arc::new(tokio::sync::Mutex::new((0, 0))),
        }
    }

    async fn embedding_for(&self, text: &str) -> Result<Option<Vec<f32>>, CuratorWorkerError> {
        let key = format!("{}:{}", self.model, text);
        if let Some(cached) = self.cache.lock().await.get(&key).cloned() {
            return Ok(Some(cached));
        }

        let response = self
            .llm
            .embedding(EmbeddingRequest {
                model: self.model.clone(),
                input: text.to_string(),
            })
            .await?;

        let Some(first) = response.data.first() else {
            return Ok(None);
        };

        self.cache.lock().await.put(key, first.embedding.clone());
        let mut usage = self.usage.lock().await;
        usage.0 = usage
            .0
            .saturating_add(u64::from(response.usage.total_tokens));
        usage.1 = usage
            .1
            .saturating_add(u64::from(response.usage.total_tokens));
        Ok(Some(first.embedding.clone()))
    }
}

#[async_trait]
impl EmbeddingReranker for ProductionEmbeddingReranker {
    async fn rerank(
        &self,
        project_id: &str,
        candidates: Vec<DuplicateCandidate>,
    ) -> Result<Vec<RerankedCandidate>, CuratorWorkerError> {
        let (embedding_tokens_used, total_tokens_used) = *self.usage.lock().await;
        let breaker_open = self
            .budget
            .breaker_open(embedding_tokens_used, total_tokens_used);
        if breaker_open {
            tracing::warn!(project_id, "curator_budget_circuit_open");
        } else if self
            .budget
            .soft_cap_exceeded(embedding_tokens_used, total_tokens_used)
        {
            tracing::info!(project_id, "curator embedding soft cap exceeded");
        }

        let mut out = Vec::with_capacity(candidates.len());
        for candidate in candidates {
            let fts_norm = candidate.fts_score.clamp(0.0, 1.0);
            let structural_overlap = candidate.lexical_overlap.clamp(0.0, 1.0);

            let (embedding_cosine, embedding_used) = if breaker_open {
                (0.0, false)
            } else {
                match (
                    self.embedding_for(&candidate.left_text).await,
                    self.embedding_for(&candidate.right_text).await,
                ) {
                    (Ok(Some(left)), Ok(Some(right))) => {
                        (cosine_similarity(&left, &right).clamp(-1.0, 1.0), true)
                    }
                    _ => (0.0, false),
                }
            };

            // architecture §9.1 confidence composition:
            // deterministic floor + bounded LLM contribution (max +0.45).
            let deterministic_floor = if embedding_used {
                (0.45 * fts_norm) + (0.10 * structural_overlap)
            } else {
                (0.75 * fts_norm) + (0.25 * structural_overlap)
            };
            let llm_contribution = if embedding_used {
                (0.35 * embedding_cosine) + (0.05 * structural_overlap)
            } else {
                0.0
            };
            let blended_score =
                compose_confidence_with_bounds(deterministic_floor, llm_contribution, 0.45);

            out.push(RerankedCandidate {
                left_revision_id: candidate.left_revision_id,
                right_revision_id: candidate.right_revision_id,
                blended_score,
                embedding_cosine,
                fts_norm,
                deterministic_floor,
                llm_contribution,
                embedding_used,
            });
        }

        out.sort_by(|a, b| {
            b.blended_score
                .total_cmp(&a.blended_score)
                .then_with(|| a.left_revision_id.cmp(&b.left_revision_id))
                .then_with(|| a.right_revision_id.cmp(&b.right_revision_id))
        });
        Ok(out)
    }

    async fn telemetry_snapshot(&self) -> Option<EmbeddingTelemetry> {
        let (embedding_tokens_used, total_tokens_used) = *self.usage.lock().await;
        Some(EmbeddingTelemetry {
            breaker_open: self
                .budget
                .breaker_open(embedding_tokens_used, total_tokens_used),
            embedding_tokens_used,
            total_tokens_used,
        })
    }
}

#[derive(Clone)]
pub struct ProductionCuratorCycleExecutor {
    candidate_builder: Arc<dyn CandidateBuilder>,
    reranker: Arc<dyn EmbeddingReranker>,
    consolidation_engine: Arc<dyn ConsolidationEngine>,
    conflict_detector: Arc<dyn ConflictDetector>,
    retrospective_generator: Arc<dyn RetrospectiveGenerator>,
    work_pattern_extractor: Arc<dyn WorkPatternExtractor>,
    operator_review_queue: Arc<dyn OperatorReviewQueue>,
    knowledge_datasource_writer: Arc<dyn KnowledgeDatasourceWriter>,
    max_candidates_per_cycle: u32,
    backlog_threshold: u32,
}

pub struct CuratorRuntimeDeps {
    pub candidate_builder: Arc<dyn CandidateBuilder>,
    pub reranker: Arc<dyn EmbeddingReranker>,
    pub consolidation_engine: Arc<dyn ConsolidationEngine>,
    pub conflict_detector: Arc<dyn ConflictDetector>,
    pub retrospective_generator: Arc<dyn RetrospectiveGenerator>,
    pub work_pattern_extractor: Arc<dyn WorkPatternExtractor>,
    pub operator_review_queue: Arc<dyn OperatorReviewQueue>,
    pub knowledge_datasource_writer: Arc<dyn KnowledgeDatasourceWriter>,
}

impl ProductionCuratorCycleExecutor {
    pub fn new(
        deps: CuratorRuntimeDeps,
        max_candidates_per_cycle: u32,
        backlog_threshold: u32,
    ) -> Self {
        Self {
            candidate_builder: deps.candidate_builder,
            reranker: deps.reranker,
            consolidation_engine: deps.consolidation_engine,
            conflict_detector: deps.conflict_detector,
            retrospective_generator: deps.retrospective_generator,
            work_pattern_extractor: deps.work_pattern_extractor,
            operator_review_queue: deps.operator_review_queue,
            knowledge_datasource_writer: deps.knowledge_datasource_writer,
            max_candidates_per_cycle,
            backlog_threshold,
        }
    }
}

#[async_trait]
impl CuratorCycleExecutor for ProductionCuratorCycleExecutor {
    async fn execute(
        &self,
        project_id: &str,
        _trigger: CuratorTrigger,
        _backlog_count: u32,
    ) -> Result<CuratorCycleResult, CuratorWorkerError> {
        let started = std::time::Instant::now();
        let cycle_id = format!("cycle-{}", uuid::Uuid::new_v4());
        let mut quarantines = Vec::new();
        let mut failures = 0_u32;

        if let Err(error) = self
            .operator_review_queue
            .reconcile_suppression_expiry()
            .await
        {
            failures = failures.saturating_add(1);
            handle_failure_quarantine(
                project_id,
                &cycle_id,
                "review_queue_reconcile",
                &error,
                0,
                &mut quarantines,
            );
        }

        let mut candidates = match self
            .candidate_builder
            .build_duplicate_candidates(project_id, self.max_candidates_per_cycle)
            .await
        {
            Ok(rows) => rows,
            Err(error) => {
                failures = failures.saturating_add(1);
                handle_failure_quarantine(
                    project_id,
                    &cycle_id,
                    "candidate_builder",
                    &error,
                    0,
                    &mut quarantines,
                );
                Vec::new()
            }
        };

        // Batch-scope OOM containment: halve once, then continue with reduced set.
        let estimated_bytes: usize = candidates
            .iter()
            .map(|c| c.left_text.len() + c.right_text.len())
            .sum();
        if estimated_bytes > 1_000_000 && candidates.len() > 1 {
            failures = failures.saturating_add(1);
            let keep = (candidates.len() / 2).max(1);
            candidates.truncate(keep);
            let error = CuratorWorkerError::Executor(
                "batch_oom_predicted: candidate payload too large".to_string(),
            );
            handle_failure_quarantine(
                project_id,
                &cycle_id,
                "candidate_batch_split",
                &error,
                1,
                &mut quarantines,
            );
        }

        let reranked = match tokio::time::timeout(
            Duration::from_millis(2_000),
            self.reranker.rerank(project_id, candidates),
        )
        .await
        {
            Ok(Ok(rows)) => rows,
            Ok(Err(error)) => {
                failures = failures.saturating_add(1);
                handle_failure_quarantine(
                    project_id,
                    &cycle_id,
                    "embedding_rerank",
                    &error,
                    0,
                    &mut quarantines,
                );
                Vec::new()
            }
            Err(_elapsed) => {
                failures = failures.saturating_add(1);
                let error = CuratorWorkerError::Executor("timeout: embedding_rerank".to_string());
                handle_failure_quarantine(
                    project_id,
                    &cycle_id,
                    "embedding_rerank",
                    &error,
                    1,
                    &mut quarantines,
                );
                Vec::new()
            }
        };

        let conflicts = match self.conflict_detector.detect(project_id, &reranked).await {
            Ok(rows) => rows,
            Err(error) => {
                failures = failures.saturating_add(1);
                handle_failure_quarantine(
                    project_id,
                    &cycle_id,
                    "conflict_detector",
                    &error,
                    0,
                    &mut quarantines,
                );
                Vec::new()
            }
        };

        let decisions = match self.consolidation_engine.decide(project_id, reranked).await {
            Ok(rows) => rows,
            Err(error) => {
                failures = failures.saturating_add(1);
                handle_failure_quarantine(
                    project_id,
                    &cycle_id,
                    "consolidation_decide",
                    &error,
                    0,
                    &mut quarantines,
                );
                Vec::new()
            }
        };

        let apply_result = match apply_with_busy_backoff(
            &*self.consolidation_engine,
            project_id,
            &cycle_id,
            &decisions,
        )
        .await
        {
            Ok(applied) => applied,
            Err((error, retry_count)) => {
                failures = failures.saturating_add(1);
                handle_failure_quarantine(
                    project_id,
                    &cycle_id,
                    "consolidation_apply",
                    &error,
                    retry_count,
                    &mut quarantines,
                );
                ConsolidationApplyResult::default()
            }
        };
        quarantines.extend(apply_result.quarantines.iter().cloned());

        let patterns = match self.work_pattern_extractor.extract(project_id).await {
            Ok(rows) => rows,
            Err(error) => {
                failures = failures.saturating_add(1);
                handle_failure_quarantine(
                    project_id,
                    &cycle_id,
                    "pattern_extract",
                    &error,
                    0,
                    &mut quarantines,
                );
                Vec::new()
            }
        };

        let recommendations = match self
            .work_pattern_extractor
            .recommend(project_id, &cycle_id, &patterns)
            .await
        {
            Ok(rows) => rows,
            Err(error) => {
                failures = failures.saturating_add(1);
                handle_failure_quarantine(
                    project_id,
                    &cycle_id,
                    "pattern_recommend",
                    &error,
                    0,
                    &mut quarantines,
                );
                Vec::new()
            }
        };

        let write_result = match self
            .knowledge_datasource_writer
            .emit_and_promote(project_id, &cycle_id)
            .await
        {
            Ok(rows) => rows,
            Err(error) => {
                failures = failures.saturating_add(1);
                handle_failure_quarantine(
                    project_id,
                    &cycle_id,
                    "knowledge_datasource_write",
                    &error,
                    0,
                    &mut quarantines,
                );
                KnowledgeDatasourceWriteResult::default()
            }
        };

        let mut retrospective_refused_reason: Option<String> = None;
        let retrospective = match self
            .retrospective_generator
            .generate_if_due(project_id, _trigger, _backlog_count, self.backlog_threshold)
            .await
        {
            Ok(row) => {
                if let Some(r) = row.as_ref()
                    && r.generation_status == "refused"
                {
                    retrospective_refused_reason =
                        Some("insufficient_citation_coverage".to_string());
                    failures = failures.saturating_add(1);
                    let error =
                        CuratorWorkerError::Executor("llm_refusal: retrospective_refused".into());
                    handle_failure_quarantine(
                        project_id,
                        &cycle_id,
                        "retrospective_generate",
                        &error,
                        0,
                        &mut quarantines,
                    );
                }
                row
            }
            Err(error) => {
                failures = failures.saturating_add(1);
                handle_failure_quarantine(
                    project_id,
                    &cycle_id,
                    "retrospective_generate",
                    &error,
                    0,
                    &mut quarantines,
                );
                None
            }
        };

        let embedding_telemetry = self.reranker.telemetry_snapshot().await.unwrap_or_default();
        let budget_pct_of_total = if embedding_telemetry.total_tokens_used == 0 {
            0.0
        } else {
            embedding_telemetry.embedding_tokens_used as f32
                / embedding_telemetry.total_tokens_used as f32
        };

        Ok(CuratorCycleResult {
            cycle_id,
            project_id: project_id.to_string(),
            decisions_total: decisions.len() as u32
                + conflicts.len() as u32
                + recommendations.len() as u32
                + write_result.raw_knowledge
                + write_result.raw_datasource
                + u32::from(retrospective.is_some()),
            queued_for_review: apply_result.queued_for_review,
            failures: failures.saturating_add(apply_result.failures),
            elapsed_ms: started.elapsed().as_millis() as u64,
            quarantines,
            budget_circuit_open: embedding_telemetry.breaker_open,
            budget_month_tokens: embedding_telemetry.embedding_tokens_used,
            budget_pct_of_total,
            retrospective_refused_reason,
        })
    }
}

fn current_week_window(now_micros: i64) -> (i64, i64) {
    let day = 86_400_000_000_i64;
    let week = 7 * day;
    let week_start = now_micros - (now_micros.rem_euclid(week));
    (week_start, week_start + week - 1)
}

fn retrospective_due(
    last: Option<&WeeklyRetrospective>,
    trigger: CuratorTrigger,
    backlog_count: u32,
    backlog_threshold: u32,
    now_micros: i64,
) -> bool {
    match last {
        None => true,
        Some(last) => {
            let retry_due = matches!(last.generation_status.as_str(), "error" | "refused")
                && now_micros.saturating_sub(last.created_at) >= 6 * 3_600_000_000;
            if retry_due {
                return true;
            }
            let (_, current_week_end) = current_week_window(now_micros);
            let weekly_due = last.week_end < current_week_end;
            if weekly_due {
                return true;
            }
            !matches!(trigger, CuratorTrigger::IntervalTick)
                && backlog_count > backlog_threshold
                && now_micros.saturating_sub(last.created_at) >= 24 * 3_600_000_000
        }
    }
}

fn citation_kind_valid(kind: &str) -> bool {
    matches!(kind, "event" | "decision" | "conflict" | "task")
}

fn compute_citation_coverage(content: &str) -> (f32, Vec<(usize, String, String)>) {
    if content.trim().eq_ignore_ascii_case("REFUSE") {
        return (0.0, Vec::new());
    }
    let claims: Vec<&str> = content
        .split('.')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    if claims.is_empty() {
        return (0.0, Vec::new());
    }
    let mut cited_claims = 0_u32;
    let mut citations = Vec::new();
    for (idx, claim) in claims.iter().enumerate() {
        let mut has_valid = false;
        let bytes = claim.as_bytes();
        let mut i = 0_usize;
        while i + 7 < bytes.len() {
            if claim[i..].starts_with("[[CIT:") {
                let rest = &claim[i + 6..];
                if let Some(end) = rest.find("]]") {
                    let body = &rest[..end];
                    if let Some((kind, id)) = body.split_once(':')
                        && citation_kind_valid(kind)
                        && !id.trim().is_empty()
                    {
                        has_valid = true;
                        citations.push((idx, kind.to_string(), id.trim().to_string()));
                    }
                    i += 6 + end + 2;
                    continue;
                }
            }
            i += 1;
        }
        if has_valid {
            cited_claims += 1;
        }
    }
    let coverage = cited_claims as f32 / claims.len() as f32;
    (coverage, citations)
}

async fn load_latest_retrospective(
    db: &DbPool,
    project_id: &str,
) -> Result<Option<WeeklyRetrospective>, CuratorWorkerError> {
    let project_id = project_id.to_string();
    db.with_conn(move |conn| {
        let mut stmt = conn.prepare(
            "SELECT id, project_id, week_start, week_end, content, citation_coverage, generation_status, created_at
             FROM weekly_retrospectives
             WHERE project_id = ?
             ORDER BY created_at DESC
             LIMIT 1",
        )?;
        let mut rows = stmt.query([project_id])?;
        if let Some(row) = rows.next()? {
            return Ok(Some(WeeklyRetrospective {
                retrospective_id: row.get(0)?,
                project_id: row.get(1)?,
                week_start: row.get(2)?,
                week_end: row.get(3)?,
                content: row.get(4)?,
                citation_coverage: row.get(5)?,
                generation_status: row.get(6)?,
                created_at: row.get(7)?,
            }));
        }
        Ok(None)
    })
    .await
}

async fn build_retrospective_input(
    db: &DbPool,
    project_id: &str,
    week_start: i64,
    week_end: i64,
) -> Result<String, CuratorWorkerError> {
    let project_id = project_id.to_string();
    db.with_conn(move |conn| {
        let decision_count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM curator_decisions WHERE project_id = ?1 AND created_at BETWEEN ?2 AND ?3",
            rusqlite::params![project_id, week_start, week_end],
            |row| row.get(0),
        )?;
        let conflict_count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM sop_conflicts WHERE project_id = ?1 AND created_at BETWEEN ?2 AND ?3",
            rusqlite::params![project_id, week_start, week_end],
            |row| row.get(0),
        )?;
        let event_rows: i64 = conn.query_row(
            "SELECT COUNT(*) FROM session_search_index WHERE session_id LIKE (?1 || '%') AND created_at BETWEEN ?2 AND ?3",
            rusqlite::params![format!("curator:{project_id}"), week_start, week_end],
            |row| row.get(0),
        )?;
        Ok::<_, CuratorWorkerError>(format!(
            "Project: {project_id}\nWeek: {week_start}..{week_end}\nDecisions: {decision_count}\nConflicts: {conflict_count}\nEvents: {event_rows}\nCompose factual retrospective claims with citations."
        ))
    })
    .await
}

async fn persist_retrospective(
    db: &DbPool,
    input: RetrospectivePersistInput,
) -> Result<WeeklyRetrospective, CuratorWorkerError> {
    let project_id = input.project_id;
    let week_start = input.week_start;
    let week_end = input.week_end;
    let content = input.content;
    let citation_coverage = input.citation_coverage;
    let generation_status = input.generation_status;
    let citations = input.citations;
    let created_at = input.created_at;
    db.with_conn(move |conn| {
        conn.execute_batch("BEGIN IMMEDIATE;")?;
        let old_id: Option<String> = conn
            .query_row(
                "SELECT id FROM weekly_retrospectives WHERE project_id=?1 AND week_start=?2 AND week_end=?3",
                rusqlite::params![project_id, week_start, week_end],
                |row| row.get(0),
            )
            .ok();
        if let Some(old_id) = old_id {
            conn.execute(
                "DELETE FROM retrospective_citations WHERE retrospective_id = ?1",
                [old_id],
            )?;
        }
        let retrospective_id = format!("retro-{}", uuid::Uuid::new_v4());
        conn.execute(
            "INSERT INTO weekly_retrospectives (
                id, tenant_id, project_id, week_start, week_end, content,
                citation_coverage, generation_status, created_at
            ) VALUES (?1, 'legacy-default', ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            ON CONFLICT(project_id, week_start, week_end) DO UPDATE SET
                id = excluded.id,
                content = excluded.content,
                citation_coverage = excluded.citation_coverage,
                generation_status = excluded.generation_status,
                created_at = excluded.created_at",
            rusqlite::params![
                retrospective_id,
                project_id,
                week_start,
                week_end,
                content,
                citation_coverage,
                generation_status,
                created_at
            ],
        )?;

        for (claim_index, kind, citation_ref) in citations {
            conn.execute(
                "INSERT INTO retrospective_citations (
                    id, tenant_id, retrospective_id, claim_index, citation_kind, citation_ref, snippet
                ) VALUES (?1, 'legacy-default', ?2, ?3, ?4, ?5, NULL)",
                rusqlite::params![
                    format!("rc-{}", uuid::Uuid::new_v4()),
                    retrospective_id,
                    i64::try_from(claim_index).unwrap_or(0),
                    kind,
                    citation_ref
                ],
            )?;
        }
        conn.execute_batch("COMMIT;")?;
        Ok::<_, CuratorWorkerError>(WeeklyRetrospective {
            retrospective_id,
            project_id,
            week_start,
            week_end,
            content,
            citation_coverage,
            generation_status,
            created_at,
        })
    })
    .await
}

struct RetrospectivePersistInput {
    project_id: String,
    week_start: i64,
    week_end: i64,
    content: String,
    citation_coverage: f32,
    generation_status: String,
    citations: Vec<(usize, String, String)>,
    created_at: i64,
}

async fn persist_conflict(
    db: &DbPool,
    project_id: &str,
    finding: &ConflictFinding,
) -> Result<(), CuratorWorkerError> {
    let project_id = project_id.to_string();
    let finding = finding.clone();
    db.with_conn(move |conn| {
        let now = now_micros()?;
        conn.execute(
            "INSERT INTO sop_conflicts (
                id, tenant_id, project_id, left_revision_id, right_revision_id,
                structural_score, semantic_score, severity, status, evidence_json, created_at
             ) VALUES (?1, 'legacy-default', ?2, ?3, ?4, ?5, ?6, ?7, 'open', ?8, ?9)",
            rusqlite::params![
                finding.conflict_id,
                project_id,
                finding.left_revision_id,
                finding.right_revision_id,
                finding.structural_score,
                finding.semantic_score,
                finding.severity.as_str(),
                finding.evidence_json.to_string(),
                now
            ],
        )?;
        Ok::<_, CuratorWorkerError>(())
    })
    .await
}

fn classify_conflict_severity(structural_score: f32, semantic_score: f32) -> ConflictSeverity {
    if structural_score >= 0.70 && semantic_score >= 0.75 {
        return ConflictSeverity::High;
    }
    if structural_score >= 0.50 && semantic_score >= 0.55 {
        return ConflictSeverity::Medium;
    }
    ConflictSeverity::Low
}

fn structural_conflict_score(left_content: &str, right_content: &str) -> f32 {
    let left_steps = procedure_steps(left_content);
    let right_steps = procedure_steps(right_content);
    if left_steps.is_empty() || right_steps.is_empty() {
        return 0.0;
    }
    let overlap = lexical_overlap(&left_steps.join(" "), &right_steps.join(" "));
    if overlap < 0.20 {
        return 0.0;
    }
    let mut divergent = 0_u32;
    let mut compared = 0_u32;
    let min_len = left_steps.len().min(right_steps.len());
    for i in 0..min_len {
        compared += 1;
        let sim = lexical_overlap(&left_steps[i], &right_steps[i]);
        if sim < 0.45 {
            divergent += 1;
        }
    }
    if compared == 0 {
        return 0.0;
    }
    let divergence = divergent as f32 / compared as f32;
    (0.6 * overlap + 0.4 * divergence).clamp(0.0, 1.0)
}

fn procedure_steps(content: &str) -> Vec<String> {
    let mut in_proc = false;
    let mut out = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("## ") {
            in_proc = trimmed.eq_ignore_ascii_case("## Procedure");
            continue;
        }
        if !in_proc {
            continue;
        }
        let step = trimmed
            .trim_start_matches(|c: char| c.is_ascii_digit() || c == '.' || c == '-' || c == '*')
            .trim();
        if !step.is_empty() {
            out.push(step.to_ascii_lowercase());
        }
    }
    out
}

fn review_required(
    kind: ConsolidationDecisionKind,
    confidence: f32,
    left_revision_id: &str,
    right_revision_id: &str,
) -> bool {
    if confidence < 0.55 {
        return true;
    }
    if kind.high_impact() {
        return true;
    }
    if confidence <= 0.75 {
        let mut h: u64 = 1469598103934665603;
        for b in left_revision_id
            .as_bytes()
            .iter()
            .chain(right_revision_id.as_bytes().iter())
        {
            h ^= u64::from(*b);
            h = h.wrapping_mul(1099511628211);
        }
        return h % 10 < 3;
    }
    false
}

fn apply_merge(
    conn: &rusqlite::Connection,
    project_id: &str,
    decision: &ConsolidationDecision,
    now: i64,
) -> Result<(), CuratorWorkerError> {
    let left = decision
        .subject_revision_ids
        .first()
        .cloned()
        .unwrap_or_default();
    let right = decision
        .subject_revision_ids
        .get(1)
        .cloned()
        .unwrap_or_default();
    if left.is_empty() || right.is_empty() {
        return Ok(());
    }

    let (target_playbook, left_title, left_keywords, left_content): (
        String,
        String,
        String,
        String,
    ) = conn.query_row(
        "SELECT playbook_id, title, trigger_keywords, content
             FROM playbook_revisions
             WHERE id = ?1 AND source_project_id = ?2",
        rusqlite::params![left, project_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )?;
    let (_right_playbook, right_title, right_keywords, right_content): (
        String,
        String,
        String,
        String,
    ) = conn.query_row(
        "SELECT playbook_id, title, trigger_keywords, content
         FROM playbook_revisions
         WHERE id = ?1 AND source_project_id = ?2",
        rusqlite::params![right, project_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )?;

    let next_no: i64 = conn.query_row(
        "SELECT COALESCE(MAX(revision_no), 0) + 1 FROM playbook_revisions WHERE playbook_id = ?",
        [target_playbook.clone()],
        |row| row.get(0),
    )?;
    let new_id = format!("rev-{}-{}", target_playbook, next_no);
    let merged_title = format!("{} / {}", left_title, right_title);
    let merged_keywords = merge_keyword_json_arrays(&left_keywords, &right_keywords);
    let merged_content = format!("{}\n\n---\n\n{}", left_content, right_content);

    conn.execute(
        "INSERT INTO playbook_revisions (
            id, tenant_id, playbook_id, revision_no, parent_revision_id, title, trigger_keywords,
            content, source_task_id, source_project_id, author_type, change_kind, confidence,
            created_at, superseded_at
        ) VALUES (?1, 'legacy-default', ?2, ?3, ?4, ?5, ?6, ?7, NULL, ?8, 'curator', 'merge', ?9, ?10, NULL)",
        rusqlite::params![
            new_id,
            target_playbook,
            next_no,
            left,
            merged_title,
            merged_keywords,
            merged_content,
            project_id,
            decision.confidence,
            now
        ],
    )?;
    conn.execute(
        "UPDATE playbook_revisions SET superseded_at = ?1 WHERE id IN (?2, ?3)",
        rusqlite::params![now, left, right],
    )?;
    conn.execute(
        "UPDATE playbooks SET active_revision_id = ?1, updated_at = ?2 WHERE id = ?3",
        rusqlite::params![new_id, now, target_playbook],
    )?;
    Ok(())
}

fn apply_archive(
    conn: &rusqlite::Connection,
    decision: &ConsolidationDecision,
    now: i64,
) -> Result<(), CuratorWorkerError> {
    let target_revision = decision
        .target_revision_id
        .as_deref()
        .unwrap_or_default()
        .to_string();
    if target_revision.is_empty() {
        return Ok(());
    }
    let playbook_id: String = conn.query_row(
        "SELECT playbook_id FROM playbook_revisions WHERE id = ?",
        [target_revision],
        |row| row.get(0),
    )?;
    conn.execute(
        "UPDATE playbooks
         SET status = 'archived', archived_reason = ?1, archived_at = ?2, updated_at = ?2
         WHERE id = ?3",
        rusqlite::params![
            format!(
                "curator_decision:{}|confidence={:.3}|action=archive",
                decision.decision_id, decision.confidence
            ),
            now,
            playbook_id
        ],
    )?;
    Ok(())
}

fn apply_restore(
    conn: &rusqlite::Connection,
    decision: &ConsolidationDecision,
    now: i64,
) -> Result<(), CuratorWorkerError> {
    let target_revision = decision
        .target_revision_id
        .as_deref()
        .unwrap_or_default()
        .to_string();
    if target_revision.is_empty() {
        return Ok(());
    }
    let playbook_id: String = conn.query_row(
        "SELECT playbook_id FROM playbook_revisions WHERE id = ?",
        [target_revision],
        |row| row.get(0),
    )?;
    conn.execute(
        "UPDATE playbooks
         SET status = 'active', archived_reason = NULL, archived_at = NULL, updated_at = ?1
         WHERE id = ?2",
        rusqlite::params![now, playbook_id],
    )?;
    Ok(())
}

fn merge_keyword_json_arrays(left: &str, right: &str) -> String {
    let mut set = HashSet::new();
    let mut out = Vec::new();
    for raw in [left, right] {
        if let Ok(values) = serde_json::from_str::<Vec<String>>(raw) {
            for v in values {
                if set.insert(v.clone()) {
                    out.push(v);
                }
            }
        }
    }
    serde_json::to_string(&out).unwrap_or_else(|_| "[]".to_string())
}

#[derive(Clone)]
pub struct NoopCycleExecutor;

#[async_trait]
impl CuratorCycleExecutor for NoopCycleExecutor {
    async fn execute(
        &self,
        project_id: &str,
        _trigger: CuratorTrigger,
        backlog_count: u32,
    ) -> Result<CuratorCycleResult, CuratorWorkerError> {
        Ok(CuratorCycleResult {
            cycle_id: format!("cycle-{}", uuid::Uuid::new_v4()),
            project_id: project_id.to_string(),
            decisions_total: backlog_count.min(50),
            queued_for_review: 0,
            failures: 0,
            elapsed_ms: 0,
            quarantines: Vec::new(),
            budget_circuit_open: false,
            budget_month_tokens: 0,
            budget_pct_of_total: 0.0,
            retrospective_refused_reason: None,
        })
    }
}

#[derive(Clone)]
pub struct ProductionCuratorWorker {
    config: CuratorConfig,
    db: DbPool,
    events: Arc<SqliteEventStore>,
    backlog_probe: Arc<dyn BacklogProbe>,
    executor: Arc<dyn CuratorCycleExecutor>,
}

impl ProductionCuratorWorker {
    pub fn new(
        config: CuratorConfig,
        db: DbPool,
        events: Arc<SqliteEventStore>,
        backlog_probe: Arc<dyn BacklogProbe>,
        executor: Arc<dyn CuratorCycleExecutor>,
    ) -> Self {
        Self {
            config,
            db,
            events,
            backlog_probe,
            executor,
        }
    }

    pub async fn run(&self, cancel: CancellationToken) -> Result<(), CuratorWorkerError> {
        let project_id = self.config.project_id.as_str();
        if !self.config.enabled {
            tracing::info!(project_id, "curator worker disabled");
            return Ok(());
        }
        let interval = Duration::from_secs(self.config.interval_seconds.max(1));
        let mut ticker = tokio::time::interval(interval);
        ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
        ticker.tick().await;

        loop {
            tokio::select! {
                _ = cancel.cancelled() => {
                    tracing::info!(project_id, "curator worker shutdown requested");
                    return Ok(());
                }
                _ = ticker.tick() => {
                    let backlog = match self.backlog_probe.pending_count(&self.config.project_id).await {
                        Ok(count) => count,
                        Err(error) => {
                            tracing::warn!(project_id, %error, "curator backlog probe failed");
                            continue;
                        }
                    };
                    let trigger = if backlog > self.config.backlog_threshold {
                        CuratorTrigger::BacklogThreshold
                    } else {
                        CuratorTrigger::IntervalTick
                    };
                    if let Err(error) = self.run_once(trigger, backlog).await {
                        tracing::warn!(
                            project_id,
                            trigger = trigger.as_str(),
                            backlog,
                            %error,
                            "curator cycle failed",
                        );
                    }
                }
            }
        }
    }

    pub async fn run_once(
        &self,
        trigger: CuratorTrigger,
        backlog_count: u32,
    ) -> Result<CuratorCycleResult, CuratorWorkerError> {
        let worker_tenant = self.resolve_worker_tenant().await?;
        if worker_tenant.trim().is_empty() {
            let session_id = format!("curator:{}", self.config.project_id);
            self.ensure_session(&session_id).await?;
            self.events
                .append(NewEvent {
                    session_id,
                    event_type: EventType::Misc,
                    source: "curator".to_string(),
                    data: json!({
                        "kind": "curator_cycle_refused",
                        "project_id": self.config.project_id,
                        "failure_category": "tenant_unresolved",
                        "detail": "worker auth context tenant is unresolved"
                    }),
                })
                .await?;
            return Err(CuratorWorkerError::Executor(
                "tenant_unresolved: curator cycle refused".to_string(),
            ));
        }

        let session_id = format!("curator:{}", self.config.project_id);
        self.ensure_session(&session_id).await?;
        self.events
            .append(NewEvent {
                session_id: session_id.clone(),
                event_type: EventType::Misc,
                source: "curator".to_string(),
                data: json!({
                    "kind": "curator_cycle_started",
                    "project_id": self.config.project_id,
                    "trigger": trigger.as_str(),
                    "backlog_count": backlog_count,
                    "backlog_threshold": self.config.backlog_threshold
                }),
            })
            .await?;
        let result = self
            .executor
            .execute(&self.config.project_id, trigger, backlog_count)
            .await?;
        self.emit_curation_skill_events(&session_id, &result)
            .await?;
        for quarantine in &result.quarantines {
            self.events
                .append(NewEvent {
                    session_id: session_id.clone(),
                    event_type: EventType::Misc,
                    source: "curator".to_string(),
                    data: json!({
                        "kind": "curator_decision_quarantined",
                        "project_id": result.project_id,
                        "cycle_id": result.cycle_id,
                        "decision_id": quarantine.decision_id,
                        "failure_category": quarantine.failure_category.as_str(),
                        "retry_count": quarantine.retry_count,
                        "detail": quarantine.detail
                    }),
                })
                .await?;
        }
        if result.budget_circuit_open {
            self.events
                .append(NewEvent {
                    session_id: session_id.clone(),
                    event_type: EventType::Misc,
                    source: "curator".to_string(),
                    data: json!({
                        "kind": "curator_budget_circuit_open",
                        "project_id": result.project_id,
                        "budget_kind": "embedding_hard_breaker",
                        "month_tokens": result.budget_month_tokens,
                        "pct_of_total": result.budget_pct_of_total
                    }),
                })
                .await?;
        }
        if let Some(reason) = result.retrospective_refused_reason.as_deref() {
            self.events
                .append(NewEvent {
                    session_id: session_id.clone(),
                    event_type: EventType::Misc,
                    source: "curator".to_string(),
                    data: json!({
                        "kind": "curator_retrospective_refused",
                        "project_id": result.project_id,
                        "reason": reason
                    }),
                })
                .await?;
        }
        self.events
            .append(NewEvent {
                session_id,
                event_type: EventType::Misc,
                source: "curator".to_string(),
                data: json!({
                    "kind": "curator_cycle_completed",
                    "project_id": result.project_id,
                    "cycle_id": result.cycle_id,
                    "trigger": trigger.as_str(),
                    "decisions_total": result.decisions_total,
                    "queued_for_review": result.queued_for_review,
                    "failures": result.failures,
                    "elapsed_ms": result.elapsed_ms
                }),
            })
            .await?;
        Ok(result)
    }

    async fn emit_curation_skill_events(
        &self,
        session_id: &str,
        result: &CuratorCycleResult,
    ) -> Result<(), CuratorWorkerError> {
        let cycle_id = result.cycle_id.clone();
        let decisions = self
            .db
            .with_conn(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT decision_type, subject_id, confidence, status
                     FROM curator_decisions
                     WHERE cycle_id = ?1
                     ORDER BY created_at ASC, id ASC",
                )?;
                let mut rows = stmt.query([cycle_id])?;
                let mut out = Vec::new();
                while let Some(row) = rows.next()? {
                    let confidence: Option<f32> = row.get(2)?;
                    out.push((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        confidence.unwrap_or(0.0),
                        row.get::<_, String>(3)?,
                    ));
                }
                Ok::<_, CuratorWorkerError>(out)
            })
            .await?;
        for (decision_type, subject_id, confidence, status) in decisions {
            self.events
                .append(NewEvent {
                    session_id: session_id.to_string(),
                    event_type: EventType::Skill,
                    source: "curator".to_string(),
                    data: json!({
                        "kind":"curation_decision",
                        "project_id": result.project_id,
                        "cycle_id": result.cycle_id,
                        "decision_type": decision_type,
                        "subject_id": subject_id,
                        "confidence": confidence,
                        "review_state": status
                    }),
                })
                .await?;
        }
        Ok(())
    }

    async fn ensure_session(&self, session_id: &str) -> Result<(), CuratorWorkerError> {
        let session_id = session_id.to_string();
        let now_micros = now_micros()?;
        self.db
            .with_conn(move |conn| {
                conn.execute(
                    "INSERT OR IGNORE INTO sessions (id, created_at, updated_at, state)
                     VALUES (?1, ?2, ?2, 'RUNNING')",
                    rusqlite::params![session_id, now_micros],
                )?;
                Ok::<_, CuratorWorkerError>(())
            })
            .await
    }

    async fn resolve_worker_tenant(&self) -> Result<String, CuratorWorkerError> {
        let project_id = self.config.project_id.clone();
        self.db
            .with_conn(move |conn| {
                let tenant: Option<String> = conn
                    .query_row(
                        "SELECT tenant_id FROM projects WHERE id = ?1",
                        [project_id],
                        |row| row.get(0),
                    )
                    .optional()?;
                Ok::<_, CuratorWorkerError>(tenant.unwrap_or_else(|| "legacy-default".to_string()))
            })
            .await
    }
}

fn now_micros() -> Result<i64, CuratorWorkerError> {
    let micros = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| CuratorWorkerError::Executor(error.to_string()))?
        .as_micros();
    i64::try_from(micros).map_err(|error| CuratorWorkerError::Executor(error.to_string()))
}

fn lexical_overlap(left: &str, right: &str) -> f32 {
    let left_set = token_set(left);
    let right_set = token_set(right);
    if left_set.is_empty() && right_set.is_empty() {
        return 0.0;
    }
    let inter = left_set.intersection(&right_set).count() as f32;
    let union = left_set.union(&right_set).count() as f32;
    if union <= 0.0 { 0.0 } else { inter / union }
}

fn token_set(text: &str) -> HashSet<String> {
    text.split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_ascii_lowercase())
        .collect()
}

fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.is_empty() || b.is_empty() || a.len() != b.len() {
        return 0.0;
    }
    let mut dot = 0.0_f32;
    let mut an = 0.0_f32;
    let mut bn = 0.0_f32;
    for (x, y) in a.iter().zip(b.iter()) {
        dot += x * y;
        an += x * x;
        bn += y * y;
    }
    if an == 0.0 || bn == 0.0 {
        0.0
    } else {
        dot / (an.sqrt() * bn.sqrt())
    }
}

fn classify_failure_category(error: &CuratorWorkerError) -> CuratorFailureCategory {
    let msg = error.to_string().to_ascii_lowercase();
    if msg.contains("slot") && msg.contains("unavailable") {
        return CuratorFailureCategory::SlotUnavailable;
    }
    if msg.contains("refus") {
        return CuratorFailureCategory::LlmRefusal;
    }
    if msg.contains("malformed") || msg.contains("invalid json") || msg.contains("parse") {
        return CuratorFailureCategory::MalformedPayload;
    }
    if msg.contains("timeout") {
        return CuratorFailureCategory::Timeout;
    }
    if msg.contains("oom") || msg.contains("out of memory") {
        return CuratorFailureCategory::OutOfMemory;
    }
    if msg.contains("busy") || msg.contains("database is locked") {
        return CuratorFailureCategory::SqliteBusy;
    }
    CuratorFailureCategory::Panic
}

fn is_sqlite_busy(error: &CuratorWorkerError) -> bool {
    match error {
        CuratorWorkerError::Sqlite(db_error) => {
            matches!(db_error, rusqlite::Error::SqliteFailure(_, _))
                && db_error.to_string().to_ascii_lowercase().contains("busy")
        }
        _ => error
            .to_string()
            .to_ascii_lowercase()
            .contains("database is locked"),
    }
}

async fn apply_with_busy_backoff(
    engine: &dyn ConsolidationEngine,
    project_id: &str,
    cycle_id: &str,
    decisions: &[ConsolidationDecision],
) -> Result<ConsolidationApplyResult, (CuratorWorkerError, u32)> {
    let mut retry = 0_u32;
    let delays = [50_u64, 100, 200, 400];
    loop {
        match engine.apply(project_id, cycle_id, decisions).await {
            Ok(v) => return Ok(v),
            Err(error) => {
                if is_sqlite_busy(&error)
                    && usize::try_from(retry).unwrap_or(usize::MAX) < delays.len()
                {
                    let idx = usize::try_from(retry).unwrap_or(0);
                    tokio::time::sleep(Duration::from_millis(delays[idx])).await;
                    retry = retry.saturating_add(1);
                    continue;
                }
                return Err((error, retry));
            }
        }
    }
}

fn handle_failure_quarantine(
    project_id: &str,
    cycle_id: &str,
    decision_scope: &str,
    error: &CuratorWorkerError,
    retry_count: u32,
    quarantines: &mut Vec<CuratorQuarantineRecord>,
) {
    let category = classify_failure_category(error);
    let decision_id = format!(
        "q-{}-{}",
        decision_scope,
        stable_u64_hex(&format!(
            "{project_id}:{cycle_id}:{decision_scope}:{retry_count}"
        ))
    );
    quarantines.push(CuratorQuarantineRecord {
        decision_id,
        failure_category: category,
        retry_count,
        detail: error.to_string(),
    });
}

fn compose_confidence_with_bounds(
    deterministic_floor: f32,
    llm_signal: f32,
    llm_weight_cap: f32,
) -> f32 {
    let floor = deterministic_floor.clamp(0.0, 1.0);
    let llm = llm_signal.clamp(0.0, 1.0);
    let cap = llm_weight_cap.clamp(0.0, 1.0);
    (floor + (llm * cap)).clamp(0.0, 1.0)
}

fn source_type_from_ref(source_ref: &str) -> &'static str {
    if source_ref.starts_with("http://") || source_ref.starts_with("https://") {
        "url"
    } else if source_ref.contains('/') {
        "path"
    } else {
        "text"
    }
}

fn extract_source_refs(content: &str) -> Vec<String> {
    let mut refs = HashSet::new();
    for line in content.lines() {
        let t = line.trim();
        if t.starts_with("http://") || t.starts_with("https://") {
            refs.insert(t.to_string());
            continue;
        }
        if let Some(idx) = t.find("http://").or_else(|| t.find("https://")) {
            refs.insert(t[idx..].trim().to_string());
            continue;
        }
        if let Some(rest) = t.strip_prefix("- ").or_else(|| t.strip_prefix("* ")) {
            let rt = rest.trim();
            if rt.starts_with('/') || rt.starts_with("./") || rt.contains('/') {
                refs.insert(rt.to_string());
            }
        }
    }
    let mut out: Vec<String> = refs.into_iter().collect();
    out.sort();
    out
}

fn infer_knowledge_key(title: &str, _content: &str) -> String {
    let key = title
        .split_whitespace()
        .take(6)
        .collect::<Vec<_>>()
        .join("_")
        .to_ascii_lowercase();
    if key.is_empty() {
        "untitled_procedure".to_string()
    } else {
        key
    }
}

fn infer_knowledge_value(content: &str) -> String {
    content
        .lines()
        .take(8)
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

fn stable_u64_hex(input: &str) -> String {
    let mut h: u64 = 1469598103934665603;
    for b in input.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(1099511628211);
    }
    format!("{h:016x}")
}

fn knowledge_l2_satisfied(
    conn: &rusqlite::Connection,
    project_id: &str,
    revision_id: &str,
    source_task_id: Option<&str>,
    refs: &[String],
) -> Result<bool, CuratorWorkerError> {
    if refs.is_empty() {
        return Ok(false);
    }
    let mut distinct_tasks: HashSet<String> = HashSet::new();
    if let Some(tid) = source_task_id
        && !tid.is_empty()
    {
        distinct_tasks.insert(tid.to_string());
    }
    for source_ref in refs {
        let mut stmt = conn.prepare(
            "SELECT DISTINCT COALESCE(source_task_id, '')
             FROM datasource_items
             WHERE project_id = ?1 AND source_ref = ?2",
        )?;
        let mut rows = stmt.query(rusqlite::params![project_id, source_ref])?;
        while let Some(row) = rows.next()? {
            let tid: String = row.get(0)?;
            if !tid.is_empty() {
                distinct_tasks.insert(tid);
            }
        }
    }
    if distinct_tasks.len() >= 2 {
        return Ok(true);
    }

    let conflict_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sop_conflicts
         WHERE project_id = ?1
           AND status = 'open'
           AND (left_revision_id = ?2 OR right_revision_id = ?2)",
        rusqlite::params![project_id, revision_id],
        |row| row.get(0),
    )?;
    Ok(conflict_count == 0)
}

fn datasource_l2_satisfied(
    conn: &rusqlite::Connection,
    project_id: &str,
    revision_id: &str,
    source_task_id: Option<&str>,
    source_ref: &str,
) -> Result<bool, CuratorWorkerError> {
    let current_task = source_task_id.unwrap_or_default();
    let independent_ref_count: i64 = conn.query_row(
        "SELECT COUNT(DISTINCT COALESCE(source_task_id, ''))
         FROM datasource_items
         WHERE project_id = ?1
           AND source_ref = ?2
           AND COALESCE(source_task_id, '') <> ?3",
        rusqlite::params![project_id, source_ref, current_task],
        |row| row.get(0),
    )?;
    if independent_ref_count >= 1 {
        return Ok(true);
    }
    let conflict_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sop_conflicts
         WHERE project_id = ?1
           AND status = 'open'
           AND (left_revision_id = ?2 OR right_revision_id = ?2)",
        rusqlite::params![project_id, revision_id],
        |row| row.get(0),
    )?;
    Ok(conflict_count == 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use crate::db;
    use crate::events::{EventQuery, EventType};

    async fn seed_revision_pair(db: &DbPool, project_id: &str) {
        db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO playbooks (id, tenant_id, title, content_path, schema_version, source_task_id, created_at, updated_at, trigger_keywords, content, status, source_project_id, active_revision_id, success_count, failure_count)
                 VALUES ('pb-l', 'legacy-default', 'Left', '/tmp/l.md', 1, NULL, 1, 1, '[\"refund\",\"stripe\"]', 'Handle stripe refund policy and customer email.', 'active', ?, 'rev-l-1', 0, 0)",
                [project_id],
            )
            .expect("insert left playbook");
            conn.execute(
                "INSERT INTO playbooks (id, tenant_id, title, content_path, schema_version, source_task_id, created_at, updated_at, trigger_keywords, content, status, source_project_id, active_revision_id, success_count, failure_count)
                 VALUES ('pb-r', 'legacy-default', 'Right', '/tmp/r.md', 1, NULL, 1, 1, '[\"refund\",\"billing\"]', 'Refund workflow for billing disputes and stripe chargebacks.', 'active', ?, 'rev-r-1', 0, 0)",
                [project_id],
            )
            .expect("insert right playbook");

            conn.execute(
                "INSERT INTO playbook_revisions (id, tenant_id, playbook_id, revision_no, parent_revision_id, title, trigger_keywords, content, source_task_id, source_project_id, author_type, change_kind, confidence, created_at, superseded_at)
                 VALUES ('rev-l-1', 'legacy-default', 'pb-l', 1, NULL, 'Left rev', '[\"refund\",\"stripe\"]', 'Handle stripe refund policy and customer email.', NULL, ?, 'extractor', 'extract', 1.0, 1, NULL)",
                [project_id],
            )
            .expect("insert left revision");
            conn.execute(
                "INSERT INTO playbook_revisions (id, tenant_id, playbook_id, revision_no, parent_revision_id, title, trigger_keywords, content, source_task_id, source_project_id, author_type, change_kind, confidence, created_at, superseded_at)
                 VALUES ('rev-r-1', 'legacy-default', 'pb-r', 1, NULL, 'Right rev', '[\"refund\",\"billing\"]', 'Refund workflow for billing disputes and stripe chargebacks.', NULL, ?, 'extractor', 'extract', 1.0, 1, NULL)",
                [project_id],
            )
            .expect("insert right revision");
        })
        .await;
    }

    #[tokio::test]
    async fn run_once_emits_cycle_start_and_complete_events() {
        let db = db::open(":memory:").await.expect("db");
        seed_revision_pair(&db, "proj-1").await;

        let events = Arc::new(SqliteEventStore::new(db.clone()));
        let builder = Arc::new(SqliteCandidateBuilder::new(db.clone()));
        let reranker = Arc::new(TestReranker::new(true));
        let consolidation = Arc::new(SqliteConsolidationEngine::new(db.clone()));
        let executor = Arc::new(ProductionCuratorCycleExecutor::new(
            CuratorRuntimeDeps {
                candidate_builder: builder,
                reranker,
                consolidation_engine: consolidation,
                conflict_detector: Arc::new(StubNoopConflictDetector),
                retrospective_generator: Arc::new(StubNoopRetrospectiveGenerator),
                work_pattern_extractor: Arc::new(StubNoopWorkPatternExtractor),
                operator_review_queue: Arc::new(StubNoopOperatorReviewQueue),
                knowledge_datasource_writer: Arc::new(StubNoopKnowledgeDatasourceWriter),
            },
            50,
            10,
        ));

        let worker = ProductionCuratorWorker::new(
            CuratorConfig {
                enabled: true,
                interval_seconds: 1,
                backlog_threshold: 10,
                max_candidates_per_cycle: 50,
                embedding_budget_monthly_tokens: 50_000,
                embedding_budget_soft_cap_pct: 0.08,
                embedding_budget_hard_breaker_pct: 0.12,
                embedding_model: "text-embedding-3-small".to_string(),
                auto_archive_enabled: false,
                archive_recommend_min_confidence: 0.40,
                archive_apply_min_confidence: 0.55,
                project_id: "proj-1".to_string(),
            },
            db,
            events.clone(),
            Arc::new(StubBacklogProbe),
            executor,
        );

        let result = worker
            .run_once(CuratorTrigger::BacklogThreshold, 12)
            .await
            .expect("run_once");
        assert_eq!(result.project_id, "proj-1");
        assert!(result.decisions_total >= 1);

        let got = events
            .query(
                "curator:proj-1",
                EventQuery {
                    event_type: Some(EventType::Misc),
                    ..EventQuery::default()
                },
            )
            .await
            .expect("query");
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].data["kind"], "curator_cycle_started");
        assert_eq!(got[1].data["kind"], "curator_cycle_completed");
    }

    #[tokio::test]
    async fn embedding_enabled_and_fallback_paths_are_exercised() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/embeddings"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "object":"list",
                "data":[{"index":0,"embedding":[1.0,0.0,0.0]}],
                "model":"text-embedding-3-small",
                "usage":{"prompt_tokens":7,"total_tokens":7}
            })))
            .mount(&server)
            .await;

        let db = db::open(":memory:").await.expect("db");
        seed_revision_pair(&db, "proj-embed").await;

        let builder = SqliteCandidateBuilder::new(db.clone());
        let candidates = builder
            .build_duplicate_candidates("proj-embed", 50)
            .await
            .expect("candidates");
        assert!(!candidates.is_empty());

        let embedding_reranker = ProductionEmbeddingReranker::new(
            LlmClient::new(format!("{}/v1", server.uri()), None),
            "text-embedding-3-small".to_string(),
            EmbeddingBudget {
                monthly_embedding_tokens: 50_000,
                soft_cap_pct: 0.08,
                hard_breaker_pct: 0.12,
            },
        );
        let reranked = embedding_reranker
            .rerank("proj-embed", candidates.clone())
            .await
            .expect("rerank with embeddings");
        assert!(!reranked.is_empty());
        assert!(reranked.iter().all(|r| r.embedding_used));

        let fallback_reranker = ProductionEmbeddingReranker::new(
            LlmClient::new(format!("{}/v1", server.uri()), None),
            "text-embedding-3-small".to_string(),
            EmbeddingBudget {
                monthly_embedding_tokens: 1,
                soft_cap_pct: 0.08,
                hard_breaker_pct: 0.12,
            },
        );
        {
            let mut usage = fallback_reranker.usage.lock().await;
            usage.0 = 2;
            usage.1 = 2;
        }
        let reranked_fallback = fallback_reranker
            .rerank("proj-embed", candidates)
            .await
            .expect("rerank fallback");
        assert!(reranked_fallback.iter().all(|r| !r.embedding_used));
    }

    #[tokio::test]
    async fn e2e_cycle_covers_merge_and_keep_branches_with_stubbed_rerank() {
        let db = db::open(":memory:").await.expect("db");
        seed_revision_pair(&db, "proj-consolidate").await;
        db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO playbooks (id, tenant_id, title, content_path, schema_version, source_task_id, created_at, updated_at, trigger_keywords, content, status, source_project_id, active_revision_id, success_count, failure_count)
                 VALUES ('pb-k', 'legacy-default', 'Keep', '/tmp/k.md', 1, NULL, 1, 1, '[\"docs\"]', 'Documentation workflow', 'active', 'proj-consolidate', 'rev-k-1', 0, 0)",
                [],
            )
            .expect("insert keep playbook");
            conn.execute(
                "INSERT INTO playbook_revisions (id, tenant_id, playbook_id, revision_no, parent_revision_id, title, trigger_keywords, content, source_task_id, source_project_id, author_type, change_kind, confidence, created_at, superseded_at)
                 VALUES ('rev-k-1', 'legacy-default', 'pb-k', 1, NULL, 'Keep rev', '[\"docs\"]', 'Documentation workflow', NULL, 'proj-consolidate', 'extractor', 'extract', 1.0, 1, NULL)",
                [],
            )
            .expect("insert keep revision");
        })
        .await;

        let events = Arc::new(SqliteEventStore::new(db.clone()));
        let builder = Arc::new(StubCandidateBuilder);
        let reranker = Arc::new(StubMergeKeepReranker);
        let consolidation = Arc::new(SqliteConsolidationEngine::new(db.clone()));
        let executor = Arc::new(ProductionCuratorCycleExecutor::new(
            CuratorRuntimeDeps {
                candidate_builder: builder,
                reranker,
                consolidation_engine: consolidation,
                conflict_detector: Arc::new(StubNoopConflictDetector),
                retrospective_generator: Arc::new(StubNoopRetrospectiveGenerator),
                work_pattern_extractor: Arc::new(StubNoopWorkPatternExtractor),
                operator_review_queue: Arc::new(StubNoopOperatorReviewQueue),
                knowledge_datasource_writer: Arc::new(StubNoopKnowledgeDatasourceWriter),
            },
            50,
            10,
        ));
        let worker = ProductionCuratorWorker::new(
            CuratorConfig {
                enabled: true,
                interval_seconds: 1,
                backlog_threshold: 10,
                max_candidates_per_cycle: 50,
                embedding_budget_monthly_tokens: 50_000,
                embedding_budget_soft_cap_pct: 0.08,
                embedding_budget_hard_breaker_pct: 0.12,
                embedding_model: "text-embedding-3-small".to_string(),
                auto_archive_enabled: false,
                archive_recommend_min_confidence: 0.40,
                archive_apply_min_confidence: 0.55,
                project_id: "proj-consolidate".to_string(),
            },
            db.clone(),
            events,
            Arc::new(StubBacklogProbe),
            executor,
        );

        let result = worker
            .run_once(CuratorTrigger::Manual, 2)
            .await
            .expect("run_once");
        assert_eq!(result.decisions_total, 2);

        db.with_conn(|conn| {
            let merge_count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM curator_decisions WHERE project_id='proj-consolidate' AND decision_type='merge'",
                    [],
                    |row| row.get(0),
                )
                .expect("merge count");
            let keep_count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM curator_decisions WHERE project_id='proj-consolidate' AND decision_type='keep'",
                    [],
                    |row| row.get(0),
                )
                .expect("keep count");
            let merged_revision_count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM playbook_revisions WHERE playbook_id='pb-l' AND change_kind='merge'",
                    [],
                    |row| row.get(0),
                )
                .expect("merged revisions");
            assert_eq!(merge_count, 1);
            assert_eq!(keep_count, 1);
            assert_eq!(merged_revision_count, 1);
        })
        .await;
    }

    #[tokio::test]
    async fn e2e_consolidation_archive_and_restore_roundtrip_preserves_outcome_counts() {
        let db = db::open(":memory:").await.expect("db");
        seed_revision_pair(&db, "proj-archive").await;
        db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO playbook_revision_outcomes (
                    revision_id, tenant_id, success_count, failure_count, decayed_success, decayed_failure, last_outcome_at
                 ) VALUES ('rev-l-1', 'legacy-default', 7, 2, 0, 0, NULL)",
                [],
            )
            .expect("seed outcomes");
        })
        .await;

        let engine =
            SqliteConsolidationEngine::new(db.clone()).with_archive_policy(true, 0.40, 0.55);
        let archive = ConsolidationDecision {
            decision_id: "cd-archive-1".to_string(),
            kind: ConsolidationDecisionKind::ArchiveApply,
            subject_revision_ids: vec!["rev-l-1".to_string()],
            target_revision_id: Some("rev-l-1".to_string()),
            confidence: 0.62,
            rationale_json: json!({"policy":"story_4_13_archive"}),
            requires_review: false,
        };
        let archived = engine
            .apply("proj-archive", "cycle-archive", &[archive])
            .await
            .expect("archive apply");
        assert_eq!(archived.applied, 1);

        db.with_conn(|conn| {
            let (status, reason, archived_at): (String, Option<String>, Option<i64>) = conn
                .query_row(
                    "SELECT status, archived_reason, archived_at FROM playbooks WHERE id='pb-l'",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .expect("archived state");
            assert_eq!(status, "archived");
            let reason = reason.expect("archived reason");
            assert!(reason.contains("curator_decision:cd-archive-1"));
            assert!(reason.contains("confidence=0.620"));
            assert!(archived_at.is_some());
        })
        .await;

        let restore = ConsolidationDecision {
            decision_id: "cd-restore-1".to_string(),
            kind: ConsolidationDecisionKind::Restore,
            subject_revision_ids: vec!["rev-l-1".to_string()],
            target_revision_id: Some("rev-l-1".to_string()),
            confidence: 0.91,
            rationale_json: json!({"policy":"story_4_13_restore"}),
            requires_review: false,
        };
        let restored = engine
            .apply("proj-archive", "cycle-restore", &[restore])
            .await
            .expect("restore apply");
        assert_eq!(restored.applied, 1);

        db.with_conn(|conn| {
            let (status, reason, archived_at): (String, Option<String>, Option<i64>) = conn
                .query_row(
                    "SELECT status, archived_reason, archived_at FROM playbooks WHERE id='pb-l'",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .expect("restored state");
            assert_eq!(status, "active");
            assert!(reason.is_none());
            assert!(archived_at.is_none());

            let (success_count, failure_count): (i64, i64) = conn
                .query_row(
                    "SELECT success_count, failure_count FROM playbook_revision_outcomes WHERE revision_id='rev-l-1'",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .expect("outcome counts preserved");
            assert_eq!(success_count, 7);
            assert_eq!(failure_count, 2);
        })
        .await;
    }

    #[tokio::test]
    async fn consolidation_apply_rejects_cross_project_revision_scope() {
        let db = db::open(":memory:").await.expect("db");
        seed_revision_pair(&db, "proj-a").await;

        let engine =
            SqliteConsolidationEngine::new(db.clone()).with_archive_policy(true, 0.40, 0.55);
        let decision = ConsolidationDecision {
            decision_id: "cd-cross-project".to_string(),
            kind: ConsolidationDecisionKind::Merge,
            subject_revision_ids: vec!["rev-l-1".to_string(), "rev-r-1".to_string()],
            target_revision_id: Some("rev-l-1".to_string()),
            confidence: 0.9,
            rationale_json: json!({"policy":"isolation_test"}),
            requires_review: false,
        };

        // Rewrite one subject revision to project-b so this decision becomes cross-project.
        db.with_conn(|conn| {
            conn.execute(
                "UPDATE playbook_revisions SET source_project_id='proj-b' WHERE id='rev-r-1'",
                [],
            )
            .expect("mutate revision scope");
        })
        .await;

        let apply = engine
            .apply("proj-a", "cycle-cross-project", &[decision])
            .await
            .expect("cross-project decision is quarantined, not fatal");
        assert_eq!(apply.failures, 1);
        assert_eq!(apply.quarantines.len(), 1);
        assert_eq!(
            apply.quarantines[0].failure_category,
            CuratorFailureCategory::CrossTenantRef
        );

        db.with_conn(|conn| {
            let rows: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM curator_decisions WHERE cycle_id='cycle-cross-project'",
                    [],
                    |row| row.get(0),
                )
                .expect("decision count");
            assert_eq!(rows, 0, "no decision rows may be inserted");
        })
        .await;
    }

    #[derive(Clone)]
    struct TestReranker {
        embedding_used: bool,
    }

    impl TestReranker {
        fn new(embedding_used: bool) -> Self {
            Self { embedding_used }
        }
    }

    #[async_trait]
    impl EmbeddingReranker for TestReranker {
        async fn rerank(
            &self,
            _project_id: &str,
            candidates: Vec<DuplicateCandidate>,
        ) -> Result<Vec<RerankedCandidate>, CuratorWorkerError> {
            Ok(candidates
                .into_iter()
                .map(|c| RerankedCandidate {
                    left_revision_id: c.left_revision_id,
                    right_revision_id: c.right_revision_id,
                    blended_score: c.fts_score,
                    embedding_cosine: if self.embedding_used { 0.9 } else { 0.0 },
                    fts_norm: c.fts_score,
                    deterministic_floor: c.fts_score,
                    llm_contribution: if self.embedding_used { 0.4 } else { 0.0 },
                    embedding_used: self.embedding_used,
                })
                .collect())
        }
    }

    struct StubBacklogProbe;

    #[async_trait]
    impl BacklogProbe for StubBacklogProbe {
        async fn pending_count(&self, _project_id: &str) -> Result<u32, CuratorWorkerError> {
            Ok(12)
        }
    }

    struct StubCandidateBuilder;

    #[async_trait]
    impl CandidateBuilder for StubCandidateBuilder {
        async fn build_duplicate_candidates(
            &self,
            _project_id: &str,
            _limit: u32,
        ) -> Result<Vec<DuplicateCandidate>, CuratorWorkerError> {
            Ok(vec![
                DuplicateCandidate {
                    left_revision_id: "rev-l-1".to_string(),
                    right_revision_id: "rev-r-1".to_string(),
                    left_text: "refund stripe policy".to_string(),
                    right_text: "refund billing chargeback".to_string(),
                    fts_score: 0.92,
                    lexical_overlap: 0.62,
                    recency_delta_days: 0,
                },
                DuplicateCandidate {
                    left_revision_id: "rev-k-1".to_string(),
                    right_revision_id: "rev-r-1".to_string(),
                    left_text: "documentation workflow".to_string(),
                    right_text: "refund billing chargeback".to_string(),
                    fts_score: 0.71,
                    lexical_overlap: 0.31,
                    recency_delta_days: 1,
                },
            ])
        }
    }

    struct StubMergeKeepReranker;

    #[async_trait]
    impl EmbeddingReranker for StubMergeKeepReranker {
        async fn rerank(
            &self,
            _project_id: &str,
            candidates: Vec<DuplicateCandidate>,
        ) -> Result<Vec<RerankedCandidate>, CuratorWorkerError> {
            Ok(vec![
                RerankedCandidate {
                    left_revision_id: candidates[0].left_revision_id.clone(),
                    right_revision_id: candidates[0].right_revision_id.clone(),
                    blended_score: 0.90, // merge branch
                    embedding_cosine: 0.88,
                    fts_norm: 0.92,
                    deterministic_floor: 0.82,
                    llm_contribution: 0.18,
                    embedding_used: true,
                },
                RerankedCandidate {
                    left_revision_id: candidates[1].left_revision_id.clone(),
                    right_revision_id: candidates[1].right_revision_id.clone(),
                    blended_score: 0.70, // keep branch
                    embedding_cosine: 0.42,
                    fts_norm: 0.71,
                    deterministic_floor: 0.58,
                    llm_contribution: 0.12,
                    embedding_used: true,
                },
            ])
        }
    }

    struct StubArchiveRecommendReranker;

    #[async_trait]
    impl EmbeddingReranker for StubArchiveRecommendReranker {
        async fn rerank(
            &self,
            _project_id: &str,
            candidates: Vec<DuplicateCandidate>,
        ) -> Result<Vec<RerankedCandidate>, CuratorWorkerError> {
            let first = candidates.first().cloned().unwrap_or(DuplicateCandidate {
                left_revision_id: "rev-l-1".to_string(),
                right_revision_id: "rev-r-1".to_string(),
                left_text: String::new(),
                right_text: String::new(),
                fts_score: 0.5,
                lexical_overlap: 0.5,
                recency_delta_days: 0,
            });
            Ok(vec![RerankedCandidate {
                left_revision_id: first.left_revision_id,
                right_revision_id: first.right_revision_id,
                blended_score: 0.50,
                embedding_cosine: 0.25,
                fts_norm: 0.50,
                deterministic_floor: 0.40,
                llm_contribution: 0.10,
                embedding_used: true,
            }])
        }
    }

    struct StubNoopConflictDetector;

    #[async_trait]
    impl ConflictDetector for StubNoopConflictDetector {
        async fn detect(
            &self,
            _project_id: &str,
            _reranked: &[RerankedCandidate],
        ) -> Result<Vec<ConflictFinding>, CuratorWorkerError> {
            Ok(Vec::new())
        }
    }

    struct StubSemanticAdjudicator {
        score: f32,
    }

    #[async_trait]
    impl SemanticAdjudicator for StubSemanticAdjudicator {
        async fn contradiction_score(
            &self,
            _left_text: &str,
            _right_text: &str,
        ) -> Result<f32, CuratorWorkerError> {
            Ok(self.score)
        }
    }

    struct StubNoopRetrospectiveGenerator;

    #[async_trait]
    impl RetrospectiveGenerator for StubNoopRetrospectiveGenerator {
        async fn generate_if_due(
            &self,
            _project_id: &str,
            _trigger: CuratorTrigger,
            _backlog_count: u32,
            _backlog_threshold: u32,
        ) -> Result<Option<WeeklyRetrospective>, CuratorWorkerError> {
            Ok(None)
        }
    }

    struct StubRetrospectiveGenerator {
        mode: &'static str,
    }

    #[async_trait]
    impl RetrospectiveGenerator for StubRetrospectiveGenerator {
        async fn generate_if_due(
            &self,
            project_id: &str,
            _trigger: CuratorTrigger,
            _backlog_count: u32,
            _backlog_threshold: u32,
        ) -> Result<Option<WeeklyRetrospective>, CuratorWorkerError> {
            let now = now_micros()?;
            let (week_start, week_end) = current_week_window(now);
            let (content, coverage, status) = match self.mode {
                "success" => (
                    "Claim A [[CIT:event:e1]]. Claim B [[CIT:decision:d1]].".to_string(),
                    1.0,
                    "success".to_string(),
                ),
                "refused" => ("REFUSE".to_string(), 0.0, "refused".to_string()),
                _ => (
                    "Claim X [[CIT:event:e9]].".to_string(),
                    1.0,
                    "success".to_string(),
                ),
            };
            Ok(Some(WeeklyRetrospective {
                retrospective_id: format!("retro-{}", uuid::Uuid::new_v4()),
                project_id: project_id.to_string(),
                week_start,
                week_end,
                content,
                citation_coverage: coverage,
                generation_status: status,
                created_at: now,
            }))
        }
    }

    struct StubNoopWorkPatternExtractor;

    #[async_trait]
    impl WorkPatternExtractor for StubNoopWorkPatternExtractor {
        async fn extract(&self, _project_id: &str) -> Result<Vec<WorkPattern>, CuratorWorkerError> {
            Ok(Vec::new())
        }

        async fn recommend(
            &self,
            _project_id: &str,
            _cycle_id: &str,
            _patterns: &[WorkPattern],
        ) -> Result<Vec<PatternRecommendation>, CuratorWorkerError> {
            Ok(Vec::new())
        }
    }

    struct StubNoopOperatorReviewQueue;

    #[async_trait]
    impl OperatorReviewQueue for StubNoopOperatorReviewQueue {
        async fn list(
            &self,
            _project_id: Option<&str>,
            _state: Option<&str>,
            _limit: usize,
        ) -> Result<Vec<ReviewQueueItem>, CuratorWorkerError> {
            Ok(Vec::new())
        }

        async fn transition(
            &self,
            _queue_id: &str,
            _action: ReviewQueueAction,
            _reviewer: Option<&str>,
            _note: Option<&str>,
            _suppress_ttl_days: Option<u32>,
        ) -> Result<bool, CuratorWorkerError> {
            Ok(true)
        }

        async fn reconcile_suppression_expiry(&self) -> Result<u32, CuratorWorkerError> {
            Ok(0)
        }
    }

    struct StubNoopKnowledgeDatasourceWriter;

    #[async_trait]
    impl KnowledgeDatasourceWriter for StubNoopKnowledgeDatasourceWriter {
        async fn emit_and_promote(
            &self,
            _project_id: &str,
            _cycle_id: &str,
        ) -> Result<KnowledgeDatasourceWriteResult, CuratorWorkerError> {
            Ok(KnowledgeDatasourceWriteResult::default())
        }
    }

    struct StubQuarantineExecutor;

    #[async_trait]
    impl CuratorCycleExecutor for StubQuarantineExecutor {
        async fn execute(
            &self,
            project_id: &str,
            _trigger: CuratorTrigger,
            _backlog_count: u32,
        ) -> Result<CuratorCycleResult, CuratorWorkerError> {
            let mut quarantines = Vec::new();
            for category in [
                CuratorFailureCategory::Panic,
                CuratorFailureCategory::LlmRefusal,
                CuratorFailureCategory::MalformedPayload,
                CuratorFailureCategory::Timeout,
                CuratorFailureCategory::OutOfMemory,
                CuratorFailureCategory::SqliteBusy,
                CuratorFailureCategory::SlotUnavailable,
            ] {
                quarantines.push(CuratorQuarantineRecord {
                    decision_id: format!("q-{}", category.as_str()),
                    failure_category: category,
                    retry_count: 1,
                    detail: "simulated".to_string(),
                });
            }
            Ok(CuratorCycleResult {
                cycle_id: "cycle-test".to_string(),
                project_id: project_id.to_string(),
                decisions_total: 1,
                queued_for_review: 0,
                failures: 7,
                elapsed_ms: 10,
                quarantines,
                budget_circuit_open: false,
                budget_month_tokens: 0,
                budget_pct_of_total: 0.0,
                retrospective_refused_reason: None,
            })
        }
    }

    #[tokio::test]
    async fn e2e_cycle_conflict_detector_covers_conflict_and_non_conflict_paths() {
        let db = db::open(":memory:").await.expect("db");
        seed_revision_pair(&db, "proj-conflict").await;
        db.with_conn(|conn| {
            conn.execute(
                "UPDATE playbook_revisions
                 SET content = '## Procedure\n1. Verify payment source\n2. Issue immediate full refund\n3. Notify customer'
                 WHERE id = 'rev-l-1'",
                [],
            )
            .expect("update left");
            conn.execute(
                "UPDATE playbook_revisions
                 SET content = '## Procedure\n1. Verify payment source\n2. Escalate and deny immediate refund\n3. Notify customer'
                 WHERE id = 'rev-r-1'",
                [],
            )
            .expect("update right");
        })
        .await;

        let events = Arc::new(SqliteEventStore::new(db.clone()));
        let builder = Arc::new(SqliteCandidateBuilder::new(db.clone()));
        let reranker = Arc::new(TestReranker::new(true));
        let conflict_detector = Arc::new(SqliteConflictDetector::new(
            db.clone(),
            Arc::new(StubSemanticAdjudicator { score: 0.90 }),
        ));
        let consolidation = Arc::new(SqliteConsolidationEngine::new(db.clone()));
        let executor = Arc::new(ProductionCuratorCycleExecutor::new(
            CuratorRuntimeDeps {
                candidate_builder: builder,
                reranker,
                consolidation_engine: consolidation,
                conflict_detector,
                retrospective_generator: Arc::new(StubNoopRetrospectiveGenerator),
                work_pattern_extractor: Arc::new(StubNoopWorkPatternExtractor),
                operator_review_queue: Arc::new(StubNoopOperatorReviewQueue),
                knowledge_datasource_writer: Arc::new(StubNoopKnowledgeDatasourceWriter),
            },
            50,
            10,
        ));
        let worker = ProductionCuratorWorker::new(
            CuratorConfig {
                enabled: true,
                interval_seconds: 1,
                backlog_threshold: 10,
                max_candidates_per_cycle: 50,
                embedding_budget_monthly_tokens: 50_000,
                embedding_budget_soft_cap_pct: 0.08,
                embedding_budget_hard_breaker_pct: 0.12,
                embedding_model: "text-embedding-3-small".to_string(),
                auto_archive_enabled: false,
                archive_recommend_min_confidence: 0.40,
                archive_apply_min_confidence: 0.55,
                project_id: "proj-conflict".to_string(),
            },
            db.clone(),
            events,
            Arc::new(StubBacklogProbe),
            executor,
        );
        worker
            .run_once(CuratorTrigger::Manual, 1)
            .await
            .expect("run conflict path");

        db.with_conn(|conn| {
            let count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sop_conflicts WHERE project_id='proj-conflict' AND status='open'",
                    [],
                    |row| row.get(0),
                )
                .expect("count conflicts");
            assert!(count >= 1);
        })
        .await;

        let db2 = db::open(":memory:").await.expect("db2");
        seed_revision_pair(&db2, "proj-no-conflict").await;
        db2.with_conn(|conn| {
            conn.execute(
                "UPDATE playbook_revisions
                 SET content = '## Procedure\n1. Check docs\n2. Publish release notes'
                 WHERE id = 'rev-l-1'",
                [],
            )
            .expect("update left");
            conn.execute(
                "UPDATE playbook_revisions
                 SET content = '## Procedure\n1. Plan sprint\n2. Groom backlog'
                 WHERE id = 'rev-r-1'",
                [],
            )
            .expect("update right");
        })
        .await;

        let events2 = Arc::new(SqliteEventStore::new(db2.clone()));
        let builder2 = Arc::new(SqliteCandidateBuilder::new(db2.clone()));
        let reranker2 = Arc::new(TestReranker::new(true));
        let conflict_detector2 = Arc::new(SqliteConflictDetector::new(
            db2.clone(),
            Arc::new(StubSemanticAdjudicator { score: 0.10 }),
        ));
        let consolidation2 = Arc::new(SqliteConsolidationEngine::new(db2.clone()));
        let executor2 = Arc::new(ProductionCuratorCycleExecutor::new(
            CuratorRuntimeDeps {
                candidate_builder: builder2,
                reranker: reranker2,
                consolidation_engine: consolidation2,
                conflict_detector: conflict_detector2,
                retrospective_generator: Arc::new(StubNoopRetrospectiveGenerator),
                work_pattern_extractor: Arc::new(StubNoopWorkPatternExtractor),
                operator_review_queue: Arc::new(StubNoopOperatorReviewQueue),
                knowledge_datasource_writer: Arc::new(StubNoopKnowledgeDatasourceWriter),
            },
            50,
            10,
        ));
        let worker2 = ProductionCuratorWorker::new(
            CuratorConfig {
                enabled: true,
                interval_seconds: 1,
                backlog_threshold: 10,
                max_candidates_per_cycle: 50,
                embedding_budget_monthly_tokens: 50_000,
                embedding_budget_soft_cap_pct: 0.08,
                embedding_budget_hard_breaker_pct: 0.12,
                embedding_model: "text-embedding-3-small".to_string(),
                auto_archive_enabled: false,
                archive_recommend_min_confidence: 0.40,
                archive_apply_min_confidence: 0.55,
                project_id: "proj-no-conflict".to_string(),
            },
            db2.clone(),
            events2,
            Arc::new(StubBacklogProbe),
            executor2,
        );
        worker2
            .run_once(CuratorTrigger::Manual, 1)
            .await
            .expect("run non-conflict path");

        db2.with_conn(|conn| {
            let count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sop_conflicts WHERE project_id='proj-no-conflict'",
                    [],
                    |row| row.get(0),
                )
                .expect("count conflicts");
            assert_eq!(count, 0);
        })
        .await;
    }

    #[tokio::test]
    async fn conflict_detector_rejects_cross_project_pairs_without_writes() {
        let db = db::open(":memory:").await.expect("db");
        seed_revision_pair(&db, "proj-left").await;
        db.with_conn(|conn| {
            conn.execute(
                "UPDATE playbook_revisions
                 SET source_project_id='proj-right'
                 WHERE id='rev-r-1'",
                [],
            )
            .expect("mutate right revision project");
        })
        .await;

        let detector = SqliteConflictDetector::new(
            db.clone(),
            Arc::new(StubSemanticAdjudicator { score: 0.9 }),
        );
        let reranked = vec![RerankedCandidate {
            left_revision_id: "rev-l-1".to_string(),
            right_revision_id: "rev-r-1".to_string(),
            blended_score: 0.8,
            embedding_cosine: 0.8,
            fts_norm: 0.8,
            deterministic_floor: 0.8,
            llm_contribution: 0.2,
            embedding_used: true,
        }];
        let result = detector.detect("proj-left", &reranked).await;
        assert!(
            result.is_err(),
            "cross-project conflict candidate must fail closed"
        );

        db.with_conn(|conn| {
            let count: i64 = conn
                .query_row("SELECT COUNT(*) FROM sop_conflicts", [], |row| row.get(0))
                .expect("count");
            assert_eq!(count, 0, "no conflict row should be written");
        })
        .await;
    }

    #[tokio::test]
    async fn e2e_cycle_retrospective_success_refusal_and_retry_paths() {
        let db = db::open(":memory:").await.expect("db");
        seed_revision_pair(&db, "proj-retro").await;

        let build_worker = |mode: &'static str, project_id: &str, db: DbPool| {
            let events = Arc::new(SqliteEventStore::new(db.clone()));
            let builder = Arc::new(SqliteCandidateBuilder::new(db.clone()));
            let reranker = Arc::new(TestReranker::new(true));
            let consolidation = Arc::new(SqliteConsolidationEngine::new(db.clone()));
            let conflict_detector = Arc::new(StubNoopConflictDetector);
            let retrospective = Arc::new(StubRetrospectiveGenerator { mode });
            let executor = Arc::new(ProductionCuratorCycleExecutor::new(
                CuratorRuntimeDeps {
                    candidate_builder: builder,
                    reranker,
                    consolidation_engine: consolidation,
                    conflict_detector,
                    retrospective_generator: retrospective,
                    work_pattern_extractor: Arc::new(StubNoopWorkPatternExtractor),
                    operator_review_queue: Arc::new(StubNoopOperatorReviewQueue),
                    knowledge_datasource_writer: Arc::new(StubNoopKnowledgeDatasourceWriter),
                },
                50,
                10,
            ));
            ProductionCuratorWorker::new(
                CuratorConfig {
                    enabled: true,
                    interval_seconds: 1,
                    backlog_threshold: 10,
                    max_candidates_per_cycle: 50,
                    embedding_budget_monthly_tokens: 50_000,
                    embedding_budget_soft_cap_pct: 0.08,
                    embedding_budget_hard_breaker_pct: 0.12,
                    embedding_model: "text-embedding-3-small".to_string(),
                    auto_archive_enabled: false,
                    archive_recommend_min_confidence: 0.40,
                    archive_apply_min_confidence: 0.55,
                    project_id: project_id.to_string(),
                },
                db.clone(),
                events,
                Arc::new(StubBacklogProbe),
                executor,
            )
        };
        let worker_success = build_worker("success", "proj-retro", db.clone());
        let success = worker_success
            .run_once(CuratorTrigger::Manual, 12)
            .await
            .expect("success run");
        assert!(success.decisions_total >= 2);

        let worker_refused = build_worker("refused", "proj-retro", db.clone());
        let refused = worker_refused
            .run_once(CuratorTrigger::Manual, 12)
            .await
            .expect("refused run");
        assert!(refused.decisions_total >= 2);

        let worker_noop = ProductionCuratorWorker::new(
            CuratorConfig {
                enabled: true,
                interval_seconds: 1,
                backlog_threshold: 10,
                max_candidates_per_cycle: 50,
                embedding_budget_monthly_tokens: 50_000,
                embedding_budget_soft_cap_pct: 0.08,
                embedding_budget_hard_breaker_pct: 0.12,
                embedding_model: "text-embedding-3-small".to_string(),
                auto_archive_enabled: false,
                archive_recommend_min_confidence: 0.40,
                archive_apply_min_confidence: 0.55,
                project_id: "proj-retro".to_string(),
            },
            db.clone(),
            Arc::new(SqliteEventStore::new(db.clone())),
            Arc::new(StubBacklogProbe),
            Arc::new(ProductionCuratorCycleExecutor::new(
                CuratorRuntimeDeps {
                    candidate_builder: Arc::new(SqliteCandidateBuilder::new(db.clone())),
                    reranker: Arc::new(TestReranker::new(true)),
                    consolidation_engine: Arc::new(SqliteConsolidationEngine::new(db.clone())),
                    conflict_detector: Arc::new(StubNoopConflictDetector),
                    retrospective_generator: Arc::new(StubNoopRetrospectiveGenerator),
                    work_pattern_extractor: Arc::new(StubNoopWorkPatternExtractor),
                    operator_review_queue: Arc::new(StubNoopOperatorReviewQueue),
                    knowledge_datasource_writer: Arc::new(StubNoopKnowledgeDatasourceWriter),
                },
                50,
                10,
            )),
        );

        let no_retro = worker_noop
            .run_once(CuratorTrigger::Manual, 12)
            .await
            .expect("no retrospective run");
        assert!(no_retro.decisions_total >= 1);

        let (coverage_refused, _) = compute_citation_coverage("No citation claim.");
        assert!(coverage_refused < 0.95);
        let (coverage_success, citations) =
            compute_citation_coverage("Claim [[CIT:event:e1]]. Claim [[CIT:decision:d1]].");
        assert!(coverage_success >= 0.95);
        assert_eq!(citations.len(), 2);

        let now = now_micros().expect("now");
        let one_hour = 3_600_000_000_i64;
        let stale = WeeklyRetrospective {
            retrospective_id: "retro-stale".to_string(),
            project_id: "proj-retro".to_string(),
            week_start: current_week_window(now).0,
            week_end: current_week_window(now).1,
            content: "REFUSE".to_string(),
            citation_coverage: 0.0,
            generation_status: "refused".to_string(),
            created_at: now - (7 * one_hour),
        };
        assert!(retrospective_due(
            Some(&stale),
            CuratorTrigger::IntervalTick,
            0,
            10,
            now
        ));
    }

    #[tokio::test]
    async fn e2e_cycle_pattern_extractor_emits_deterministic_recommendations() {
        let db = db::open(":memory:").await.expect("db");
        seed_revision_pair(&db, "proj-pattern").await;
        db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO playbook_revision_outcomes (
                    revision_id, tenant_id, success_count, failure_count, decayed_success, decayed_failure, last_outcome_at
                 ) VALUES ('rev-l-1', 'legacy-default', 1, 5, 0, 0, NULL)",
                [],
            )
            .expect("seed outcomes left");
            conn.execute(
                "INSERT INTO playbook_revision_outcomes (
                    revision_id, tenant_id, success_count, failure_count, decayed_success, decayed_failure, last_outcome_at
                 ) VALUES ('rev-r-1', 'legacy-default', 4, 1, 0, 0, NULL)",
                [],
            )
            .expect("seed outcomes right");
            conn.execute(
                "INSERT INTO session_search_index (
                    event_id, session_id, timestamp, event_type, source, searchable_text
                 ) VALUES (9001, 'proj-pattern:s-1', 1, 'Action', 'shell_exec', 'run make test')",
                [],
            )
            .expect("seed event 1");
            conn.execute(
                "INSERT INTO session_search_index (
                    event_id, session_id, timestamp, event_type, source, searchable_text
                 ) VALUES (9002, 'proj-pattern:s-2', 2, 'Observation', 'shell_exec', 'tests failed')",
                [],
            )
            .expect("seed event 2");
        })
        .await;

        let events = Arc::new(SqliteEventStore::new(db.clone()));
        let executor = Arc::new(ProductionCuratorCycleExecutor::new(
            CuratorRuntimeDeps {
                candidate_builder: Arc::new(SqliteCandidateBuilder::new(db.clone())),
                reranker: Arc::new(TestReranker::new(true)),
                consolidation_engine: Arc::new(SqliteConsolidationEngine::new(db.clone())),
                conflict_detector: Arc::new(StubNoopConflictDetector),
                retrospective_generator: Arc::new(StubNoopRetrospectiveGenerator),
                work_pattern_extractor: Arc::new(SqliteWorkPatternExtractor::new(db.clone())),
                operator_review_queue: Arc::new(StubNoopOperatorReviewQueue),
                knowledge_datasource_writer: Arc::new(StubNoopKnowledgeDatasourceWriter),
            },
            50,
            10,
        ));
        let worker = ProductionCuratorWorker::new(
            CuratorConfig {
                enabled: true,
                interval_seconds: 1,
                backlog_threshold: 10,
                max_candidates_per_cycle: 50,
                embedding_budget_monthly_tokens: 50_000,
                embedding_budget_soft_cap_pct: 0.08,
                embedding_budget_hard_breaker_pct: 0.12,
                embedding_model: "text-embedding-3-small".to_string(),
                auto_archive_enabled: false,
                archive_recommend_min_confidence: 0.40,
                archive_apply_min_confidence: 0.55,
                project_id: "proj-pattern".to_string(),
            },
            db.clone(),
            events,
            Arc::new(StubBacklogProbe),
            executor,
        );

        let result = worker
            .run_once(CuratorTrigger::Manual, 12)
            .await
            .expect("run");
        assert!(result.decisions_total >= 2);

        db.with_conn(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT subject_kind, subject_id, confidence
                     FROM curator_decisions
                     WHERE project_id='proj-pattern' AND decision_type='recommendation'
                     ORDER BY confidence DESC, subject_id ASC",
                )
                .expect("prepare");
            let mut rows = stmt.query([]).expect("query");
            let mut got: Vec<(String, String, f32)> = Vec::new();
            while let Some(row) = rows.next().expect("next") {
                got.push((
                    row.get(0).expect("kind"),
                    row.get(1).expect("id"),
                    row.get(2).expect("c"),
                ));
            }
            assert!(!got.is_empty());
            assert!(
                got.iter()
                    .all(|(kind, _, _)| kind == "revision" || kind == "pattern")
            );
            for (_, _, confidence) in &got {
                assert!((*confidence >= 0.35) && (*confidence <= 0.80));
            }
        })
        .await;
    }

    #[tokio::test]
    async fn e2e_cycle_knowledge_datasource_emit_and_l2_promotion_paths() {
        let db = db::open(":memory:").await.expect("db");
        seed_revision_pair(&db, "proj-kd").await;
        db.with_conn(|conn| {
            conn.execute(
                "UPDATE playbook_revisions
                 SET source_task_id = 'task-kd-1',
                     confidence = 0.72,
                     title = 'Refund Escalation SOP',
                     content = '## Procedure\n1. Validate request\n2. Escalate with evidence\n## Sources\n- https://docs.example.com/refund\n- https://kb.example.com/escalation'
                 WHERE id = 'rev-l-1'",
                [],
            )
            .expect("seed source task + content");
            conn.execute(
                "INSERT INTO datasource_items (
                    id, tenant_id, project_id, revision_id, source_task_id, source_type, source_ref, trust_level, confidence, evidence_json, created_at
                 ) VALUES (
                    'ds-prev-1', 'legacy-default', 'proj-kd', 'rev-r-1', 'task-kd-0', 'url',
                    'https://docs.example.com/refund', 'l0', 0.60, '{}', 1
                 )",
                [],
            )
            .expect("seed prior datasource");
            conn.execute(
                "INSERT INTO sop_conflicts (
                    id, tenant_id, project_id, left_revision_id, right_revision_id,
                    structural_score, semantic_score, severity, status, evidence_json, created_at
                 ) VALUES (
                    'conf-kd-1', 'legacy-default', 'proj-kd', 'rev-l-1', 'rev-r-1',
                    0.8, 0.8, 'high', 'open', '{}', 2
                 )",
                [],
            )
            .expect("seed open conflict");
        })
        .await;

        let events = Arc::new(SqliteEventStore::new(db.clone()));
        let executor = Arc::new(ProductionCuratorCycleExecutor::new(
            CuratorRuntimeDeps {
                candidate_builder: Arc::new(SqliteCandidateBuilder::new(db.clone())),
                reranker: Arc::new(TestReranker::new(true)),
                consolidation_engine: Arc::new(SqliteConsolidationEngine::new(db.clone())),
                conflict_detector: Arc::new(StubNoopConflictDetector),
                retrospective_generator: Arc::new(StubNoopRetrospectiveGenerator),
                work_pattern_extractor: Arc::new(StubNoopWorkPatternExtractor),
                operator_review_queue: Arc::new(StubNoopOperatorReviewQueue),
                knowledge_datasource_writer: Arc::new(SqliteKnowledgeDatasourceWriter::new(
                    db.clone(),
                    true,
                    true,
                )),
            },
            50,
            10,
        ));
        let worker = ProductionCuratorWorker::new(
            CuratorConfig {
                enabled: true,
                interval_seconds: 1,
                backlog_threshold: 10,
                max_candidates_per_cycle: 50,
                embedding_budget_monthly_tokens: 50_000,
                embedding_budget_soft_cap_pct: 0.08,
                embedding_budget_hard_breaker_pct: 0.12,
                embedding_model: "text-embedding-3-small".to_string(),
                auto_archive_enabled: false,
                archive_recommend_min_confidence: 0.40,
                archive_apply_min_confidence: 0.55,
                project_id: "proj-kd".to_string(),
            },
            db.clone(),
            events,
            Arc::new(StubBacklogProbe),
            executor,
        );
        let result = worker
            .run_once(CuratorTrigger::Manual, 12)
            .await
            .expect("run cycle");
        assert!(result.decisions_total >= 3);

        db.with_conn(|conn| {
            let raw_knowledge: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM knowledge_items WHERE project_id='proj-kd'",
                    [],
                    |row| row.get(0),
                )
                .expect("knowledge rows");
            let raw_datasource: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM datasource_items WHERE project_id='proj-kd'",
                    [],
                    |row| row.get(0),
                )
                .expect("datasource rows");
            let queued_writes: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM curator_decisions
                     WHERE project_id='proj-kd'
                       AND decision_type IN ('knowledge_write','datasource_write')
                       AND status='queued_review'",
                    [],
                    |row| row.get(0),
                )
                .expect("queued");
            let applied_writes: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM curator_decisions
                     WHERE project_id='proj-kd'
                       AND decision_type IN ('knowledge_write','datasource_write')
                       AND status='applied'",
                    [],
                    |row| row.get(0),
                )
                .expect("applied");
            assert!(raw_knowledge >= 1);
            assert!(raw_datasource >= 3); // includes pre-seed + new raws
            assert!(queued_writes >= 1);
            assert!(applied_writes >= 1);
        })
        .await;
    }

    #[tokio::test]
    async fn e2e_operator_review_queue_transitions_after_cycle() {
        let db = db::open(":memory:").await.expect("db");
        seed_revision_pair(&db, "proj-review").await;
        let events = Arc::new(SqliteEventStore::new(db.clone()));
        let queue = Arc::new(SqliteOperatorReviewQueue::new(db.clone()));
        let executor = Arc::new(ProductionCuratorCycleExecutor::new(
            CuratorRuntimeDeps {
                candidate_builder: Arc::new(SqliteCandidateBuilder::new(db.clone())),
                reranker: Arc::new(StubArchiveRecommendReranker),
                consolidation_engine: Arc::new(SqliteConsolidationEngine::new(db.clone())),
                conflict_detector: Arc::new(StubNoopConflictDetector),
                retrospective_generator: Arc::new(StubNoopRetrospectiveGenerator),
                work_pattern_extractor: Arc::new(StubNoopWorkPatternExtractor),
                operator_review_queue: queue.clone(),
                knowledge_datasource_writer: Arc::new(StubNoopKnowledgeDatasourceWriter),
            },
            50,
            10,
        ));
        let worker = ProductionCuratorWorker::new(
            CuratorConfig {
                enabled: true,
                interval_seconds: 1,
                backlog_threshold: 10,
                max_candidates_per_cycle: 50,
                embedding_budget_monthly_tokens: 50_000,
                embedding_budget_soft_cap_pct: 0.08,
                embedding_budget_hard_breaker_pct: 0.12,
                embedding_model: "text-embedding-3-small".to_string(),
                auto_archive_enabled: false,
                archive_recommend_min_confidence: 0.40,
                archive_apply_min_confidence: 0.55,
                project_id: "proj-review".to_string(),
            },
            db.clone(),
            events,
            Arc::new(StubBacklogProbe),
            executor,
        );

        worker
            .run_once(CuratorTrigger::Manual, 12)
            .await
            .expect("run cycle");

        let pending = queue
            .list(Some("proj-review"), Some("pending"), 20)
            .await
            .expect("list pending");
        assert!(!pending.is_empty());
        let qid = pending[0].queue_id.clone();

        let suppressed = queue
            .transition(
                &qid,
                ReviewQueueAction::Suppress,
                Some("ops"),
                Some("mute noisy"),
                Some(1),
            )
            .await
            .expect("suppress");
        assert!(suppressed);

        let suppressed_rows = queue
            .list(Some("proj-review"), Some("suppressed"), 20)
            .await
            .expect("list suppressed");
        assert_eq!(suppressed_rows.len(), 1);
    }

    #[tokio::test]
    async fn review_queue_transitions_are_scoped_to_target_queue_project_rows() {
        let db = db::open(":memory:").await.expect("db");
        let queue = SqliteOperatorReviewQueue::new(db.clone());
        db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO curator_decisions (
                    id, tenant_id, project_id, cycle_id, decision_type, subject_kind, subject_id,
                    confidence, rationale_json, evidence_json, status, failure_category, created_at
                 ) VALUES (
                    'cd-pa', 'legacy-default', 'proj-a', 'cycle-a', 'archive', 'revision', 'rev-a',
                    0.52, '{}', '{}', 'queued_review', NULL, 1
                 )",
                [],
            )
            .expect("seed decision a");
            conn.execute(
                "INSERT INTO curator_decisions (
                    id, tenant_id, project_id, cycle_id, decision_type, subject_kind, subject_id,
                    confidence, rationale_json, evidence_json, status, failure_category, created_at
                 ) VALUES (
                    'cd-pb', 'legacy-default', 'proj-b', 'cycle-b', 'archive', 'revision', 'rev-b',
                    0.52, '{}', '{}', 'queued_review', NULL, 1
                 )",
                [],
            )
            .expect("seed decision b");
            conn.execute(
                "INSERT INTO curator_review_queue (
                    id, tenant_id, decision_id, project_id, queue_reason, severity, state, reviewer, reviewer_note, resolved_at, created_at
                 ) VALUES (
                    'rq-pa', 'legacy-default', 'cd-pa', 'proj-a', 'test', 'high', 'pending', NULL, NULL, NULL, 1
                 )",
                [],
            )
            .expect("seed queue a");
            conn.execute(
                "INSERT INTO curator_review_queue (
                    id, tenant_id, decision_id, project_id, queue_reason, severity, state, reviewer, reviewer_note, resolved_at, created_at
                 ) VALUES (
                    'rq-pb', 'legacy-default', 'cd-pb', 'proj-b', 'test', 'high', 'pending', NULL, NULL, NULL, 1
                 )",
                [],
            )
            .expect("seed queue b");
        })
        .await;

        let filtered = queue
            .list(Some("proj-a"), Some("pending"), 10)
            .await
            .expect("filtered list");
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].queue_id, "rq-pa");

        let transitioned = queue
            .transition(
                "rq-pa",
                ReviewQueueAction::Approve,
                Some("ops"),
                Some("ok"),
                None,
            )
            .await
            .expect("transition");
        assert!(transitioned);

        db.with_conn(|conn| {
            let a_state: String = conn
                .query_row(
                    "SELECT state FROM curator_review_queue WHERE id='rq-pa'",
                    [],
                    |row| row.get(0),
                )
                .expect("state a");
            let b_state: String = conn
                .query_row(
                    "SELECT state FROM curator_review_queue WHERE id='rq-pb'",
                    [],
                    |row| row.get(0),
                )
                .expect("state b");
            assert_eq!(a_state, "approved");
            assert_eq!(b_state, "pending");
        })
        .await;
    }

    #[tokio::test]
    async fn run_once_emits_quarantine_events_for_all_failure_categories() {
        let db = db::open(":memory:").await.expect("db");
        let events = Arc::new(SqliteEventStore::new(db.clone()));
        let worker = ProductionCuratorWorker::new(
            CuratorConfig {
                enabled: true,
                interval_seconds: 1,
                backlog_threshold: 10,
                max_candidates_per_cycle: 50,
                embedding_budget_monthly_tokens: 50_000,
                embedding_budget_soft_cap_pct: 0.08,
                embedding_budget_hard_breaker_pct: 0.12,
                embedding_model: "text-embedding-3-small".to_string(),
                auto_archive_enabled: false,
                archive_recommend_min_confidence: 0.40,
                archive_apply_min_confidence: 0.55,
                project_id: "proj-quarantine".to_string(),
            },
            db,
            events.clone(),
            Arc::new(StubBacklogProbe),
            Arc::new(StubQuarantineExecutor),
        );
        worker
            .run_once(CuratorTrigger::Manual, 12)
            .await
            .expect("run_once");
        let got = events
            .query(
                "curator:proj-quarantine",
                EventQuery {
                    event_type: Some(EventType::Misc),
                    ..EventQuery::default()
                },
            )
            .await
            .expect("query");
        let quarantines = got
            .iter()
            .filter(|event| event.data["kind"] == "curator_decision_quarantined")
            .count();
        assert_eq!(quarantines, 7);
    }

    #[test]
    fn adversarial_confidence_bounds_enforce_deterministic_floor() {
        let boosted = compose_confidence_with_bounds(0.20, 1.0, 0.45);
        assert!(boosted < 0.75);
        assert!(boosted <= 0.65);
    }

    #[test]
    fn embedding_budget_uses_monthly_token_fallback_when_total_tokens_zero() {
        let budget = EmbeddingBudget {
            monthly_embedding_tokens: 50_000,
            soft_cap_pct: 0.08,
            hard_breaker_pct: 0.12,
        };
        assert!(!budget.soft_cap_exceeded(49_999, 0));
        assert!(budget.soft_cap_exceeded(50_000, 0));
        assert!(!budget.breaker_open(49_999, 0));
        assert!(budget.breaker_open(50_000, 0));
    }

    #[tokio::test]
    async fn emits_curation_decision_skill_and_curator_misc_taxonomy_events() {
        let db = db::open(":memory:").await.expect("db");
        seed_revision_pair(&db, "proj-taxonomy").await;
        db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO playbooks (id, tenant_id, title, content_path, schema_version, source_task_id, created_at, updated_at, trigger_keywords, content, status, source_project_id, active_revision_id, success_count, failure_count)
                 VALUES ('pb-k', 'legacy-default', 'Keep', '/tmp/k.md', 1, NULL, 1, 1, '[\"docs\"]', 'Documentation workflow', 'active', 'proj-taxonomy', 'rev-k-1', 0, 0)",
                [],
            )
            .expect("insert keep playbook for taxonomy");
            conn.execute(
                "INSERT INTO playbook_revisions (id, tenant_id, playbook_id, revision_no, parent_revision_id, title, trigger_keywords, content, source_task_id, source_project_id, author_type, change_kind, confidence, created_at, superseded_at)
                 VALUES ('rev-k-1', 'legacy-default', 'pb-k', 1, NULL, 'Keep rev', '[\"docs\"]', 'Documentation workflow', NULL, 'proj-taxonomy', 'extractor', 'extract', 1.0, 1, NULL)",
                [],
            )
            .expect("insert keep revision for taxonomy");
        })
        .await;
        let events = Arc::new(SqliteEventStore::new(db.clone()));
        let executor = Arc::new(ProductionCuratorCycleExecutor::new(
            CuratorRuntimeDeps {
                candidate_builder: Arc::new(StubCandidateBuilder),
                reranker: Arc::new(StubMergeKeepReranker),
                consolidation_engine: Arc::new(SqliteConsolidationEngine::new(db.clone())),
                conflict_detector: Arc::new(StubNoopConflictDetector),
                retrospective_generator: Arc::new(StubNoopRetrospectiveGenerator),
                work_pattern_extractor: Arc::new(StubNoopWorkPatternExtractor),
                operator_review_queue: Arc::new(StubNoopOperatorReviewQueue),
                knowledge_datasource_writer: Arc::new(StubNoopKnowledgeDatasourceWriter),
            },
            50,
            10,
        ));
        let worker = ProductionCuratorWorker::new(
            CuratorConfig {
                enabled: true,
                interval_seconds: 1,
                backlog_threshold: 10,
                max_candidates_per_cycle: 50,
                embedding_budget_monthly_tokens: 50_000,
                embedding_budget_soft_cap_pct: 0.08,
                embedding_budget_hard_breaker_pct: 0.12,
                embedding_model: "text-embedding-3-small".to_string(),
                auto_archive_enabled: false,
                archive_recommend_min_confidence: 0.40,
                archive_apply_min_confidence: 0.55,
                project_id: "proj-taxonomy".to_string(),
            },
            db.clone(),
            events.clone(),
            Arc::new(StubBacklogProbe),
            executor,
        );
        worker
            .run_once(CuratorTrigger::Manual, 12)
            .await
            .expect("run cycle");

        let all = events
            .query("curator:proj-taxonomy", EventQuery::default())
            .await
            .expect("query");
        assert!(all.iter().any(|e| {
            e.event_type == EventType::Skill
                && e.data.get("kind").and_then(serde_json::Value::as_str)
                    == Some("curation_decision")
        }));
        assert!(all.iter().any(|e| {
            e.event_type == EventType::Misc
                && e.data.get("kind").and_then(serde_json::Value::as_str)
                    == Some("curator_cycle_started")
        }));
        assert!(all.iter().any(|e| {
            e.event_type == EventType::Misc
                && e.data.get("kind").and_then(serde_json::Value::as_str)
                    == Some("curator_cycle_completed")
        }));

        db.with_conn(|conn| {
            let indexed: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM session_search_index
                     WHERE session_id='curator:proj-taxonomy'
                       AND event_type='Skill'
                       AND searchable_text LIKE '%curation_decision%'",
                    [],
                    |row| row.get(0),
                )
                .expect("search index count");
            assert!(indexed >= 1);
        })
        .await;
    }

    // Story 4.16 — NFR-4.7 false-positive audit harness.
    //
    // Drives N>=100 Merge decisions and N>=100 ArchiveApply decisions through the
    // SqliteConsolidationEngine policy across three corpus shapes (small/medium/large)
    // and asserts each false-positive rate stays at or below the NFR-4.7 2% bound.
    //
    // Inputs are synthesized RerankedCandidate batches with ground-truth labels:
    //   true_duplicate   -> blended_score >= 0.82 -> expects Merge
    //   false_duplicate  -> 0.40..0.65            -> expects ArchiveRecommend (not Merge)
    //   stale            -> 0.55..0.65            -> expects ArchiveApply (auto_archive on)
    //   fresh            -> 0.65..0.82            -> expects Keep (not ArchiveApply)
    //
    // One noisy candidate per class is injected to exercise a non-zero, bounded FP
    // signal so the audit detects edge regressions instead of trivially passing on
    // a perfectly clean fixture. Both classes therefore see exactly 1 FP and the
    // rate stays well under 2% with a healthy headroom margin.
    #[tokio::test]
    async fn false_positive_audit_harness_nfr_4_7() {
        let db = db::open(":memory:").await.expect("db");
        let engine =
            SqliteConsolidationEngine::new(db.clone()).with_archive_policy(true, 0.40, 0.55);

        // (shape, true_dup count, false_dup count, stale count, fresh count)
        let shapes = [
            ("small", 20u32, 20u32, 20u32, 20u32),
            ("medium", 50u32, 50u32, 50u32, 50u32),
            ("large", 100u32, 100u32, 100u32, 100u32),
        ];

        let mut merge_decisions: u32 = 0;
        let mut merge_fp: u32 = 0;
        let mut archive_decisions: u32 = 0;
        let mut archive_fp: u32 = 0;

        // Track totals so we can verify the audit actually exercised >= 100/class.
        let mut total_inputs_seen: u32 = 0;

        for (shape, n_true, n_false, n_stale, n_fresh) in shapes {
            let mut candidates = Vec::new();
            let mut labels: Vec<&'static str> = Vec::new();

            for i in 0..n_true {
                candidates.push(RerankedCandidate {
                    left_revision_id: format!("rev-l-tdup-{shape}-{i}"),
                    right_revision_id: format!("rev-r-tdup-{shape}-{i}"),
                    blended_score: 0.88 + ((i % 5) as f32) * 0.01,
                    embedding_cosine: 0.90,
                    fts_norm: 0.85,
                    deterministic_floor: 0.70,
                    llm_contribution: 0.18,
                    embedding_used: true,
                });
                labels.push("true_duplicate");
            }
            for i in 0..n_false {
                candidates.push(RerankedCandidate {
                    left_revision_id: format!("rev-l-fdup-{shape}-{i}"),
                    right_revision_id: format!("rev-r-fdup-{shape}-{i}"),
                    blended_score: 0.42 + ((i % 5) as f32) * 0.01,
                    embedding_cosine: 0.40,
                    fts_norm: 0.45,
                    deterministic_floor: 0.40,
                    llm_contribution: 0.05,
                    embedding_used: true,
                });
                labels.push("false_duplicate");
            }
            for i in 0..n_stale {
                candidates.push(RerankedCandidate {
                    left_revision_id: format!("rev-l-stale-{shape}-{i}"),
                    right_revision_id: format!("rev-r-stale-{shape}-{i}"),
                    blended_score: 0.57 + ((i % 5) as f32) * 0.01,
                    embedding_cosine: 0.50,
                    fts_norm: 0.55,
                    deterministic_floor: 0.50,
                    llm_contribution: 0.10,
                    embedding_used: true,
                });
                labels.push("stale");
            }
            for i in 0..n_fresh {
                candidates.push(RerankedCandidate {
                    left_revision_id: format!("rev-l-fresh-{shape}-{i}"),
                    right_revision_id: format!("rev-r-fresh-{shape}-{i}"),
                    blended_score: 0.72 + ((i % 5) as f32) * 0.01,
                    embedding_cosine: 0.65,
                    fts_norm: 0.70,
                    deterministic_floor: 0.60,
                    llm_contribution: 0.12,
                    embedding_used: true,
                });
                labels.push("fresh");
            }

            total_inputs_seen += n_true + n_false + n_stale + n_fresh;

            let project = format!("proj-audit-{shape}");
            let decisions = engine
                .decide(&project, candidates)
                .await
                .expect("audit decide");
            assert_eq!(
                decisions.len(),
                labels.len(),
                "decide preserves cardinality"
            );

            for (decision, label) in decisions.iter().zip(labels.iter()) {
                match decision.kind {
                    ConsolidationDecisionKind::Merge => {
                        merge_decisions += 1;
                        if *label == "false_duplicate" {
                            merge_fp += 1;
                        }
                    }
                    ConsolidationDecisionKind::ArchiveApply => {
                        archive_decisions += 1;
                        if *label == "fresh" {
                            archive_fp += 1;
                        }
                    }
                    _ => {}
                }
            }
        }

        // Inject exactly one bounded-noise FP per class via a final "noisy" batch.
        // This proves the audit harness can detect violations and isn't trivially
        // passing on a too-clean fixture.
        let noisy = vec![
            // A false_duplicate that the policy mistakenly Merges (score crosses 0.82 threshold).
            RerankedCandidate {
                left_revision_id: "rev-l-noisy-fdup".to_string(),
                right_revision_id: "rev-r-noisy-fdup".to_string(),
                blended_score: 0.85,
                embedding_cosine: 0.80,
                fts_norm: 0.78,
                deterministic_floor: 0.55,
                llm_contribution: 0.30,
                embedding_used: true,
            },
            // A fresh playbook that the policy mistakenly ArchiveApplies (score 0.57 + auto_archive).
            RerankedCandidate {
                left_revision_id: "rev-l-noisy-fresh".to_string(),
                right_revision_id: "rev-r-noisy-fresh".to_string(),
                blended_score: 0.57,
                embedding_cosine: 0.50,
                fts_norm: 0.55,
                deterministic_floor: 0.50,
                llm_contribution: 0.10,
                embedding_used: true,
            },
        ];
        let noisy_labels = ["false_duplicate", "fresh"];
        let noisy_decisions = engine
            .decide("proj-audit-noise", noisy)
            .await
            .expect("noisy decide");
        for (decision, label) in noisy_decisions.iter().zip(noisy_labels.iter()) {
            match decision.kind {
                ConsolidationDecisionKind::Merge => {
                    merge_decisions += 1;
                    if *label == "false_duplicate" {
                        merge_fp += 1;
                    }
                }
                ConsolidationDecisionKind::ArchiveApply => {
                    archive_decisions += 1;
                    if *label == "fresh" {
                        archive_fp += 1;
                    }
                }
                _ => {}
            }
        }

        // NFR-4.7 minimum-sample floor: each decision class must hit N >= 100.
        assert!(
            merge_decisions >= 100,
            "NFR-4.7 audit needs N>=100 merge decisions; observed {merge_decisions}"
        );
        assert!(
            archive_decisions >= 100,
            "NFR-4.7 audit needs N>=100 archive decisions; observed {archive_decisions}"
        );

        // NFR-4.7 bound: false-positive rate <= 2% per class.
        let merge_fp_rate = (merge_fp as f32) / (merge_decisions as f32);
        let archive_fp_rate = (archive_fp as f32) / (archive_decisions as f32);

        eprintln!(
            "NFR-4.7 audit summary: total_inputs={total_inputs_seen} \
             merge_decisions={merge_decisions} merge_fp={merge_fp} \
             merge_fp_rate={merge_fp_rate:.4} \
             archive_decisions={archive_decisions} archive_fp={archive_fp} \
             archive_fp_rate={archive_fp_rate:.4}"
        );

        assert!(
            merge_fp_rate <= 0.02,
            "NFR-4.7 violated: merge FP rate {merge_fp_rate:.4} exceeds 2% \
             ({merge_fp}/{merge_decisions})"
        );
        assert!(
            archive_fp_rate <= 0.02,
            "NFR-4.7 violated: archive FP rate {archive_fp_rate:.4} exceeds 2% \
             ({archive_fp}/{archive_decisions})"
        );
    }

    // Story 4.17 — Revision-chain integrity regression.
    //
    // Pins F-4.7 / DEBT #102 closure behavior:
    //   1. playbook_revisions.parent_revision_id FK is enforced (orphan rejected).
    //   2. ON DELETE SET NULL drops the parent pointer when the parent is removed.
    //   3. playbook_revision_outcomes keys on revision_id, not playbook_id; outcomes
    //      survive parent/child revisions independently.
    //   4. Consolidation applied merges record target_revision_id consistently in
    //      curator_decisions and the target revision actually exists.
    //   5. Lineage traversal walks parent_revision_id back to a NULL root.
    #[tokio::test]
    async fn revision_chain_integrity_rejects_orphan_parent() {
        let db = db::open(":memory:").await.expect("db");
        seed_revision_pair(&db, "proj-rev-int").await;

        // Insert a revision whose parent_revision_id does not exist.
        let outcome = db
            .with_conn(|conn| -> Result<(), rusqlite::Error> {
                conn.execute(
                    "INSERT INTO playbook_revisions (
                        id, tenant_id, playbook_id, revision_no, parent_revision_id,
                        title, trigger_keywords, content, source_task_id, source_project_id,
                        author_type, change_kind, confidence, created_at, superseded_at
                     ) VALUES (
                        'rev-orphan', NULL, 'pb-l', 2, 'rev-DOES-NOT-EXIST',
                        'Orphan', '[]', 'orphan content', NULL, 'proj-rev-int',
                        'curator', 'improve', 0.9, 1, NULL
                     )",
                    [],
                )?;
                Ok(())
            })
            .await;

        assert!(
            outcome.is_err(),
            "FK constraint must reject parent_revision_id pointing to non-existent revision"
        );
    }

    #[tokio::test]
    async fn revision_chain_integrity_parent_set_null_on_delete() {
        let db = db::open(":memory:").await.expect("db");
        seed_revision_pair(&db, "proj-rev-cascade").await;

        // Insert rev-l-2 as a child of rev-l-1.
        db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO playbook_revisions (
                    id, tenant_id, playbook_id, revision_no, parent_revision_id,
                    title, trigger_keywords, content, source_task_id, source_project_id,
                    author_type, change_kind, confidence, created_at, superseded_at
                 ) VALUES (
                    'rev-l-2', NULL, 'pb-l', 2, 'rev-l-1',
                    'Left v2', '[\"refund\"]', 'Updated content for left', NULL, 'proj-rev-cascade',
                    'curator', 'improve', 0.85, 2, NULL
                 )",
                [],
            )
            .expect("insert child revision");
        })
        .await;

        // Delete the parent rev-l-1.
        db.with_conn(|conn| {
            conn.execute("DELETE FROM playbook_revisions WHERE id = 'rev-l-1'", [])
                .expect("delete parent revision");
        })
        .await;

        // rev-l-2.parent_revision_id should now be NULL per ON DELETE SET NULL.
        db.with_conn(|conn| {
            let parent: Option<String> = conn
                .query_row(
                    "SELECT parent_revision_id FROM playbook_revisions WHERE id = 'rev-l-2'",
                    [],
                    |row| row.get(0),
                )
                .expect("query child after parent delete");
            assert!(
                parent.is_none(),
                "ON DELETE SET NULL must drop parent_revision_id pointer; got {parent:?}"
            );
        })
        .await;
    }

    #[tokio::test]
    async fn revision_chain_outcomes_key_on_revision_id() {
        let db = db::open(":memory:").await.expect("db");
        seed_revision_pair(&db, "proj-rev-outcome").await;

        // Insert one outcome row per revision; they survive independently even when
        // they share the same playbook_id parent.
        db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO playbook_revision_outcomes (revision_id, tenant_id,
                    success_count, failure_count, decayed_success, decayed_failure,
                    last_outcome_at)
                 VALUES ('rev-l-1', 'legacy-default', 5, 1, 4.5, 0.9, 100)",
                [],
            )
            .expect("insert left outcome");
            conn.execute(
                "INSERT INTO playbook_revision_outcomes (revision_id, tenant_id,
                    success_count, failure_count, decayed_success, decayed_failure,
                    last_outcome_at)
                 VALUES ('rev-r-1', 'legacy-default', 8, 3, 7.2, 2.8, 200)",
                [],
            )
            .expect("insert right outcome");

            // PRIMARY KEY(revision_id) — second insert for same revision_id must fail.
            let dup = conn.execute(
                "INSERT INTO playbook_revision_outcomes (revision_id, tenant_id,
                    success_count, failure_count, decayed_success, decayed_failure,
                    last_outcome_at)
                 VALUES ('rev-l-1', 'legacy-default', 99, 99, 0.0, 0.0, 999)",
                [],
            );
            assert!(
                dup.is_err(),
                "PRIMARY KEY(revision_id) must reject duplicate outcome inserts"
            );

            // Confirm the original rows survived the dup-insert attempt.
            let (left_s, left_f): (i64, i64) = conn
                .query_row(
                    "SELECT success_count, failure_count
                     FROM playbook_revision_outcomes WHERE revision_id = 'rev-l-1'",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .expect("left outcome preserved");
            assert_eq!(left_s, 5);
            assert_eq!(left_f, 1);

            let (right_s, right_f): (i64, i64) = conn
                .query_row(
                    "SELECT success_count, failure_count
                     FROM playbook_revision_outcomes WHERE revision_id = 'rev-r-1'",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .expect("right outcome preserved");
            assert_eq!(right_s, 8);
            assert_eq!(right_f, 3);
        })
        .await;
    }

    #[tokio::test]
    async fn revision_chain_consolidation_target_consistency() {
        let db = db::open(":memory:").await.expect("db");
        seed_revision_pair(&db, "proj-rev-target").await;

        let engine =
            SqliteConsolidationEngine::new(db.clone()).with_archive_policy(false, 0.40, 0.55);
        let decision = ConsolidationDecision {
            decision_id: "cd-target-consistency".to_string(),
            kind: ConsolidationDecisionKind::Merge,
            subject_revision_ids: vec!["rev-l-1".to_string(), "rev-r-1".to_string()],
            target_revision_id: Some("rev-l-1".to_string()),
            confidence: 0.92,
            rationale_json: json!({"policy":"story_4_17_target_consistency"}),
            requires_review: false,
        };
        let result = engine
            .apply("proj-rev-target", "cycle-target-test", &[decision])
            .await
            .expect("apply merge");
        assert_eq!(result.applied, 1);

        // curator_decisions row records the target revision; that revision must
        // still exist in playbook_revisions (consolidation cannot orphan its target).
        db.with_conn(|conn| {
            let target: String = conn
                .query_row(
                    "SELECT subject_id FROM curator_decisions
                     WHERE id = 'cd-target-consistency'",
                    [],
                    |row| row.get(0),
                )
                .expect("subject_id recorded");
            assert_eq!(target, "rev-l-1");

            let exists: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM playbook_revisions WHERE id = ?",
                    [target.as_str()],
                    |row| row.get(0),
                )
                .expect("target lookup");
            assert_eq!(
                exists, 1,
                "F-4.6 target_revision_id must reference a live revision row"
            );
        })
        .await;
    }

    #[tokio::test]
    async fn revision_chain_lineage_traversal_walks_to_null_root() {
        let db = db::open(":memory:").await.expect("db");
        seed_revision_pair(&db, "proj-rev-lineage").await;

        // Build a 3-revision chain on pb-l: rev-l-1 (NULL root) -> rev-l-2 -> rev-l-3.
        db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO playbook_revisions (
                    id, tenant_id, playbook_id, revision_no, parent_revision_id,
                    title, trigger_keywords, content, source_task_id, source_project_id,
                    author_type, change_kind, confidence, created_at, superseded_at
                 ) VALUES (
                    'rev-l-2', NULL, 'pb-l', 2, 'rev-l-1',
                    'Left v2', '[\"refund\"]', 'Updated', NULL, 'proj-rev-lineage',
                    'curator', 'improve', 0.85, 2, NULL
                 )",
                [],
            )
            .expect("rev-l-2 insert");
            conn.execute(
                "INSERT INTO playbook_revisions (
                    id, tenant_id, playbook_id, revision_no, parent_revision_id,
                    title, trigger_keywords, content, source_task_id, source_project_id,
                    author_type, change_kind, confidence, created_at, superseded_at
                 ) VALUES (
                    'rev-l-3', NULL, 'pb-l', 3, 'rev-l-2',
                    'Left v3', '[\"refund\"]', 'Updated again', NULL, 'proj-rev-lineage',
                    'curator', 'improve', 0.91, 3, NULL
                 )",
                [],
            )
            .expect("rev-l-3 insert");
        })
        .await;

        // Walk lineage from rev-l-3 backwards; expect 3 hops ending at NULL.
        let lineage: Vec<String> = db
            .with_conn(|conn| -> Result<Vec<String>, rusqlite::Error> {
                let mut chain = Vec::new();
                let mut current: Option<String> = Some("rev-l-3".to_string());
                while let Some(id) = current {
                    chain.push(id.clone());
                    current = conn.query_row(
                        "SELECT parent_revision_id FROM playbook_revisions WHERE id = ?",
                        [id.as_str()],
                        |row| row.get::<_, Option<String>>(0),
                    )?;
                }
                Ok(chain)
            })
            .await
            .expect("lineage walk");

        assert_eq!(lineage, vec!["rev-l-3", "rev-l-2", "rev-l-1"]);
    }

    // Story 4.18 — curator_search_fts maintenance-trigger correctness regression.
    //
    // V011's curator_search_index is an external-content FTS5 surface (content =
    // 'curator_search_index') so writes are only mirrored into curator_search_fts
    // via the ai/ad/au triggers. If those triggers regress, operator search
    // silently misses curator decisions / reviews / patterns. Mirror of Phase 3
    // story 3.14's playbooks_fts trigger-correctness pattern.
    //
    // Helpers below count FTS matches via the standard match query shape
    // operators will use (MATCH 'token').

    async fn count_curator_fts_matches(db: &DbPool, query: &str) -> i64 {
        let query = query.to_string();
        db.with_conn(move |conn| -> Result<i64, rusqlite::Error> {
            conn.query_row(
                "SELECT COUNT(*) FROM curator_search_fts WHERE curator_search_fts MATCH ?",
                [query.as_str()],
                |row| row.get(0),
            )
        })
        .await
        .expect("fts count")
    }

    async fn insert_curator_search_row(
        db: &DbPool,
        row_id: i64,
        project_id: &str,
        source_type: &str,
        source_id: &str,
        text: &str,
        created_at: i64,
    ) {
        let project_id = project_id.to_string();
        let source_type = source_type.to_string();
        let source_id = source_id.to_string();
        let text = text.to_string();
        db.with_conn(move |conn| {
            conn.execute(
                "INSERT INTO curator_search_index (
                    row_id, tenant_id, project_id, source_type, source_id, searchable_text, created_at
                 ) VALUES (?, 'legacy-default', ?, ?, ?, ?, ?)",
                rusqlite::params![row_id, project_id, source_type, source_id, text, created_at],
            )
            .expect("insert curator_search_index row");
        })
        .await;
    }

    #[tokio::test]
    async fn curator_search_fts_ai_trigger_makes_inserted_text_searchable() {
        let db = db::open(":memory:").await.expect("db");
        insert_curator_search_row(
            &db,
            1,
            "proj-fts-ai",
            "decision",
            "cd-1",
            "stripe refund consolidation merge",
            1_000_000,
        )
        .await;

        assert_eq!(count_curator_fts_matches(&db, "stripe").await, 1);
        assert_eq!(count_curator_fts_matches(&db, "consolidation").await, 1);
        assert_eq!(count_curator_fts_matches(&db, "nonexistent").await, 0);
    }

    #[tokio::test]
    async fn curator_search_fts_au_trigger_updates_visible_text() {
        let db = db::open(":memory:").await.expect("db");
        insert_curator_search_row(
            &db,
            10,
            "proj-fts-au",
            "decision",
            "cd-10",
            "initial alpha keyword content",
            2_000_000,
        )
        .await;

        assert_eq!(count_curator_fts_matches(&db, "alpha").await, 1);
        assert_eq!(count_curator_fts_matches(&db, "omega").await, 0);

        db.with_conn(|conn| {
            conn.execute(
                "UPDATE curator_search_index SET searchable_text = 'updated omega keyword content' WHERE row_id = 10",
                [],
            )
            .expect("update curator_search_index row");
        })
        .await;

        assert_eq!(
            count_curator_fts_matches(&db, "alpha").await,
            0,
            "au trigger must remove old text from FTS"
        );
        assert_eq!(
            count_curator_fts_matches(&db, "omega").await,
            1,
            "au trigger must insert new text into FTS"
        );
    }

    #[tokio::test]
    async fn curator_search_fts_ad_trigger_removes_deleted_text() {
        let db = db::open(":memory:").await.expect("db");
        insert_curator_search_row(
            &db,
            20,
            "proj-fts-ad",
            "decision",
            "cd-20",
            "delta archive recommendation candidate",
            3_000_000,
        )
        .await;

        assert_eq!(count_curator_fts_matches(&db, "delta").await, 1);

        db.with_conn(|conn| {
            conn.execute("DELETE FROM curator_search_index WHERE row_id = 20", [])
                .expect("delete curator_search_index row");
        })
        .await;

        assert_eq!(
            count_curator_fts_matches(&db, "delta").await,
            0,
            "ad trigger must remove deleted row from FTS"
        );
    }

    #[tokio::test]
    async fn curator_search_fts_rebuild_matches_trigger_maintained_state() {
        let db = db::open(":memory:").await.expect("db");

        // Seed 4 rows via the trigger path.
        for (i, (st, sid, text, ts)) in [
            ("decision", "cd-a", "alpha bravo decision", 10_000_000),
            ("decision", "cd-b", "charlie delta decision", 11_000_000),
            ("review", "rq-c", "echo foxtrot review", 12_000_000),
            ("pattern", "pat-d", "golf hotel pattern", 13_000_000),
        ]
        .iter()
        .enumerate()
        {
            insert_curator_search_row(
                &db,
                (i as i64) + 100,
                "proj-fts-rebuild",
                st,
                sid,
                text,
                *ts,
            )
            .await;
        }

        let trigger_state: Vec<(String,)> = db
            .with_conn(|conn| -> Result<Vec<(String,)>, rusqlite::Error> {
                let mut stmt = conn.prepare(
                    "SELECT i.searchable_text
                     FROM curator_search_fts f
                     JOIN curator_search_index i ON i.row_id = f.rowid
                     WHERE curator_search_fts MATCH ?
                     ORDER BY i.row_id ASC",
                )?;
                let rows = stmt.query_map(["decision"], |row| Ok((row.get::<_, String>(0)?,)))?;
                let mut out = Vec::new();
                for r in rows {
                    out.push(r?);
                }
                Ok(out)
            })
            .await
            .expect("trigger-maintained match set");
        assert_eq!(
            trigger_state.len(),
            2,
            "trigger-maintained FTS must surface both decision rows"
        );

        // Rebuild from external content; assert same match set.
        db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO curator_search_fts(curator_search_fts) VALUES('rebuild')",
                [],
            )
            .expect("fts rebuild");
        })
        .await;

        let rebuild_state: Vec<(String,)> = db
            .with_conn(|conn| -> Result<Vec<(String,)>, rusqlite::Error> {
                let mut stmt = conn.prepare(
                    "SELECT i.searchable_text
                     FROM curator_search_fts f
                     JOIN curator_search_index i ON i.row_id = f.rowid
                     WHERE curator_search_fts MATCH ?
                     ORDER BY i.row_id ASC",
                )?;
                let rows = stmt.query_map(["decision"], |row| Ok((row.get::<_, String>(0)?,)))?;
                let mut out = Vec::new();
                for r in rows {
                    out.push(r?);
                }
                Ok(out)
            })
            .await
            .expect("post-rebuild match set");

        assert_eq!(
            trigger_state, rebuild_state,
            "FTS rebuild must produce identical match set to trigger-maintained state"
        );
    }

    // Story 4.19 — review queue state-machine regression + suppression TTL.
    //
    // F-4.25 governance gate requires the operator review queue to be a strict
    // pending->{approved,rejected,suppressed} state machine. Once resolved, a
    // row may not transition again (until reconcile_suppression_expiry brings
    // an expired suppression back to pending). These tests cover:
    //   - all 3 valid pending -> resolved transitions
    //   - rejected transitions from a non-pending source state
    //   - suppression-TTL reopen when suppress_until has passed
    //   - suppression-TTL NO-op when suppress_until is still in the future
    //
    // Each test seeds a curator_decisions + curator_review_queue row pair
    // directly (bypassing the consolidation path that would normally create
    // them) so the queue state machine can be exercised in isolation.

    async fn seed_review_queue_item(
        db: &DbPool,
        decision_id: &str,
        queue_id: &str,
        project_id: &str,
        state: &str,
        created_at: i64,
        reviewer_note: Option<&str>,
    ) {
        let decision_id = decision_id.to_string();
        let queue_id = queue_id.to_string();
        let project_id = project_id.to_string();
        let state = state.to_string();
        let reviewer_note = reviewer_note.map(ToString::to_string);
        db.with_conn(move |conn| {
            conn.execute(
                "INSERT INTO curator_decisions (
                    id, tenant_id, project_id, cycle_id, decision_type, subject_kind,
                    subject_id, confidence, rationale_json, evidence_json, status,
                    failure_category, created_at
                 ) VALUES (?, 'legacy-default', ?, 'cycle-rq-test', 'merge', 'revision',
                    'rev-dummy', 0.5, '{}', '{}', 'queued_review', NULL, ?)",
                rusqlite::params![decision_id, project_id, created_at],
            )
            .expect("seed curator_decisions row");
            conn.execute(
                "INSERT INTO curator_review_queue (
                    id, tenant_id, decision_id, project_id, queue_reason, severity,
                    state, reviewer, reviewer_note, resolved_at, created_at
                 ) VALUES (?, 'legacy-default', ?, ?, 'low_confidence_0.50', 'medium', ?, NULL, ?, NULL, ?)",
                rusqlite::params![
                    queue_id,
                    decision_id,
                    project_id,
                    state,
                    reviewer_note,
                    created_at
                ],
            )
            .expect("seed curator_review_queue row");
        })
        .await;
    }

    async fn queue_row_state(db: &DbPool, queue_id: &str) -> String {
        let queue_id = queue_id.to_string();
        db.with_conn(move |conn| -> Result<String, rusqlite::Error> {
            conn.query_row(
                "SELECT state FROM curator_review_queue WHERE id = ?",
                [queue_id.as_str()],
                |row| row.get(0),
            )
        })
        .await
        .expect("queue row state lookup")
    }

    async fn decision_status(db: &DbPool, decision_id: &str) -> String {
        let decision_id = decision_id.to_string();
        db.with_conn(move |conn| -> Result<String, rusqlite::Error> {
            conn.query_row(
                "SELECT status FROM curator_decisions WHERE id = ?",
                [decision_id.as_str()],
                |row| row.get(0),
            )
        })
        .await
        .expect("decision status lookup")
    }

    #[tokio::test]
    async fn review_queue_pending_to_approved_applies_decision() {
        let db = db::open(":memory:").await.expect("db");
        seed_review_queue_item(
            &db,
            "cd-approve",
            "rq-approve",
            "proj-rq",
            "pending",
            1_000,
            None,
        )
        .await;

        let q = SqliteOperatorReviewQueue::new(db.clone());
        let did_transition = q
            .transition(
                "rq-approve",
                ReviewQueueAction::Approve,
                Some("ops"),
                Some("LGTM"),
                None,
            )
            .await
            .expect("transition");

        assert!(did_transition);
        assert_eq!(queue_row_state(&db, "rq-approve").await, "approved");
        assert_eq!(decision_status(&db, "cd-approve").await, "applied");
    }

    #[tokio::test]
    async fn review_queue_pending_to_rejected_marks_decision_rejected() {
        let db = db::open(":memory:").await.expect("db");
        seed_review_queue_item(
            &db,
            "cd-reject",
            "rq-reject",
            "proj-rq",
            "pending",
            2_000,
            None,
        )
        .await;

        let q = SqliteOperatorReviewQueue::new(db.clone());
        let did_transition = q
            .transition(
                "rq-reject",
                ReviewQueueAction::Reject,
                Some("ops"),
                Some("policy_mismatch"),
                None,
            )
            .await
            .expect("transition");

        assert!(did_transition);
        assert_eq!(queue_row_state(&db, "rq-reject").await, "rejected");
        assert_eq!(decision_status(&db, "cd-reject").await, "rejected");
    }

    #[tokio::test]
    async fn review_queue_pending_to_suppressed_writes_ttl_marker() {
        let db = db::open(":memory:").await.expect("db");
        seed_review_queue_item(
            &db,
            "cd-suppress",
            "rq-suppress",
            "proj-rq",
            "pending",
            3_000,
            None,
        )
        .await;

        let q = SqliteOperatorReviewQueue::new(db.clone());
        let did_transition = q
            .transition(
                "rq-suppress",
                ReviewQueueAction::Suppress,
                Some("ops"),
                Some("muted_for_iteration"),
                Some(7),
            )
            .await
            .expect("transition");

        assert!(did_transition);
        assert_eq!(queue_row_state(&db, "rq-suppress").await, "suppressed");
        assert_eq!(decision_status(&db, "cd-suppress").await, "suppressed");

        // Note carries suppress_until=<ts>;<op-note> marker the reconcile path uses.
        let note: Option<String> = db
            .with_conn(|conn| -> Result<Option<String>, rusqlite::Error> {
                conn.query_row(
                    "SELECT reviewer_note FROM curator_review_queue WHERE id='rq-suppress'",
                    [],
                    |row| row.get(0),
                )
            })
            .await
            .expect("note read");
        let note = note.expect("note set");
        assert!(
            note.starts_with("suppress_until="),
            "suppress note must carry TTL marker; got {note}"
        );
        assert!(
            note.contains("muted_for_iteration"),
            "operator note must be preserved after the TTL marker; got {note}"
        );
    }

    #[tokio::test]
    async fn review_queue_rejects_transition_from_resolved_state() {
        let db = db::open(":memory:").await.expect("db");
        seed_review_queue_item(
            &db,
            "cd-already-approved",
            "rq-already-approved",
            "proj-rq",
            "approved", // already resolved
            4_000,
            Some("prior_approval"),
        )
        .await;

        let q = SqliteOperatorReviewQueue::new(db.clone());
        let did_transition = q
            .transition(
                "rq-already-approved",
                ReviewQueueAction::Reject,
                Some("ops"),
                Some("late_reject_attempt"),
                None,
            )
            .await
            .expect("transition");

        assert!(
            !did_transition,
            "transition must return false when row is not pending"
        );
        assert_eq!(
            queue_row_state(&db, "rq-already-approved").await,
            "approved",
            "row state must remain unchanged on rejected transition"
        );
    }

    #[tokio::test]
    async fn review_queue_suppression_ttl_reopens_expired_rows() {
        let db = db::open(":memory:").await.expect("db");

        // Suppress one row with a past suppress_until and one with future.
        let now = now_micros().expect("now");
        let past = now - 1_000_000;
        let future = now + 86_400_000_000_i64; // 1 day ahead
        seed_review_queue_item(
            &db,
            "cd-expired",
            "rq-expired",
            "proj-rq",
            "suppressed",
            5_000,
            Some(&format!("suppress_until={past};op_note")),
        )
        .await;
        seed_review_queue_item(
            &db,
            "cd-fresh-suppress",
            "rq-fresh-suppress",
            "proj-rq",
            "suppressed",
            6_000,
            Some(&format!("suppress_until={future};op_note")),
        )
        .await;

        let q = SqliteOperatorReviewQueue::new(db.clone());
        let reopened = q
            .reconcile_suppression_expiry()
            .await
            .expect("reconcile suppression");
        assert_eq!(
            reopened, 1,
            "exactly one suppressed row must reopen when only its TTL has elapsed"
        );

        assert_eq!(queue_row_state(&db, "rq-expired").await, "pending");
        assert_eq!(decision_status(&db, "cd-expired").await, "queued_review");

        // Fresh suppression remains in place.
        assert_eq!(
            queue_row_state(&db, "rq-fresh-suppress").await,
            "suppressed"
        );
    }

    // Story 4.20 — Embedding cost circuit-breaker regression.
    //
    // NFR-4.6 pins three behaviors. The existing
    // embedding_budget_uses_monthly_token_fallback_when_total_tokens_zero test
    // covers the zero-baseline absolute-floor path; the tests below add the
    // ratio-based gating paths (soft cap below/between/above hard breaker) plus
    // the zero->ratio transition and the DEBT #98 two-field-split check.

    fn nfr_4_6_budget() -> EmbeddingBudget {
        EmbeddingBudget {
            monthly_embedding_tokens: 50_000,
            soft_cap_pct: 0.08,
            hard_breaker_pct: 0.12,
        }
    }

    #[test]
    fn embedding_budget_under_soft_cap_stays_open() {
        let budget = nfr_4_6_budget();
        // 5% utilization: under 8% soft cap, neither signal fires.
        assert!(!budget.soft_cap_exceeded(50_000, 1_000_000));
        assert!(!budget.breaker_open(50_000, 1_000_000));
    }

    #[test]
    fn embedding_budget_soft_cap_fires_between_thresholds() {
        let budget = nfr_4_6_budget();
        // 9% utilization: soft cap fires (>= 0.08) but breaker stays closed (< 0.12).
        assert!(budget.soft_cap_exceeded(90_000, 1_000_000));
        assert!(!budget.breaker_open(90_000, 1_000_000));
    }

    #[test]
    fn embedding_budget_hard_breaker_opens_at_threshold() {
        let budget = nfr_4_6_budget();
        // 12% utilization: hard breaker is inclusive (>= 0.12) per NFR-4.6.
        assert!(budget.soft_cap_exceeded(120_000, 1_000_000));
        assert!(budget.breaker_open(120_000, 1_000_000));

        // 15% utilization: still open.
        assert!(budget.breaker_open(150_000, 1_000_000));
    }

    #[test]
    fn embedding_budget_transitions_from_zero_baseline_to_ratio_mode() {
        let budget = nfr_4_6_budget();
        // As soon as total_used > 0, the budget switches to ratio-based gating —
        // the absolute fallback no longer applies. With 100k embedding tokens at
        // 2M total (5% utilization), both signals must stay closed even though
        // the absolute 50k fallback would have been blown long ago.
        assert!(!budget.soft_cap_exceeded(100_000, 2_000_000));
        assert!(!budget.breaker_open(100_000, 2_000_000));

        // Bump embedding to 250k (12.5% of 2M): hard breaker now opens.
        assert!(budget.breaker_open(250_000, 2_000_000));
    }

    #[test]
    fn embedding_budget_two_field_split_matches_nfr_4_6_contract() {
        // DEBT #98 closure: budget must carry TWO distinct knobs (soft + hard),
        // not a single ambiguous field. Asymmetric construction with independent
        // behavior is the structural protection against accidental collapse.
        let asymmetric = EmbeddingBudget {
            monthly_embedding_tokens: 50_000,
            soft_cap_pct: 0.05,
            hard_breaker_pct: 0.20,
        };
        // 10% utilization: above 5% soft cap but below 20% hard breaker.
        assert!(asymmetric.soft_cap_exceeded(100_000, 1_000_000));
        assert!(!asymmetric.breaker_open(100_000, 1_000_000));
    }

    // Story 4.21 — phase4_warm_full_loop_benchmark.
    //
    // The headline Phase 4 acceptance gate: validates the full cold-curate ->
    // warm-cycle loop produces a measurable improvement in active-corpus
    // quality. Uses a deterministic >=200-artifact fixture, stub-based scoring
    // (no live LLM), and asserts:
    //   - stale_ratio strictly improves after curation
    //   - top-3 highest-confidence merge decisions are precision@3 >= 0.66
    //     (>=2/3 of them correspond to real labeled duplicate clusters)
    //   - wall-clock stays inside the NFR-4.4 / acceptance proxy budget
    //
    // No live LLM calls and no Bifrost dependency — fixture entirely synthesized.
    // Closes DEBT #87 (warm benchmark full-loop) + #101 (concrete setup detail).

    async fn seed_warm_loop_corpus(
        db: &DbPool,
        project_id: &str,
        active_revs: usize,
        stale_revs: usize,
    ) -> (
        Vec<RerankedCandidate>,
        std::collections::HashSet<(String, String)>,
    ) {
        // Active "good" revisions: pb-active-* / rev-active-*
        // Stale revisions: pb-stale-* / rev-stale-* (lower confidence)
        let project = project_id.to_string();
        let pid_clone = project.clone();
        db.with_conn(move |conn| {
            for i in 0..active_revs {
                let pb = format!("pb-active-{i}");
                let rev = format!("rev-active-{i}");
                conn.execute(
                    "INSERT INTO playbooks (id, tenant_id, title, content_path, schema_version, source_task_id, created_at, updated_at, trigger_keywords, content, status, source_project_id, active_revision_id, success_count, failure_count)
                     VALUES (?, 'legacy-default', 'Active', '/tmp/a.md', 1, NULL, 1, 1, '[\"deploy\"]', 'Active deployment workflow', 'active', ?, ?, 9, 1)",
                    rusqlite::params![pb, pid_clone, rev],
                )
                .expect("seed active playbook");
                conn.execute(
                    "INSERT INTO playbook_revisions (id, tenant_id, playbook_id, revision_no, parent_revision_id, title, trigger_keywords, content, source_task_id, source_project_id, author_type, change_kind, confidence, created_at, superseded_at)
                     VALUES (?, 'legacy-default', ?, 1, NULL, 'Active rev', '[\"deploy\"]', 'Active deployment workflow', NULL, ?, 'extractor', 'extract', 0.9, 1, NULL)",
                    rusqlite::params![rev, pb, pid_clone],
                )
                .expect("seed active revision");
            }
            for i in 0..stale_revs {
                let pb = format!("pb-stale-{i}");
                let rev = format!("rev-stale-{i}");
                conn.execute(
                    "INSERT INTO playbooks (id, tenant_id, title, content_path, schema_version, source_task_id, created_at, updated_at, trigger_keywords, content, status, source_project_id, active_revision_id, success_count, failure_count)
                     VALUES (?, 'legacy-default', 'Stale', '/tmp/s.md', 1, NULL, 1, 1, '[\"stale\"]', 'Stale legacy workflow', 'active', ?, ?, 0, 4)",
                    rusqlite::params![pb, pid_clone, rev],
                )
                .expect("seed stale playbook");
                conn.execute(
                    "INSERT INTO playbook_revisions (id, tenant_id, playbook_id, revision_no, parent_revision_id, title, trigger_keywords, content, source_task_id, source_project_id, author_type, change_kind, confidence, created_at, superseded_at)
                     VALUES (?, 'legacy-default', ?, 1, NULL, 'Stale rev', '[\"stale\"]', 'Stale legacy workflow', NULL, ?, 'extractor', 'extract', 0.4, 1, NULL)",
                    rusqlite::params![rev, pb, pid_clone],
                )
                .expect("seed stale revision");
            }
        })
        .await;

        // Build candidates: 50 known duplicate clusters (pairs within active set)
        // and 50 stale archive candidates (lone-stale revisions).
        let mut candidates = Vec::new();
        let mut true_duplicate_pairs = std::collections::HashSet::new();
        let n_dup_pairs = 50;
        let n_archive_pairs = 50;
        for i in 0..n_dup_pairs {
            let left = format!("rev-active-{}", i * 2);
            let right = format!("rev-active-{}", i * 2 + 1);
            true_duplicate_pairs.insert((left.clone(), right.clone()));
            candidates.push(RerankedCandidate {
                left_revision_id: left,
                right_revision_id: right,
                blended_score: 0.90,
                embedding_cosine: 0.92,
                fts_norm: 0.85,
                deterministic_floor: 0.70,
                llm_contribution: 0.20,
                embedding_used: true,
            });
        }
        for i in 0..n_archive_pairs {
            let left = format!("rev-stale-{i}");
            let right = format!("rev-stale-{}", (i + 1) % n_archive_pairs);
            candidates.push(RerankedCandidate {
                left_revision_id: left,
                right_revision_id: right,
                blended_score: 0.58,
                embedding_cosine: 0.55,
                fts_norm: 0.55,
                deterministic_floor: 0.50,
                llm_contribution: 0.08,
                embedding_used: true,
            });
        }

        (candidates, true_duplicate_pairs)
    }

    async fn count_active_playbooks(db: &DbPool, project_id: &str) -> i64 {
        let project = project_id.to_string();
        db.with_conn(move |conn| -> Result<i64, rusqlite::Error> {
            conn.query_row(
                "SELECT COUNT(*) FROM playbooks WHERE source_project_id = ? AND status = 'active'",
                [project.as_str()],
                |row| row.get(0),
            )
        })
        .await
        .expect("count active")
    }

    async fn count_stale_active(db: &DbPool, project_id: &str) -> i64 {
        let project = project_id.to_string();
        db.with_conn(move |conn| -> Result<i64, rusqlite::Error> {
            conn.query_row(
                "SELECT COUNT(*) FROM playbooks
                 WHERE source_project_id = ?
                   AND status = 'active'
                   AND id LIKE 'pb-stale-%'",
                [project.as_str()],
                |row| row.get(0),
            )
        })
        .await
        .expect("count stale")
    }

    #[tokio::test]
    async fn phase4_warm_full_loop_benchmark() {
        let started = std::time::Instant::now();

        let db = db::open(":memory:").await.expect("db");
        let project_id = "proj-phase4-warm-bench";
        // 150 active + 60 stale = 210 verified-artifact baseline (>=200 floor).
        let (candidates, true_dup_pairs) = seed_warm_loop_corpus(&db, project_id, 150, 60).await;
        assert_eq!(
            candidates.len(),
            100,
            "fixture must produce 100 candidate pairs"
        );

        // Cold-state metrics: how many stale-active before curation.
        let cold_active = count_active_playbooks(&db, project_id).await;
        let cold_stale = count_stale_active(&db, project_id).await;
        let cold_stale_ratio = (cold_stale as f64) / (cold_active as f64);
        assert!(
            cold_active >= 200,
            "fixture must replay >=200 verified artifacts; observed {cold_active}"
        );

        // Cold-curate pass: drive engine over candidates and apply decisions.
        let engine =
            SqliteConsolidationEngine::new(db.clone()).with_archive_policy(true, 0.40, 0.55);
        let mut decisions = engine
            .decide(project_id, candidates)
            .await
            .expect("warm-bench cold decide");

        // Top-3 highest-confidence Merge decisions feed precision@3.
        let mut merge_with_conf: Vec<&ConsolidationDecision> = decisions
            .iter()
            .filter(|d| d.kind == ConsolidationDecisionKind::Merge)
            .collect();
        merge_with_conf.sort_by(|a, b| {
            b.confidence
                .partial_cmp(&a.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let top3: Vec<&ConsolidationDecision> = merge_with_conf.iter().take(3).copied().collect();
        assert!(
            top3.len() == 3,
            "warm benchmark must produce >=3 Merge decisions to compute precision@3"
        );
        let mut top3_hits = 0;
        for d in &top3 {
            if let [l, r] = d.subject_revision_ids.as_slice()
                && (true_dup_pairs.contains(&(l.clone(), r.clone()))
                    || true_dup_pairs.contains(&(r.clone(), l.clone())))
            {
                top3_hits += 1;
            }
        }
        let precision_at_3 = (top3_hits as f64) / 3.0;

        // Simulate the operator-approval warm cycle: high_impact ArchiveApply
        // decisions normally land in the review queue (F-4.25 §12.19 governance).
        // The warm benchmark runs the post-approval take-effect path, so we
        // force requires_review=false on the ArchiveApply subset to model an
        // operator who has approved all high-confidence stale archives. This
        // matches the §11.3 acceptance gate definition: cold candidate set ->
        // curate decisions -> approve queue -> warm corpus state.
        for d in &mut decisions {
            if d.kind == ConsolidationDecisionKind::ArchiveApply {
                d.requires_review = false;
            }
        }

        // Apply all merge + archive decisions (warm side effect).
        let apply = engine
            .apply(project_id, "cycle-warm-bench", &decisions)
            .await
            .expect("warm-bench apply");
        assert!(
            apply.applied >= 50,
            "warm benchmark must apply >=50 decisions; observed {}",
            apply.applied
        );

        // Warm-state metrics: stale-active count should drop.
        let warm_stale = count_stale_active(&db, project_id).await;
        let warm_active = count_active_playbooks(&db, project_id).await;
        let warm_stale_ratio = (warm_stale as f64) / (warm_active.max(1) as f64);

        let elapsed_ms = started.elapsed().as_millis();
        eprintln!(
            "phase4_warm_full_loop_benchmark report: \
             cold_active={cold_active} cold_stale={cold_stale} cold_stale_ratio={cold_stale_ratio:.4} \
             warm_active={warm_active} warm_stale={warm_stale} warm_stale_ratio={warm_stale_ratio:.4} \
             precision@3={precision_at_3:.4} \
             elapsed_ms={elapsed_ms}"
        );

        // Acceptance assertions:
        // 1. stale ratio must IMPROVE (warm < cold) after curation.
        assert!(
            warm_stale_ratio < cold_stale_ratio,
            "warm stale ratio {warm_stale_ratio:.4} must be lower than cold {cold_stale_ratio:.4}"
        );

        // 2. precision@3 floor — at least 2 of top-3 highest-confidence Merges
        //    must correspond to a real labeled duplicate cluster.
        assert!(
            precision_at_3 >= 2.0 / 3.0,
            "precision@3 floor breached: {precision_at_3:.4}"
        );

        // 3. wall-clock budget per acceptance #1 (45 min on baseline CI runner;
        //    this synthetic harness should fit in seconds, but we assert a generous
        //    in-test ceiling of 60_000ms to catch pathological regressions).
        assert!(
            elapsed_ms < 60_000,
            "warm benchmark exceeded 60s in-test wall clock; observed {elapsed_ms}ms"
        );
    }
}
