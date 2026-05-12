//! Pub/sub round-trip tests against a live Redis at REDIS_TEST_URL
//! (default redis://127.0.0.1:6379). Tests are #[ignore]'d by default;
//! run with `cargo test -- --ignored` once Redis is up.
//!
//! `docker compose up -d redis` is the standard way to provide one.

use std::time::Duration;

use futures_util::StreamExt;

use super::{RedisPool, channel_for};

fn test_url() -> String {
    std::env::var("REDIS_TEST_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".into())
}

#[tokio::test]
#[ignore = "requires running Redis"]
async fn pool_ping_works() {
    let pool = RedisPool::new(test_url()).unwrap();
    pool.ping().await.expect("ping ok");
}

#[tokio::test]
#[ignore = "requires running Redis"]
async fn publish_then_receive_round_trip() {
    let pool = RedisPool::new(test_url()).unwrap();
    let sub = pool.subscribe("session-rt-1").await.unwrap();
    let mut stream = Box::pin(sub.into_stream());

    // Give the subscriber a moment to register before publishing.
    tokio::time::sleep(Duration::from_millis(50)).await;

    let received = pool.publish_event("session-rt-1", "hello").await.unwrap();
    assert!(received >= 1, "expected at least one subscriber");

    let got = tokio::time::timeout(Duration::from_secs(2), stream.next())
        .await
        .expect("recv before timeout")
        .expect("stream item");
    assert_eq!(got, "hello");
}

#[tokio::test]
#[ignore = "requires running Redis"]
async fn subscribe_isolates_by_session() {
    let pool = RedisPool::new(test_url()).unwrap();
    let sub_a = pool.subscribe("session-iso-a").await.unwrap();
    let mut stream_a = Box::pin(sub_a.into_stream());
    tokio::time::sleep(Duration::from_millis(50)).await;

    pool.publish_event("session-iso-b", "for-b").await.unwrap();
    pool.publish_event("session-iso-a", "for-a").await.unwrap();

    let got = tokio::time::timeout(Duration::from_secs(2), stream_a.next())
        .await
        .expect("recv before timeout")
        .expect("stream item");
    assert_eq!(got, "for-a", "session A should only receive A's messages");
}

#[test]
fn channel_naming_is_namespaced() {
    assert_eq!(channel_for("abc"), "sh:events:abc");
}
