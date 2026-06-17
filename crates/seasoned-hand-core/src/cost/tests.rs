use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;

#[tokio::test]
async fn delta_cents_returns_positive_diff() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/cost"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "total_usd": 1.25,
            "currency": "USD",
        })))
        .expect(1)
        .mount(&mock)
        .await;

    let client = CostClient::new(format!("{}/v1", mock.uri()));
    let baseline = CostSnapshot {
        total_cents: 100,
        currency: "USD".into(),
        ts: 0,
    };

    assert_eq!(client.delta_cents(&baseline).await.unwrap(), 25);
    assert_eq!(client.base_url(), mock.uri());
}

#[tokio::test]
async fn cost_poll_failure_returns_err_caller_tolerates() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/cost"))
        .respond_with(ResponseTemplate::new(503).set_body_string("down"))
        .mount(&mock)
        .await;

    let client = CostClient::new(mock.uri());
    let error = client.snapshot().await.unwrap_err();

    assert!(matches!(error, CostError::Status { code: 503, body: _ }));
}

fn snap(total_cents: i64) -> CostSnapshot {
    CostSnapshot {
        total_cents,
        currency: "USD".into(),
        ts: 0,
    }
}

#[test]
fn delta_between_normal_increase() {
    assert_eq!(delta_between(&snap(100), &snap(175)), 75);
    assert_eq!(delta_between(&snap(0), &snap(0)), 0);
}

#[test]
fn delta_between_rebaselines_on_counter_reset() {
    // Issue #22: when the Bifrost counter resets (current < baseline) the post-reset
    // value is the spend since the reset — bill it, don't mask it as 0.
    assert_eq!(
        delta_between(&snap(100), &snap(7)),
        7,
        "a reset to 7 must bill 7 cents, not 0"
    );
    // A reset to exactly 0 yields 0 (nothing spent yet post-reset).
    assert_eq!(delta_between(&snap(100), &snap(0)), 0);
}
