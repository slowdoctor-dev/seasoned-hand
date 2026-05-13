//! Story 2.20 — verifies the NarratorHook's classifier slot can be
//! attached at boot via `AppState::with_narrator_classifier` AFTER
//! the dispatcher already has the templated-only NarratorHook in its
//! hook chain. Closes the Phase 1 1.15 Execution-notes deferral.

use std::sync::Arc;

use seasoned_hand_core::llm::LlmClient;
use seasoned_hand_core::router::SlotRouter;
use seasoned_hand_core::sandbox::SandboxClient;
use seasoned_hand_core::search::{SearchClient, SearchProvider};
use seasoned_hand_core::{db, pubsub};
use seasoned_hand_server::{AppState, NarratorClassifierWiring};

async fn empty_state() -> AppState {
    let pool = db::open(":memory:").await.expect("db");
    let redis = pubsub::RedisPool::new("redis://127.0.0.1:6").expect("redis url");
    let sandbox = SandboxClient::new(
        "ghcr.io/agent-infra/sandbox:1.0.0.152",
        std::env::temp_dir(),
    )
    .expect("sandbox client");
    let search = SearchClient::new(SearchProvider::Brave { api_key: None });
    let router = SlotRouter::default_for_bifrost();
    AppState::new(pool, redis, sandbox, search, router, Default::default())
}

#[tokio::test]
async fn narrator_starts_with_no_classifier() {
    let state = empty_state().await;
    assert!(
        !state.narrator.classifier_is_attached(),
        "NarratorHook should be templated-only at AppState::new — story 1.15 invariant"
    );
}

#[tokio::test]
async fn with_narrator_classifier_attaches() {
    let state = empty_state().await;
    let llm = Arc::new(LlmClient::new("http://127.0.0.1:1", None));
    let state = state.with_narrator_classifier(NarratorClassifierWiring {
        llm,
        model: "classifier-mini".into(),
        system_prompt: Arc::new("You narrate tool calls.".into()),
    });
    assert!(
        state.narrator.classifier_is_attached(),
        "with_narrator_classifier must populate the OnceLock"
    );
}

#[tokio::test]
async fn second_with_narrator_classifier_is_no_op() {
    // OnceLock semantics: the first `attach_classifier` wins. The
    // builder swallows the Err from the second call (logs a warn).
    // No panic; the attached state stays from the first call.
    let state = empty_state().await;
    let llm1 = Arc::new(LlmClient::new("http://127.0.0.1:1", None));
    let llm2 = Arc::new(LlmClient::new("http://127.0.0.1:2", None));
    let state = state
        .with_narrator_classifier(NarratorClassifierWiring {
            llm: llm1,
            model: "first".into(),
            system_prompt: Arc::new("first prompt".into()),
        })
        .with_narrator_classifier(NarratorClassifierWiring {
            llm: llm2,
            model: "second".into(),
            system_prompt: Arc::new("second prompt".into()),
        });
    assert!(state.narrator.classifier_is_attached());
}
