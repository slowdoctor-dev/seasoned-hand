use crate::time::now_micros;
use rusqlite::{Connection, OptionalExtension, params};
use unicode_normalization::UnicodeNormalization;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatcherMode {
    Gate,
    Production,
}

impl MatcherMode {
    pub fn as_str(self) -> &'static str {
        match self {
            MatcherMode::Gate => "gate",
            MatcherMode::Production => "production",
        }
    }
}

#[derive(Debug, Clone)]
pub struct MatchRequest {
    pub session_id: String,
    pub fixture_id: Option<String>,
    pub brief: String,
    pub mode: MatcherMode,
    pub limit: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MatchedPlaybook {
    pub playbook_id: String,
    pub title: String,
    pub content: String,
    pub content_excerpt: String,
    pub matcher_mode: MatcherMode,
    pub match_score: f64,
    pub success_count: i64,
    pub failure_count: i64,
}

#[derive(Debug, Clone)]
struct Candidate {
    playbook_id: String,
    title: String,
    content: String,
    trigger_keywords: String,
    success_count: i64,
    failure_count: i64,
    created_at: i64,
}

pub fn normalize_brief(input: &str) -> String {
    let nfd: String = input.nfd().collect();
    let lowered = nfd.to_ascii_lowercase();
    lowered.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn match_playbooks(
    conn: &Connection,
    request: &MatchRequest,
) -> rusqlite::Result<Vec<MatchedPlaybook>> {
    let project_id: Option<String> = conn
        .query_row(
            "SELECT t.project_id
             FROM sessions s
             JOIN tasks t ON t.id = s.task_id
             WHERE s.id = ?",
            [request.session_id.as_str()],
            |r| r.get(0),
        )
        .optional()?;
    let Some(project_id) = project_id else {
        return Ok(Vec::new());
    };
    match request.mode {
        MatcherMode::Gate => gate_match(conn, &project_id, request),
        MatcherMode::Production => production_match(conn, &project_id, request),
    }
}

fn gate_match(
    conn: &Connection,
    project_id: &str,
    request: &MatchRequest,
) -> rusqlite::Result<Vec<MatchedPlaybook>> {
    let Some(fixture_id) = request.fixture_id.as_deref() else {
        return Ok(Vec::new());
    };
    let normalized_brief = normalize_brief(&request.brief);
    if normalized_brief.is_empty() {
        return Ok(Vec::new());
    }
    let fixture_key = format!("fixture:{fixture_id}");
    let brief_key = format!("brief:{}", normalized_brief);

    let mut stmt = conn.prepare(
        "SELECT p.id, p.title, p.content, p.trigger_keywords, p.success_count, p.failure_count, p.created_at
         FROM playbooks p
         JOIN tasks src ON src.id = p.source_task_id
         WHERE src.project_id = ?
           AND p.status = 'active'
           AND lower(p.trigger_keywords) LIKE '%' || lower(?) || '%'
           AND lower(p.trigger_keywords) LIKE '%' || lower(?) || '%'",
    )?;
    let rows = stmt.query_map(params![project_id, fixture_key, brief_key], |r| {
        Ok(Candidate {
            playbook_id: r.get(0)?,
            title: r.get(1)?,
            content: r.get(2)?,
            trigger_keywords: r.get(3)?,
            success_count: r.get(4)?,
            failure_count: r.get(5)?,
            created_at: r.get(6)?,
        })
    })?;
    let mut candidates = Vec::new();
    for row in rows {
        candidates.push(row?);
    }

    let now_micros = now_micros();
    let mut scored = candidates
        .into_iter()
        .map(|c| score_candidate(c, MatcherMode::Gate, &[], now_micros))
        .collect::<Vec<_>>();
    scored.sort_by(ranking_cmp);
    scored.truncate(request.limit.min(3));
    Ok(scored)
}

fn production_match(
    conn: &Connection,
    project_id: &str,
    request: &MatchRequest,
) -> rusqlite::Result<Vec<MatchedPlaybook>> {
    let normalized = normalize_brief(&request.brief);
    if normalized.is_empty() {
        return Ok(Vec::new());
    }
    let tokens = normalized
        .split(' ')
        .filter(|t| !t.is_empty())
        .collect::<Vec<_>>();
    if tokens.is_empty() {
        return Ok(Vec::new());
    }
    let fts_query = tokens
        .iter()
        .map(|t| format!("{t}*"))
        .collect::<Vec<_>>()
        .join(" ");

    let mut stmt = conn.prepare(
        "SELECT p.id, p.title, p.content, p.trigger_keywords, p.success_count, p.failure_count, p.created_at
         FROM playbooks_fts
         JOIN playbooks p ON p.rowid = playbooks_fts.rowid
         JOIN tasks src ON src.id = p.source_task_id
         WHERE playbooks_fts MATCH ?
           AND src.project_id = ?
           AND p.status = 'active'",
    )?;
    let rows = stmt.query_map(params![fts_query, project_id], |r| {
        Ok(Candidate {
            playbook_id: r.get(0)?,
            title: r.get(1)?,
            content: r.get(2)?,
            trigger_keywords: r.get(3)?,
            success_count: r.get(4)?,
            failure_count: r.get(5)?,
            created_at: r.get(6)?,
        })
    })?;
    let mut candidates = Vec::new();
    for row in rows {
        candidates.push(row?);
    }

    let now_micros = now_micros();
    let mut scored = candidates
        .into_iter()
        .map(|c| score_candidate(c, MatcherMode::Production, &tokens, now_micros))
        .filter(|m| m.match_score >= 1.0)
        .collect::<Vec<_>>();
    scored.sort_by(ranking_cmp);
    scored.truncate(request.limit.min(3));
    Ok(scored)
}

fn score_candidate(
    candidate: Candidate,
    mode: MatcherMode,
    tokens: &[&str],
    now_micros: i64,
) -> MatchedPlaybook {
    let content_excerpt = candidate.content.chars().take(512).collect::<String>();
    let match_score = match mode {
        MatcherMode::Gate => 10.0 + recency_boost(now_micros, candidate.created_at),
        MatcherMode::Production => {
            let kw_hits = prefix_hits(&candidate.trigger_keywords, tokens) as f64;
            let title_hits = prefix_hits(&candidate.title, tokens) as f64;
            let content_hits = prefix_hits(&candidate.content, tokens) as f64;
            (5.0 * kw_hits)
                + (3.0 * title_hits)
                + (1.0 * content_hits)
                + recency_boost(now_micros, candidate.created_at)
        }
    };
    MatchedPlaybook {
        playbook_id: candidate.playbook_id,
        title: candidate.title,
        content: candidate.content.clone(),
        content_excerpt,
        matcher_mode: mode,
        match_score,
        success_count: candidate.success_count,
        failure_count: candidate.failure_count,
    }
}

fn ranking_cmp(a: &MatchedPlaybook, b: &MatchedPlaybook) -> std::cmp::Ordering {
    b.match_score
        .total_cmp(&a.match_score)
        .then_with(|| {
            let a_delta = a.success_count - a.failure_count;
            let b_delta = b.success_count - b.failure_count;
            b_delta.cmp(&a_delta)
        })
        .then_with(|| b.success_count.cmp(&a.success_count))
        .then_with(|| a.playbook_id.cmp(&b.playbook_id))
}

fn prefix_hits(field: &str, tokens: &[&str]) -> usize {
    let words = normalize_brief(field)
        .split(' ')
        .filter(|w| !w.is_empty())
        .map(|w| w.to_string())
        .collect::<Vec<_>>();
    tokens
        .iter()
        .filter(|token| words.iter().any(|word| word.starts_with(**token)))
        .count()
}

fn recency_boost(now_micros: i64, created_at: i64) -> f64 {
    let age_days = age_days(now_micros, created_at);
    (0.5_f64 - (age_days / 60.0)).max(0.0)
}

fn age_days(now_micros: i64, created_at: i64) -> f64 {
    if created_at <= 0 || now_micros <= created_at {
        return 0.0;
    }
    let delta = now_micros - created_at;
    delta as f64 / 86_400_000_000.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    #[test]
    fn normalization() {
        let input = "  Café\t\n  PLAN  ";
        assert_eq!(normalize_brief(input), "café plan");
    }

    #[tokio::test]
    async fn gate_identity() {
        let pool = db::open(":memory:").await.unwrap();
        pool.with_conn(|conn| {
            conn.execute(
                "INSERT INTO sessions (id, created_at, updated_at, state)
                 VALUES ('s1', 1, 1, 'RUNNING')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO projects (id, tenant_id, status, title, created_at, updated_at)
                 VALUES ('p1', 'legacy-default', 'active', 'P1', 1, 1)",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO tasks (id, project_id, tenant_id, status, title, created_at, updated_at)
                 VALUES ('t1', 'p1', 'legacy-default', 'Running', 'Task 1', 1, 1)",
                [],
            )
            .unwrap();
            conn.execute("UPDATE sessions SET task_id = 't1' WHERE id = 's1'", [])
                .unwrap();
            conn.execute(
                "INSERT INTO playbooks (id, tenant_id, title, content_path, schema_version, source_task_id,
                 created_at, updated_at, trigger_keywords, content, success_count, failure_count, avg_duration_ms,
                 avg_tool_calls, status, version)
                 VALUES ('pb-1', NULL, 'Gate Hit', '', 1, 't1', 1, 1,
                 '[\"fixture:phase2_overnight_default_path\", \"brief:hello world\"]',
                 'body', 2, 0, NULL, NULL, 'active', 1)",
                [],
            )
            .unwrap();
        })
        .await;

        let out = pool
            .with_conn(|conn| {
                match_playbooks(
                    conn,
                    &MatchRequest {
                        session_id: "s1".into(),
                        fixture_id: Some("phase2_overnight_default_path".into()),
                        brief: "  Hello   WORLD ".into(),
                        mode: MatcherMode::Gate,
                        limit: 3,
                    },
                )
            })
            .await
            .unwrap();

        assert_eq!(out.len(), 1);
        assert_eq!(out[0].playbook_id, "pb-1");
        assert_eq!(out[0].matcher_mode, MatcherMode::Gate);
    }

    #[tokio::test]
    async fn fts_ranking_determinism() {
        let pool = db::open(":memory:").await.unwrap();
        pool.with_conn(|conn| {
            conn.execute(
                "INSERT INTO sessions (id, created_at, updated_at, state)
                 VALUES ('s1', 1, 1, 'RUNNING')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO projects (id, tenant_id, status, title, created_at, updated_at)
                 VALUES ('p1', 'legacy-default', 'active', 'P1', 1, 1)",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO tasks (id, project_id, tenant_id, status, title, created_at, updated_at)
                 VALUES ('t1', 'p1', 'legacy-default', 'Running', 'Task 1', 1, 1)",
                [],
            )
            .unwrap();
            conn.execute("UPDATE sessions SET task_id = 't1' WHERE id = 's1'", [])
                .unwrap();
            for (id, kw, title, content, succ, fail) in [
                ("pb-a", "[\"deploy\"]", "Deploy checklist", "deploy system rollout", 10, 5),
                ("pb-b", "[\"deploy\"]", "Deploy runbook", "deploy system rollout", 20, 15),
                ("pb-c", "[\"deploy\"]", "Deploy runbook", "deploy system rollout", 1, 0),
            ] {
                conn.execute(
                    "INSERT INTO playbooks (id, tenant_id, title, content_path, schema_version, source_task_id,
                     created_at, updated_at, trigger_keywords, content, success_count, failure_count, avg_duration_ms,
                     avg_tool_calls, status, version)
                     VALUES (?, NULL, ?, '', 1, 't1', 1, 1, ?, ?, ?, ?, NULL, NULL, 'active', 1)",
                    params![id, title, kw, content, succ, fail],
                )
                .unwrap();
            }
        })
        .await;

        let out = pool
            .with_conn(|conn| {
                match_playbooks(
                    conn,
                    &MatchRequest {
                        session_id: "s1".into(),
                        fixture_id: None,
                        brief: "deploy".into(),
                        mode: MatcherMode::Production,
                        limit: 3,
                    },
                )
            })
            .await
            .unwrap();

        assert_eq!(out.len(), 3);
        // delta tie: pb-c (1) > pb-a (5) ? No, pb-a delta=5, pb-b delta=5, pb-c delta=1.
        // tie on delta between a and b => tertiary success_count desc => b then a.
        assert_eq!(out[0].playbook_id, "pb-b");
        assert_eq!(out[1].playbook_id, "pb-a");
        assert_eq!(out[2].playbook_id, "pb-c");
        assert!(out[0].match_score >= 1.0);
    }

