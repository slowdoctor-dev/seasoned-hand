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
