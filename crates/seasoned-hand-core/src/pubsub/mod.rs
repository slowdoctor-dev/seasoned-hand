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

    /// `XGROUP CREATE <stream> <group> $ MKSTREAM`. Idempotent: a
    /// pre-existing group (`BUSYGROUP` error) is swallowed. `MKSTREAM`
    /// creates the stream if it doesn't yet exist.
    pub async fn xgroup_create_mkstream(
        &self,
        stream: &str,
        group: &str,
    ) -> Result<(), RedisError> {
        let mut conn = self.conn().await?;
        let res: redis::RedisResult<()> = redis::cmd("XGROUP")
            .arg("CREATE")
            .arg(stream)
            .arg(group)
            .arg("$")
            .arg("MKSTREAM")
            .query_async(&mut *conn)
            .await;
        match res {
            Ok(()) => Ok(()),
            Err(err) if err.code() == Some("BUSYGROUP") => Ok(()),
            Err(err) => Err(err.into()),
        }
    }

    /// `XREADGROUP GROUP <group> <consumer> COUNT <count> BLOCK <block_ms>
    /// STREAMS <stream> >`. Returns `(message_id, payload_bytes)` pairs
    /// for each entry whose `payload` field is set; entries without a
    /// `payload` field are skipped. An empty/nil reply (no entries within
    /// the block window) returns `Ok(vec![])`.
    ///
    /// The reply is parsed directly off `redis::Value` so we don't have
    /// to enable the upstream `streams` feature (and therefore avoid
    /// changing workspace dependencies).
    pub async fn xreadgroup_payloads(
        &self,
        stream: &str,
        group: &str,
        consumer: &str,
        count: usize,
        block_ms: usize,
    ) -> Result<Vec<(String, Vec<u8>)>, RedisError> {
        let mut conn = self.conn().await?;
        let reply: redis::Value = redis::cmd("XREADGROUP")
            .arg("GROUP")
            .arg(group)
            .arg(consumer)
            .arg("COUNT")
            .arg(count)
            .arg("BLOCK")
            .arg(block_ms)
            .arg("STREAMS")
            .arg(stream)
            .arg(">")
            .query_async(&mut *conn)
            .await?;
        Ok(parse_xreadgroup_reply(&reply))
    }

    /// `XACK <stream> <group> <id>`. Returns the number of messages
    /// acknowledged (0 if the id was already acked or never in the PEL).
    pub async fn xack(&self, stream: &str, group: &str, id: &str) -> Result<i64, RedisError> {
        let mut conn = self.conn().await?;
        let n: i64 = redis::cmd("XACK")
            .arg(stream)
            .arg(group)
            .arg(id)
            .query_async(&mut *conn)
            .await?;
        Ok(n)
    }

    /// Count of pending (unacked) entries for `group` on `stream`. Reads
    /// `XPENDING <stream> <group>`'s summary form, which begins with the
    /// pending count as its first element. Returns 0 when the group has
    /// no pending entries.
    pub async fn xpending_count(&self, stream: &str, group: &str) -> Result<i64, RedisError> {
        let mut conn = self.conn().await?;
        let val: redis::Value = redis::cmd("XPENDING")
            .arg(stream)
            .arg(group)
            .query_async(&mut *conn)
            .await?;
        if let redis::Value::Bulk(items) = &val
            && let Some(redis::Value::Int(n)) = items.first()
        {
            return Ok(*n);
        }
        Ok(0)
    }
}

pub fn channel_for(session_id: &str) -> String {
    format!("sh:events:{session_id}")
}

/// Walk an `XREADGROUP` reply value and extract `(entry_id, payload)`
/// pairs. The reply nests as
/// `[ [stream_name, [[id, [k, v, k, v, ...]], ...]], ... ]`; we only
/// surface the `payload` field per entry (XADD-side writes a single
/// `payload` field — see [`RedisPool::xadd_json`]).
fn parse_xreadgroup_reply(reply: &redis::Value) -> Vec<(String, Vec<u8>)> {
    let streams = match reply {
        redis::Value::Bulk(items) => items,
        _ => return Vec::new(),
    };
    let mut out: Vec<(String, Vec<u8>)> = Vec::new();
    for stream_entry in streams {
        let redis::Value::Bulk(stream_parts) = stream_entry else {
            continue;
        };
        // stream_parts = [stream_name, [entries]]; we only need entries.
        let entries = match stream_parts.get(1) {
            Some(redis::Value::Bulk(e)) => e,
            _ => continue,
        };
        for entry in entries {
            let redis::Value::Bulk(entry_parts) = entry else {
                continue;
            };
            let Some(redis::Value::Data(id_bytes)) = entry_parts.first() else {
                continue;
            };
            let Some(redis::Value::Bulk(fields)) = entry_parts.get(1) else {
                continue;
            };
            // fields = [k1, v1, k2, v2, ...] — find "payload"
            let mut idx = 0;
            while idx + 1 < fields.len() {
                let key_is_payload = matches!(
                    &fields[idx],
                    redis::Value::Data(b) if b == b"payload",
                );
                if key_is_payload && let redis::Value::Data(val) = &fields[idx + 1] {
                    out.push((String::from_utf8_lossy(id_bytes).into_owned(), val.clone()));
                    break;
                }
                idx += 2;
            }
        }
    }
    out
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
