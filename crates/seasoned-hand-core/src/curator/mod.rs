//! Curator worker runtime for Phase 4.
//! refs: /specs/phase-4/architecture.md §2.1, §2.2, §2.3, §4.1, §4.2, §6.5, §7

use std::collections::{HashMap, HashSet};
use std::hash::Hash;
use std::sync::Arc;
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsolidationDecisionKind {
    Merge,
    Keep,
    ArchiveRecommend,
    ArchiveApply,
    Quarantine,
}

impl ConsolidationDecisionKind {
    fn as_str(self) -> &'static str {
        match self {
            ConsolidationDecisionKind::Merge => "merge",
            ConsolidationDecisionKind::Keep => "keep",
            ConsolidationDecisionKind::ArchiveRecommend => "archive",
            ConsolidationDecisionKind::ArchiveApply => "archive",
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
                         ) VALUES (?1, NULL, ?2, ?3, 'recommendation', ?4, ?5, ?6, ?7, ?8, 'applied', NULL, ?9)",
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
                             ) VALUES (?1, NULL, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
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
                             ) VALUES (?1, NULL, ?2, ?3, 'knowledge_write', 'knowledge', ?4, ?5, ?6, ?7, ?8, NULL, ?9)",
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
                                 ) VALUES (?1, NULL, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
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
                                 ) VALUES (?1, NULL, ?2, ?3, 'datasource_write', 'datasource', ?4, ?5, ?6, ?7, ?8, NULL, ?9)",
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
        Self { db }
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
            let kind = if floor < 0.30 {
                ConsolidationDecisionKind::Quarantine
            } else if confidence >= 0.82 {
                ConsolidationDecisionKind::Merge
            } else if confidence >= 0.65 {
                ConsolidationDecisionKind::Keep
            } else if confidence >= 0.40 {
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
                for decision in &decisions {
                    let status = if decision.requires_review {
                        "queued_review"
                    } else {
                        "applied"
                    };
                    conn.execute(
                        "INSERT INTO curator_decisions (
                            id, tenant_id, project_id, cycle_id, decision_type, subject_kind,
                            subject_id, confidence, rationale_json, evidence_json, status, failure_category, created_at
                         ) VALUES (?1, NULL, ?2, ?3, ?4, 'revision', ?5, ?6, ?7, ?8, ?9, NULL, ?10)",
                        rusqlite::params![
                            decision.decision_id,
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
                             ) VALUES (?1, NULL, ?2, ?3, ?4, ?5, 'pending', NULL, NULL, NULL, ?6)",
                            rusqlite::params![
                                format!("rq-{}", uuid::Uuid::new_v4()),
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

            // architecture §4.2 blend + fallback formulas
            let blended_score = if embedding_used {
                0.45 * fts_norm + 0.40 * embedding_cosine + 0.15 * structural_overlap
            } else {
                0.75 * fts_norm + 0.25 * structural_overlap
            };

            out.push(RerankedCandidate {
                left_revision_id: candidate.left_revision_id,
                right_revision_id: candidate.right_revision_id,
                blended_score,
                embedding_cosine,
                fts_norm,
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
        let _ = self
            .operator_review_queue
            .reconcile_suppression_expiry()
            .await?;
        let candidates = self
            .candidate_builder
            .build_duplicate_candidates(project_id, self.max_candidates_per_cycle)
            .await?;
        let reranked = self.reranker.rerank(project_id, candidates).await?;
        let conflicts = self.conflict_detector.detect(project_id, &reranked).await?;
        let decisions = self
            .consolidation_engine
            .decide(project_id, reranked)
            .await?;
        let cycle_id = format!("cycle-{}", uuid::Uuid::new_v4());
        let apply_result = self
            .consolidation_engine
            .apply(project_id, &cycle_id, &decisions)
            .await?;
        let patterns = self.work_pattern_extractor.extract(project_id).await?;
        let recommendations = self
            .work_pattern_extractor
            .recommend(project_id, &cycle_id, &patterns)
            .await?;
        let write_result = self
            .knowledge_datasource_writer
            .emit_and_promote(project_id, &cycle_id)
            .await?;
        let retrospective = self
            .retrospective_generator
            .generate_if_due(project_id, _trigger, _backlog_count, self.backlog_threshold)
            .await?;

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
            failures: apply_result.failures,
            elapsed_ms: started.elapsed().as_millis() as u64,
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
            ) VALUES (?1, NULL, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
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
                ) VALUES (?1, NULL, ?2, ?3, ?4, ?5, NULL)",
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
             ) VALUES (?1, NULL, ?2, ?3, ?4, ?5, ?6, ?7, 'open', ?8, ?9)",
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
        ) VALUES (?1, NULL, ?2, ?3, ?4, ?5, ?6, ?7, NULL, ?8, 'curator', 'merge', ?9, ?10, NULL)",
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
            format!("curator_decision:{}", decision.decision_id),
            now,
            playbook_id
        ],
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
        if !self.config.enabled {
            tracing::info!("curator worker disabled");
            return Ok(());
        }
        let interval = Duration::from_secs(self.config.interval_seconds.max(1));
        let mut ticker = tokio::time::interval(interval);
        ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
        ticker.tick().await;

        loop {
            tokio::select! {
                _ = cancel.cancelled() => {
                    tracing::info!("curator worker shutdown requested");
                    return Ok(());
                }
                _ = ticker.tick() => {
                    let backlog = match self.backlog_probe.pending_count(&self.config.project_id).await {
                        Ok(count) => count,
                        Err(error) => {
                            tracing::warn!(%error, "curator backlog probe failed");
                            continue;
                        }
                    };
                    let trigger = if backlog > self.config.backlog_threshold {
                        CuratorTrigger::BacklogThreshold
                    } else {
                        CuratorTrigger::IntervalTick
                    };
                    if let Err(error) = self.run_once(trigger, backlog).await {
                        tracing::warn!(%error, "curator cycle failed");
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
                 VALUES ('pb-l', NULL, 'Left', '/tmp/l.md', 1, NULL, 1, 1, '[\"refund\",\"stripe\"]', 'Handle stripe refund policy and customer email.', 'active', ?, 'rev-l-1', 0, 0)",
                [project_id],
            )
            .expect("insert left playbook");
            conn.execute(
                "INSERT INTO playbooks (id, tenant_id, title, content_path, schema_version, source_task_id, created_at, updated_at, trigger_keywords, content, status, source_project_id, active_revision_id, success_count, failure_count)
                 VALUES ('pb-r', NULL, 'Right', '/tmp/r.md', 1, NULL, 1, 1, '[\"refund\",\"billing\"]', 'Refund workflow for billing disputes and stripe chargebacks.', 'active', ?, 'rev-r-1', 0, 0)",
                [project_id],
            )
            .expect("insert right playbook");

            conn.execute(
                "INSERT INTO playbook_revisions (id, tenant_id, playbook_id, revision_no, parent_revision_id, title, trigger_keywords, content, source_task_id, source_project_id, author_type, change_kind, confidence, created_at, superseded_at)
                 VALUES ('rev-l-1', NULL, 'pb-l', 1, NULL, 'Left rev', '[\"refund\",\"stripe\"]', 'Handle stripe refund policy and customer email.', NULL, ?, 'extractor', 'extract', 1.0, 1, NULL)",
                [project_id],
            )
            .expect("insert left revision");
            conn.execute(
                "INSERT INTO playbook_revisions (id, tenant_id, playbook_id, revision_no, parent_revision_id, title, trigger_keywords, content, source_task_id, source_project_id, author_type, change_kind, confidence, created_at, superseded_at)
                 VALUES ('rev-r-1', NULL, 'pb-r', 1, NULL, 'Right rev', '[\"refund\",\"billing\"]', 'Refund workflow for billing disputes and stripe chargebacks.', NULL, ?, 'extractor', 'extract', 1.0, 1, NULL)",
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
                 VALUES ('pb-k', NULL, 'Keep', '/tmp/k.md', 1, NULL, 1, 1, '[\"docs\"]', 'Documentation workflow', 'active', 'proj-consolidate', 'rev-k-1', 0, 0)",
                [],
            )
            .expect("insert keep playbook");
            conn.execute(
                "INSERT INTO playbook_revisions (id, tenant_id, playbook_id, revision_no, parent_revision_id, title, trigger_keywords, content, source_task_id, source_project_id, author_type, change_kind, confidence, created_at, superseded_at)
                 VALUES ('rev-k-1', NULL, 'pb-k', 1, NULL, 'Keep rev', '[\"docs\"]', 'Documentation workflow', NULL, 'proj-consolidate', 'extractor', 'extract', 1.0, 1, NULL)",
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
                    embedding_used: true,
                },
                RerankedCandidate {
                    left_revision_id: candidates[1].left_revision_id.clone(),
                    right_revision_id: candidates[1].right_revision_id.clone(),
                    blended_score: 0.70, // keep branch
                    embedding_cosine: 0.42,
                    fts_norm: 0.71,
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
                 ) VALUES ('rev-l-1', NULL, 1, 5, 0, 0, NULL)",
                [],
            )
            .expect("seed outcomes left");
            conn.execute(
                "INSERT INTO playbook_revision_outcomes (
                    revision_id, tenant_id, success_count, failure_count, decayed_success, decayed_failure, last_outcome_at
                 ) VALUES ('rev-r-1', NULL, 4, 1, 0, 0, NULL)",
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
                    'ds-prev-1', NULL, 'proj-kd', 'rev-r-1', 'task-kd-0', 'url',
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
                    'conf-kd-1', NULL, 'proj-kd', 'rev-l-1', 'rev-r-1',
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
}
