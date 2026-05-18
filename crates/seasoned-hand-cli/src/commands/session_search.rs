//! `seasoned-hand session search <query>`.

use anyhow::Result;
use clap::Subcommand;
use seasoned_hand_core::db;
use seasoned_hand_core::events::session_search::{
    SessionSearchQuery, search_session_events, summarize_hits_with_fallback,
};
use seasoned_hand_core::events::sqlite::SqliteEventStore;
use seasoned_hand_core::router::SlotRouter;
use serde::Serialize;

#[derive(Debug, Subcommand)]
pub enum SessionCmd {
    /// Search denormalized session index and show raw hits + summary.
    Search {
        query: String,
        #[arg(long)]
        session: String,
        #[arg(long)]
        r#type: Option<String>,
        #[arg(long)]
        source: Option<String>,
        #[arg(long)]
        from: Option<i64>,
        #[arg(long)]
        to: Option<i64>,
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
}

#[derive(Debug, Serialize)]
struct SearchOutput {
    query: String,
    raw_hits: Vec<seasoned_hand_core::events::session_search::EventHit>,
    summary: String,
    degraded: bool,
}

pub async fn run(cmd: SessionCmd, json: bool) -> Result<()> {
    match cmd {
        SessionCmd::Search {
            query,
            session,
            r#type,
            source,
            from,
            to,
            limit,
        } => {
            let pool = db::open(&database_url()).await?;
            let event_store = SqliteEventStore::new(pool.clone());
            let query_for_sql = query.clone();
            let session_for_sql = session.clone();
            let event_type = r#type.as_deref().map(str_to_event_type).transpose()?;
            let hits = pool
                .with_conn(move |conn| {
                    search_session_events(
                        conn,
                        &query_for_sql,
                        &SessionSearchQuery {
                            session_id: Some(session_for_sql.clone()),
                            event_type,
                            source,
                            from_timestamp: from,
                            to_timestamp: to,
                            limit: Some(limit),
                        },
                    )
                })
                .await?;
            let router = load_router();
            let summary =
                summarize_hits_with_fallback(&event_store, &router, &session, &query, &hits).await;
            let out = SearchOutput {
                query,
                raw_hits: hits,
                summary: summary.summary,
                degraded: summary.degraded,
            };
            if json {
                println!("{}", serde_json::to_string_pretty(&out)?);
            } else {
                println!("summary:\n{}\n", out.summary);
                for h in out.raw_hits {
                    println!(
                        "{} {} {} {}",
                        h.timestamp, h.event_type, h.source, h.snippet
                    );
                }
            }
        }
    }
    Ok(())
}

fn database_url() -> String {
    std::env::var("SH_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .unwrap_or_else(|_| "sqlite:./data/seasoned-hand.db".to_string())
}

fn load_router() -> SlotRouter {
    if let Ok(path) = std::env::var("SH_SLOTS_YAML")
        && let Ok(router) = SlotRouter::from_yaml(path)
    {
        return router;
    }
    SlotRouter::default_for_bifrost()
}

fn str_to_event_type(s: &str) -> Result<seasoned_hand_core::events::EventType> {
    use std::str::FromStr;
    Ok(seasoned_hand_core::events::EventType::from_str(s)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use seasoned_hand_core::events::{EventStore, EventType, NewEvent};
    use serde_json::json;

    // Serial-group key matches the env var this test mutates; the `sop` and
    // `playbook` tests share the same key for mutual exclusion under cargo's
    // parallel test runner (Phase 3 REVIEW iter-1 F1).
    #[tokio::test]
    #[serial_test::serial(SH_DATABASE_URL)]
    async fn search_returns_raw_hits() {
        let tmp = tempfile::tempdir().unwrap();
        let db_url = format!("sqlite:{}", tmp.path().join("session-search.db").display());
        unsafe { std::env::set_var("SH_DATABASE_URL", &db_url) };

        let pool = db::open(&db_url).await.unwrap();
        pool.with_conn(|conn| {
            conn.execute(
                "INSERT INTO sessions (id, created_at, updated_at, state) VALUES ('s1',1,1,'RUNNING')",
                [],
            )
            .unwrap();
        })
        .await;
        let store = SqliteEventStore::new(pool.clone());
        store
            .append(NewEvent {
                session_id: "s1".into(),
                event_type: EventType::Action,
                source: "tool:web".into(),
                data: json!({"tool_name":"web_search","tool_input":{"query":"needle_alpha"}}),
            })
            .await
            .unwrap();

        run(
            SessionCmd::Search {
                query: "needle_alpha".into(),
                session: "s1".into(),
                r#type: None,
                source: None,
                from: None,
                to: None,
                limit: 20,
            },
            true,
        )
        .await
        .unwrap();

        unsafe { std::env::remove_var("SH_DATABASE_URL") };
    }
}
