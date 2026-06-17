//! Unit + integration tests for the curator module.
//! Extracted from mod.rs (manageability iter-1) — behaviour-preserving.

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
            review_sample_rate: DEFAULT_REVIEW_SAMPLE_RATE,
            project_id: "proj-1".to_string(),
            org_aggregation_enabled: false,
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
            hard_breaker_pct: 1.10,
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

#[test]
fn consolidation_decision_types_are_distinct_for_stats() {
    assert_eq!(ConsolidationDecisionKind::Keep.as_str(), "keep");
    assert_eq!(
        ConsolidationDecisionKind::ArchiveRecommend.as_str(),
        "archive_recommend"
    );
    assert_eq!(
        ConsolidationDecisionKind::ArchiveApply.as_str(),
        "archive_apply"
    );
    assert_eq!(ConsolidationDecisionKind::Quarantine.as_str(), "quarantine");
}

#[test]
fn review_sampling_rate_is_explicit_policy() {
    assert!(!review_required(
        ConsolidationDecisionKind::Keep,
        0.70,
        "rev-left",
        "rev-right",
        0.0,
    ));
    assert!(review_required(
        ConsolidationDecisionKind::Keep,
        0.70,
        "rev-left",
        "rev-right",
        1.0,
    ));
    assert!(review_required(
        ConsolidationDecisionKind::Keep,
        0.54,
        "rev-left",
        "rev-right",
        0.0,
    ));
    assert!(review_required(
        ConsolidationDecisionKind::ArchiveApply,
        0.95,
        "rev-left",
        "rev-right",
        0.0,
    ));
}

#[test]
fn simple_lru_keeps_live_entries_after_repeated_touches() {
    let mut lru = SimpleLru::new(2);
    lru.put("a".to_string(), 1);
    for _ in 0..20 {
        assert_eq!(lru.get(&"a".to_string()), Some(&1));
    }
    lru.put("b".to_string(), 2);
    assert_eq!(lru.get(&"a".to_string()), Some(&1));
    assert_eq!(lru.get(&"b".to_string()), Some(&2));
    lru.put("c".to_string(), 3);
    assert_eq!(lru.get(&"a".to_string()), None);
    assert_eq!(lru.get(&"b".to_string()), Some(&2));
    assert_eq!(lru.get(&"c".to_string()), Some(&3));
}