    #[tokio::test]
    async fn exclude_archived() {
        let pool = db::open(":memory:").await.unwrap();
        pool.with_conn(|conn| {
            conn.execute(
                "INSERT INTO sessions (id, created_at, updated_at, state)
                 VALUES ('s1', 1, 1, 'RUNNING')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO projects (id, tenant_id, status, title, created_at, updated_at)
                 VALUES ('p1', 'legacy-default', 'active', 'P1', 1, 1)",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO tasks (id, project_id, tenant_id, status, title, created_at, updated_at)
                 VALUES ('t1', 'p1', 'legacy-default', 'Running', 'Task 1', 1, 1)",
                [],
            )
            .unwrap();
            conn.execute("UPDATE sessions SET task_id = 't1' WHERE id = 's1'", [])
                .unwrap();

            conn.execute(
                "INSERT INTO playbooks (id, tenant_id, title, content_path, schema_version, source_task_id,
                 created_at, updated_at, trigger_keywords, content, success_count, failure_count, avg_duration_ms,
                 avg_tool_calls, status, version)
                 VALUES ('pb-active', NULL, 'Deploy active', '', 1, 't1', 1, 1,
                 '[\"deploy\"]', 'deploy checklist', 1, 0, NULL, NULL, 'active', 1)",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO playbooks (id, tenant_id, title, content_path, schema_version, source_task_id,
                 created_at, updated_at, trigger_keywords, content, success_count, failure_count, avg_duration_ms,
                 avg_tool_calls, status, version)
                 VALUES ('pb-archived', NULL, 'Deploy old', '', 1, 't1', 1, 1,
                 '[\"deploy\"]', 'deploy checklist old', 10, 0, NULL, NULL, 'archived', 1)",
                [],
            )
            .unwrap();
        })
        .await;

        let out = pool
            .with_conn(|conn| {
                match_playbooks(
                    conn,
                    &MatchRequest {
                        session_id: "s1".into(),
                        fixture_id: None,
                        brief: "deploy".into(),
                        mode: MatcherMode::Production,
                        limit: 3,
                    },
                )
            })
            .await
            .unwrap();

        assert_eq!(out.len(), 1);
        assert_eq!(out[0].playbook_id, "pb-active");
    }

    #[tokio::test]
    async fn phase3_production_matcher_smoke() {
        let pool = db::open(":memory:").await.unwrap();
        pool.with_conn(|conn| {
            conn.execute(
                "INSERT INTO sessions (id, created_at, updated_at, state)
                 VALUES ('s1', 1, 1, 'RUNNING')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO sessions (id, created_at, updated_at, state)
                 VALUES ('s2', 1, 1, 'RUNNING')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO projects (id, tenant_id, status, title, created_at, updated_at)
                 VALUES ('p1', 'legacy-default', 'active', 'P1', 1, 1)",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO projects (id, tenant_id, status, title, created_at, updated_at)
                 VALUES ('p2', 'legacy-default', 'active', 'P2', 1, 1)",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO tasks (id, project_id, tenant_id, status, title, created_at, updated_at)
                 VALUES ('t1', 'p1', 'legacy-default', 'Running', 'Task 1', 1, 1)",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO tasks (id, project_id, tenant_id, status, title, created_at, updated_at)
                 VALUES ('t2', 'p2', 'legacy-default', 'Running', 'Task 2', 1, 1)",
                [],
            )
            .unwrap();
            conn.execute("UPDATE sessions SET task_id = 't1' WHERE id = 's1'", [])
                .unwrap();
            conn.execute("UPDATE sessions SET task_id = 't2' WHERE id = 's2'", [])
                .unwrap();

            // Same-project active candidates.
            conn.execute(
                "INSERT INTO playbooks (id, tenant_id, title, content_path, schema_version, source_task_id,
                 created_at, updated_at, trigger_keywords, content, success_count, failure_count, avg_duration_ms,
                 avg_tool_calls, status, version)
                 VALUES ('pb-top', NULL, 'Deploy gold runbook', '', 1, 't1', 1, 1,
                 '[\"deploy\", \"rollout\"]', 'deploy rollout checklist', 30, 5, NULL, NULL, 'active', 1)",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO playbooks (id, tenant_id, title, content_path, schema_version, source_task_id,
                 created_at, updated_at, trigger_keywords, content, success_count, failure_count, avg_duration_ms,
                 avg_tool_calls, status, version)
                 VALUES ('pb-mid', NULL, 'Deploy service', '', 1, 't1', 1, 1,
                 '[\"deploy\"]', 'deploy service instructions', 10, 2, NULL, NULL, 'active', 1)",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO playbooks (id, tenant_id, title, content_path, schema_version, source_task_id,
                 created_at, updated_at, trigger_keywords, content, success_count, failure_count, avg_duration_ms,
                 avg_tool_calls, status, version)
                 VALUES ('pb-low', NULL, 'Service notes', '', 1, 't1', 1, 1,
                 '[\"deploy\"]', 'service deployment references', 1, 0, NULL, NULL, 'active', 1)",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO playbooks (id, tenant_id, title, content_path, schema_version, source_task_id,
                 created_at, updated_at, trigger_keywords, content, success_count, failure_count, avg_duration_ms,
                 avg_tool_calls, status, version)
                 VALUES ('pb-archived', NULL, 'Deploy old', '', 1, 't1', 1, 1,
                 '[\"deploy\"]', 'legacy deploy path', 999, 1, NULL, NULL, 'archived', 1)",
                [],
            )
            .unwrap();

            // Cross-project candidate that should never surface for s1.
            conn.execute(
                "INSERT INTO playbooks (id, tenant_id, title, content_path, schema_version, source_task_id,
                 created_at, updated_at, trigger_keywords, content, success_count, failure_count, avg_duration_ms,
                 avg_tool_calls, status, version)
                 VALUES ('pb-foreign', NULL, 'Deploy foreign', '', 1, 't2', 1, 1,
                 '[\"deploy\"]', 'foreign project deploy', 50, 0, NULL, NULL, 'active', 1)",
                [],
            )
            .unwrap();
        })
        .await;

        let out = pool
            .with_conn(|conn| {
                match_playbooks(
                    conn,
                    &MatchRequest {
                        session_id: "s1".into(),
                        fixture_id: None,
                        brief: "deploy".into(),
                        mode: MatcherMode::Production,
                        limit: 3,
                    },
                )
            })
            .await
            .unwrap();

        assert_eq!(out.len(), 3, "expected top-3 production matches");
        assert_eq!(out[0].playbook_id, "pb-top");
        assert_eq!(out[1].playbook_id, "pb-mid");
        assert_eq!(out[2].playbook_id, "pb-low");

        let ids = out
            .iter()
            .map(|m| m.playbook_id.as_str())
            .collect::<Vec<_>>();
        assert!(
            !ids.contains(&"pb-archived"),
            "archived rows must be excluded from production matcher"
        );
        assert!(
            !ids.contains(&"pb-foreign"),
            "cross-project rows must be excluded from production matcher"
        );
    }
}
