//! Redis pub/sub for live event fanout.
//! refs: /specs/phase-0/architecture.md §1, §5.1, §5.2

use deadpool_redis::redis::{self, AsyncCommands};
use deadpool_redis::{Config, Connection, Pool, Runtime};
use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum RedisError {
    #[error("pool error: {0}")]
    Pool(#[from] deadpool_redis::PoolError),
    #[error("redis error: {0}")]
    Redis(#[from] redis::RedisError),
    #[error("config error: {0}")]
    Config(String),
}

#[derive(Clone)]
pub struct RedisPool {
    pool: Pool,
    pub url: String,
}

impl RedisPool {
    pub fn new(url: impl Into<String>) -> Result<Self, RedisError> {
        let url = url.into();
        let cfg = Config::from_url(url.clone());
        let pool = cfg
            .create_pool(Some(Runtime::Tokio1))
            .map_err(|e| RedisError::Config(e.to_string()))?;
        Ok(Self { pool, url })
    }

    async fn conn(&self) -> Result<Connection, RedisError> {
        Ok(self.pool.get().await?)
    }

    pub async fn ping(&self) -> Result<(), RedisError> {
        let mut conn = self.conn().await?;
        let _: String = redis::cmd("PING").query_async(&mut *conn).await?;
        Ok(())
    }

    pub async fn publish_event(&self, session_id: &str, payload: &str) -> Result<i64, RedisError> {
        let mut conn = self.conn().await?;
        let channel = channel_for(session_id);
        let n: i64 = conn.publish(channel, payload).await?;
        Ok(n)
    }

    pub async fn subscribe(&self, session_id: &str) -> Result<EventSubscription, RedisError> {
        // Pub/Sub requires a dedicated connection (PubSub mode monopolizes
        // the socket), so we go around the pool.
        let client = redis::Client::open(self.url.clone())?;
        let mut pubsub = client.get_async_pubsub().await?;
        pubsub.subscribe(channel_for(session_id)).await?;
        Ok(EventSubscription { pubsub })
    }

    pub async fn xadd_json<T: Serialize>(
        &self,
        stream: &str,
        payload: &T,
    ) -> Result<String, RedisError> {
        let mut conn = self.conn().await?;
        let body = serde_json::to_string(payload)
            .map_err(|e| RedisError::Config(format!("serialize xadd payload: {e}")))?;
        let id: String = redis::cmd("XADD")
            .arg(stream)
            .arg("*")
            .arg("payload")
            .arg(body)
            .query_async(&mut *conn)
            .await?;
        Ok(id)
    }

    pub async fn xlen(&self, stream: &str) -> Result<i64, RedisError> {
        let mut conn = self.conn().await?;
        let n: i64 = redis::cmd("XLEN")
            .arg(stream)
            .query_async(&mut *conn)
            .await?;
        Ok(n)
    }
}

pub fn channel_for(session_id: &str) -> String {
    format!("sh:events:{session_id}")
}

pub struct EventSubscription {
    pubsub: redis::aio::PubSub,
}

impl EventSubscription {
    /// Consume the subscription as a stream of JSON payloads.
    pub fn into_stream(self) -> impl futures_util::Stream<Item = String> {
        use futures_util::StreamExt;
        self.pubsub
            .into_on_message()
            .filter_map(|msg| async move { msg.get_payload::<String>().ok() })
    }
}

#[cfg(test)]
mod tests;