#[tokio::test]
async fn embedding_breaker_rechecks_inside_candidate_loop() {
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

    let reranker = ProductionEmbeddingReranker::new(
        LlmClient::new(format!("{}/v1", server.uri()), None),
        "text-embedding-3-small".to_string(),
        EmbeddingBudget {
            monthly_embedding_tokens: 50_000,
            soft_cap_pct: 0.08,
            hard_breaker_pct: 0.50,
        },
    );
    let candidates = vec![
        DuplicateCandidate {
            left_revision_id: "rev-l-1".into(),
            right_revision_id: "rev-r-1".into(),
            left_text: "left one".into(),
            right_text: "right one".into(),
            fts_score: 0.8,
            lexical_overlap: 0.4,
            recency_delta_days: 0,
        },
        DuplicateCandidate {
            left_revision_id: "rev-l-2".into(),
            right_revision_id: "rev-r-2".into(),
            left_text: "left two".into(),
            right_text: "right two".into(),
            fts_score: 0.7,
            lexical_overlap: 0.3,
            recency_delta_days: 0,
        },
    ];

    let reranked = reranker
        .rerank("proj-breaker", candidates)
        .await
        .expect("rerank");
    assert!(reranked.iter().all(|r| !r.embedding_used));
    let requests = server.received_requests().await.expect("requests");
    assert_eq!(
        requests.len(),
        1,
        "breaker should stop additional embedding calls after the first usage update"
    );
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
            review_sample_rate: DEFAULT_REVIEW_SAMPLE_RATE,
            project_id: "proj-consolidate".to_string(),
            org_aggregation_enabled: false,
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

    let engine = SqliteConsolidationEngine::new(db.clone()).with_archive_policy(true, 0.40, 0.55);
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

    let engine = SqliteConsolidationEngine::new(db.clone()).with_archive_policy(true, 0.40, 0.55);
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
            review_sample_rate: DEFAULT_REVIEW_SAMPLE_RATE,
            project_id: "proj-conflict".to_string(),
            org_aggregation_enabled: false,
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
            review_sample_rate: DEFAULT_REVIEW_SAMPLE_RATE,
            project_id: "proj-no-conflict".to_string(),
            org_aggregation_enabled: false,
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

    let detector =
        SqliteConflictDetector::new(db.clone(), Arc::new(StubSemanticAdjudicator { score: 0.9 }));
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
                review_sample_rate: DEFAULT_REVIEW_SAMPLE_RATE,
                project_id: project_id.to_string(),
                org_aggregation_enabled: false,
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
            review_sample_rate: DEFAULT_REVIEW_SAMPLE_RATE,
            project_id: "proj-retro".to_string(),
            org_aggregation_enabled: false,
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
            review_sample_rate: DEFAULT_REVIEW_SAMPLE_RATE,
            project_id: "proj-pattern".to_string(),
            org_aggregation_enabled: false,
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
            review_sample_rate: DEFAULT_REVIEW_SAMPLE_RATE,
            project_id: "proj-kd".to_string(),
            org_aggregation_enabled: false,
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
            review_sample_rate: DEFAULT_REVIEW_SAMPLE_RATE,
            project_id: "proj-review".to_string(),
            org_aggregation_enabled: false,
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
            review_sample_rate: DEFAULT_REVIEW_SAMPLE_RATE,
            project_id: "proj-quarantine".to_string(),
            org_aggregation_enabled: false,
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
            review_sample_rate: DEFAULT_REVIEW_SAMPLE_RATE,
            project_id: "proj-taxonomy".to_string(),
            org_aggregation_enabled: false,
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
            && e.data.get("kind").and_then(serde_json::Value::as_str) == Some("curation_decision")
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
    let engine = SqliteConsolidationEngine::new(db.clone()).with_archive_policy(true, 0.40, 0.55);

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

    let engine = SqliteConsolidationEngine::new(db.clone()).with_archive_policy(false, 0.40, 0.55);
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
    let engine = SqliteConsolidationEngine::new(db.clone()).with_archive_policy(true, 0.40, 0.55);
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

// --- Story 5.18: org-wide aggregation flag ---------------------------

/// Seed two playbooks under the SAME tenant but DIFFERENT projects.
/// Phase 4 project-scoped builder finds 0 candidate pairs (different
/// projects); story 5.18 aggregation builder finds 1 pair.
async fn seed_cross_project_pair_same_tenant(db: &DbPool) {
    db.with_conn(|conn| {
            // Two playbooks, same tenant 'tenant-a', different projects.
            conn.execute(
                "INSERT INTO playbooks (id, tenant_id, title, content_path, schema_version, source_task_id, created_at, updated_at, trigger_keywords, content, status, source_project_id, active_revision_id, success_count, failure_count)
                 VALUES ('pb-a', 'tenant-a', 'A', '/tmp/a.md', 1, NULL, 1, 1, '[\"refund\"]', 'Refund flow.', 'active', 'proj-1', 'rev-a-1', 0, 0)",
                [],
            )
            .expect("insert playbook a");
            conn.execute(
                "INSERT INTO playbooks (id, tenant_id, title, content_path, schema_version, source_task_id, created_at, updated_at, trigger_keywords, content, status, source_project_id, active_revision_id, success_count, failure_count)
                 VALUES ('pb-b', 'tenant-a', 'B', '/tmp/b.md', 1, NULL, 1, 1, '[\"refund\"]', 'Refund process.', 'active', 'proj-2', 'rev-b-1', 0, 0)",
                [],
            )
            .expect("insert playbook b");
            conn.execute(
                "INSERT INTO playbook_revisions (id, tenant_id, playbook_id, revision_no, parent_revision_id, title, trigger_keywords, content, source_task_id, source_project_id, author_type, change_kind, confidence, created_at, superseded_at)
                 VALUES ('rev-a-1', 'tenant-a', 'pb-a', 1, NULL, 'A rev', '[\"refund\"]', 'Refund flow.', NULL, 'proj-1', 'extractor', 'extract', 1.0, 1, NULL)",
                [],
            )
            .expect("insert rev a");
            conn.execute(
                "INSERT INTO playbook_revisions (id, tenant_id, playbook_id, revision_no, parent_revision_id, title, trigger_keywords, content, source_task_id, source_project_id, author_type, change_kind, confidence, created_at, superseded_at)
                 VALUES ('rev-b-1', 'tenant-a', 'pb-b', 1, NULL, 'B rev', '[\"refund\"]', 'Refund process.', NULL, 'proj-2', 'extractor', 'extract', 1.0, 1, NULL)",
                [],
            )
            .expect("insert rev b");
        })
        .await;
}

#[tokio::test]
async fn project_scoped_builder_does_not_cross_projects() {
    // Phase 4 behavior — the default `new()` builder must NOT pull
    // candidates from a sibling project, even within the same tenant.
    let db = db::open(":memory:").await.expect("db");
    seed_cross_project_pair_same_tenant(&db).await;
    let builder = SqliteCandidateBuilder::new(db);
    let candidates = builder
        .build_duplicate_candidates("proj-1", 10)
        .await
        .expect("candidates");
    assert_eq!(
        candidates.len(),
        0,
        "project-scoped builder must not surface cross-project pairs"
    );
}

#[tokio::test]
async fn org_aggregation_builder_pulls_from_sibling_projects_in_tenant() {
    // Story 5.18: aggregation builder pinned to 'tenant-a' surfaces
    // the cross-project pair that the project-scoped builder hides.
    let db = db::open(":memory:").await.expect("db");
    seed_cross_project_pair_same_tenant(&db).await;
    let builder = SqliteCandidateBuilder::new_with_org_aggregation(db, "tenant-a".to_string());
    // project_id arg is ignored when aggregation is on — pass an
    // arbitrary one to confirm the builder doesn't filter on it.
    let candidates = builder
        .build_duplicate_candidates("ignored", 10)
        .await
        .expect("candidates");
    assert_eq!(candidates.len(), 1);
    let pair = &candidates[0];
    // Either (rev-a-1, rev-b-1) ordering is acceptable — the SQL
    // sorts by id so left < right.
    assert!(
        (pair.left_revision_id == "rev-a-1" && pair.right_revision_id == "rev-b-1")
            || (pair.left_revision_id == "rev-b-1" && pair.right_revision_id == "rev-a-1")
    );
}

#[tokio::test]
async fn org_aggregation_builder_respects_tenant_boundary() {
    // Cross-tenant pairs must NEVER aggregate, even with org flag on.
    // Seed two playbooks in DIFFERENT tenants; the aggregation
    // builder pinned to 'tenant-a' must ignore 'tenant-b'.
    let db = db::open(":memory:").await.expect("db");
    db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO playbooks (id, tenant_id, title, content_path, schema_version, source_task_id, created_at, updated_at, trigger_keywords, content, status, source_project_id, active_revision_id, success_count, failure_count)
                 VALUES ('pb-a', 'tenant-a', 'A', '/tmp/a.md', 1, NULL, 1, 1, '[]', '', 'active', 'p1', 'rev-a', 0, 0)",
                [],
            ).expect("a");
            conn.execute(
                "INSERT INTO playbooks (id, tenant_id, title, content_path, schema_version, source_task_id, created_at, updated_at, trigger_keywords, content, status, source_project_id, active_revision_id, success_count, failure_count)
                 VALUES ('pb-b', 'tenant-b', 'B', '/tmp/b.md', 1, NULL, 1, 1, '[]', '', 'active', 'p1', 'rev-b', 0, 0)",
                [],
            ).expect("b");
            conn.execute(
                "INSERT INTO playbook_revisions (id, tenant_id, playbook_id, revision_no, parent_revision_id, title, trigger_keywords, content, source_task_id, source_project_id, author_type, change_kind, confidence, created_at, superseded_at)
                 VALUES ('rev-a', 'tenant-a', 'pb-a', 1, NULL, 'A', '[]', '', NULL, 'p1', 'extractor', 'extract', 1.0, 1, NULL)",
                [],
            ).expect("rev-a");
            conn.execute(
                "INSERT INTO playbook_revisions (id, tenant_id, playbook_id, revision_no, parent_revision_id, title, trigger_keywords, content, source_task_id, source_project_id, author_type, change_kind, confidence, created_at, superseded_at)
                 VALUES ('rev-b', 'tenant-b', 'pb-b', 1, NULL, 'B', '[]', '', NULL, 'p1', 'extractor', 'extract', 1.0, 1, NULL)",
                [],
            ).expect("rev-b");
        }).await;

    let builder = SqliteCandidateBuilder::new_with_org_aggregation(db, "tenant-a".to_string());
    let candidates = builder
        .build_duplicate_candidates("ignored", 10)
        .await
        .expect("candidates");
    // 'rev-a' is alone in 'tenant-a' — no pair possible.
    assert_eq!(candidates.len(), 0);
}
