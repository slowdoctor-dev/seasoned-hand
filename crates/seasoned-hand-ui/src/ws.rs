//! WebSocket client for the agent event stream. Rust port of
//! `frontend/lib/ws.ts`: connect, subscribe, push events into a signal, reply
//! to server pings, and reconnect with exponential backoff + subscription
//! replay.
//!
//! Simplification vs the TS original (tracked as a Phase 6 follow-up): commands
//! are fire-and-forget — the `ack`-await round-trip (resolving a per-command
//! promise by `ref`) is not yet reimplemented; acks are currently ignored.

use crate::config::ws_url;
use dioxus::prelude::*;
use futures::channel::mpsc::UnboundedReceiver;
use futures::stream::SplitSink;
use futures::{select, FutureExt, SinkExt, StreamExt};
use gloo_net::websocket::{futures::WebSocket, Message};
use gloo_timers::future::TimeoutFuture;
use seasoned_hand_dto::{ClientCommand, CommandPayload, ServerEnvelope, ServerEvent};
use std::collections::HashMap;

const EVENTS_CAP: usize = 1000;
const BACKOFF_BASE_MS: u32 = 1000;
const BACKOFF_MAX_MS: u32 = 30_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WsStatus {
    Connecting,
    Open,
    Closed,
    Reconnecting,
}

/// Handle to the live socket: reactive status + event buffer, plus a coroutine
/// channel to send [`CommandPayload`]s. Cheap to clone (signals + coroutine are
/// handles); shared through Dioxus context.
#[derive(Clone)]
pub struct AgentSocket {
    pub status: Signal<WsStatus>,
    pub events: Signal<Vec<ServerEvent>>,
    tx: Coroutine<CommandPayload>,
}

impl AgentSocket {
    /// Queue a command for the writer task.
    pub fn send(&self, payload: CommandPayload) {
        self.tx.send(payload);
    }
}

/// Open (and keep open) the agent socket. Call once near the app root and share
/// the returned handle via context.
pub fn use_agent_socket() -> AgentSocket {
    let status = use_signal(|| WsStatus::Connecting);
    let events = use_signal(Vec::<ServerEvent>::new);

    let tx = use_coroutine(
        move |mut rx: UnboundedReceiver<CommandPayload>| async move {
            // Rebind status as mutable so the Copy signal handle accepts set();
            // `events` is mutated inside handle_text (passed by value) so stays
            // immutable here.
            let mut status = status;
            let events = events;

            let url = ws_url();
            let mut subscribed: HashMap<String, i64> = HashMap::new();
            let mut next_id: u64 = 0;
            let mut backoff = BACKOFF_BASE_MS;

            loop {
                status.set(WsStatus::Connecting);
                let ws = match WebSocket::open(&url) {
                    Ok(ws) => ws,
                    Err(_) => {
                        status.set(WsStatus::Reconnecting);
                        TimeoutFuture::new(backoff).await;
                        backoff = (backoff.saturating_mul(2)).min(BACKOFF_MAX_MS);
                        continue;
                    }
                };
                backoff = BACKOFF_BASE_MS;
                let (mut write, mut read) = ws.split();
                status.set(WsStatus::Open);

                // Replay-resume: re-subscribe to known sessions with from_event_id.
                for (sid, from) in subscribed.clone() {
                    next_id += 1;
                    send_cmd(
                        &mut write,
                        next_id,
                        CommandPayload::Subscribe {
                            session_id: sid,
                            from_event_id: Some(from),
                        },
                    )
                    .await;
                }

                // Pump until the socket closes, then fall through to reconnect.
                loop {
                    select! {
                        outbound = rx.next().fuse() => match outbound {
                            Some(payload) => {
                                if let CommandPayload::Subscribe { session_id, from_event_id } = &payload {
                                    subscribed.insert(session_id.clone(), from_event_id.unwrap_or(0));
                                }
                                next_id += 1;
                                if !send_cmd(&mut write, next_id, payload).await {
                                    break;
                                }
                            }
                            None => return, // coroutine dropped → stop entirely
                        },
                        inbound = read.next().fuse() => match inbound {
                            Some(Ok(Message::Text(text))) => {
                                handle_text(&text, &mut write, events, &mut subscribed).await;
                            }
                            Some(Ok(Message::Bytes(_))) => {}
                            _ => break, // close/error → reconnect
                        },
                    }
                }

                status.set(WsStatus::Reconnecting);
                TimeoutFuture::new(backoff).await;
                backoff = (backoff.saturating_mul(2)).min(BACKOFF_MAX_MS);
            }
        },
    );

    AgentSocket { status, events, tx }
}

type Sink = SplitSink<WebSocket, Message>;

/// Serialize a command into the envelope and write it. Returns false on write
/// failure (caller should reconnect).
async fn send_cmd(write: &mut Sink, id: u64, payload: CommandPayload) -> bool {
    let cmd = ClientCommand::new(format!("c{id}"), 0, payload);
    match serde_json::to_string(&cmd) {
        Ok(text) => write.send(Message::Text(text)).await.is_ok(),
        Err(_) => true, // skip a malformed command, keep the socket
    }
}

async fn handle_text(
    text: &str,
    write: &mut Sink,
    mut events: Signal<Vec<ServerEvent>>,
    subscribed: &mut HashMap<String, i64>,
) {
    let env = match serde_json::from_str::<ServerEnvelope>(text) {
        Ok(env) => env,
        Err(_) => return, // ignore malformed server messages
    };
    match env {
        ServerEnvelope::Event {
            id,
            session_id,
            ts,
            payload,
        } => {
            if let Ok(n) = id.parse::<i64>() {
                let cur = subscribed.get(&session_id).copied().unwrap_or(0);
                if n > cur {
                    subscribed.insert(session_id.clone(), n);
                }
            }
            let mut buf = events.write();
            buf.push(ServerEvent {
                id,
                session_id,
                ts,
                payload,
            });
            if buf.len() > EVENTS_CAP {
                let overflow = buf.len() - EVENTS_CAP;
                buf.drain(0..overflow);
            }
        }
        ServerEnvelope::Ping { .. } => {
            let _ = write
                .send(Message::Text(r#"{"type":"pong","ts":0}"#.to_string()))
                .await;
        }
        // ack / pong / error: no-op for now (see module note on ack handling).
        _ => {}
    }
}
