//! WebSocket client for the agent event stream. Rust port of
//! `frontend/lib/ws.ts`: connect, subscribe, push events into a signal, reply
//! to server pings, and reconnect with exponential backoff + subscription
//! replay.
//!
//! Acks are correlated by `ref` to capture the `session_id` the server assigns
//! to a `task_create`, which is written into the shared `session_id` signal so
//! the rest of the UI subscribes to the new run. A general per-command
//! ack-await Future API (resolving an arbitrary command by `ref`) remains a
//! follow-up; only the task_create→session capture is wired here.

use crate::auth;
use crate::config::{ws_url, WS_AUTH_SUBPROTOCOL};
use dioxus::prelude::*;
use futures::channel::mpsc::UnboundedReceiver;
use futures::stream::SplitSink;
use futures::{select, FutureExt, SinkExt, StreamExt};
use gloo_net::websocket::{futures::WebSocket, Message};
use gloo_timers::future::TimeoutFuture;
use seasoned_hand_dto::{ClientCommand, CommandPayload, ServerEnvelope, ServerEvent};
use std::collections::{HashMap, HashSet};

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
/// the returned handle via context. `session_id` is the shared selection signal
/// the coroutine writes when a `task_create` ack assigns a new session.
pub fn use_agent_socket(session_id: Signal<Option<String>>) -> AgentSocket {
    let status = use_signal(|| WsStatus::Connecting);
    let events = use_signal(Vec::<ServerEvent>::new);

    let tx = use_coroutine(
        move |mut rx: UnboundedReceiver<CommandPayload>| async move {
            // Rebind status as mutable so the Copy signal handle accepts set();
            // `events` / `session_id` are mutated inside handle_text (passed by
            // value) so stay immutable here.
            let mut status = status;
            let events = events;
            let session_id = session_id;

            let url = ws_url();
            let mut subscribed: HashMap<String, i64> = HashMap::new();
            // Command ids of in-flight task_create commands, so their ack can be
            // recognised and its assigned session_id captured.
            let mut pending_task_creates: HashSet<String> = HashSet::new();
            let mut next_id: u64 = 0;
            let mut backoff = BACKOFF_BASE_MS;

            loop {
                status.set(WsStatus::Connecting);
                // Issue #26 / ADR-018: re-read the verified session token each
                // (re)connect so a fresh login is picked up. Without a token (not
                // yet logged in) wait and retry rather than opening an unauthable
                // socket that the server would reject at upgrade.
                let token = match auth::current_token() {
                    Some(token) => token,
                    None => {
                        status.set(WsStatus::Reconnecting);
                        TimeoutFuture::new(backoff).await;
                        backoff = (backoff.saturating_mul(2)).min(BACKOFF_MAX_MS);
                        continue;
                    }
                };
                let ws = match WebSocket::open_with_protocols(
                    &url,
                    &[WS_AUTH_SUBPROTOCOL, token.as_str()],
                ) {
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
                        &format!("c{next_id}"),
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
                                if let CommandPayload::Subscribe { session_id: sid, from_event_id } = &payload {
                                    // Issue #22: keep the HIGHEST watermark per
                                    // session. An app-initiated `Subscribe{from:0}`
                                    // (initial load) must not lower a watermark already
                                    // advanced by received events, or a later reconnect
                                    // would replay the whole history from 0 instead of
                                    // resuming from the last seen id.
                                    let from = from_event_id.unwrap_or(0);
                                    let cur = subscribed.get(sid).copied().unwrap_or(0);
                                    subscribed.insert(sid.clone(), cur.max(from));
                                }
                                next_id += 1;
                                let cmd_id = format!("c{next_id}");
                                if matches!(payload, CommandPayload::TaskCreate { .. }) {
                                    pending_task_creates.insert(cmd_id.clone());
                                }
                                if !send_cmd(&mut write, &cmd_id, payload).await {
                                    break;
                                }
                            }
                            None => return, // coroutine dropped → stop entirely
                        },
                        inbound = read.next().fuse() => match inbound {
                            Some(Ok(Message::Text(text))) => {
                                handle_text(
                                    &text,
                                    &mut write,
                                    events,
                                    session_id,
                                    &mut subscribed,
                                    &mut pending_task_creates,
                                )
                                .await;
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
async fn send_cmd(write: &mut Sink, id: &str, payload: CommandPayload) -> bool {
    let cmd = ClientCommand::new(id.to_string(), 0, payload);
    match serde_json::to_string(&cmd) {
        Ok(text) => write.send(Message::Text(text)).await.is_ok(),
        Err(e) => {
            // Issue #23: a serialization failure shouldn't drop the socket (return
            // true = "keep going"), but it was silent — surface it.
            log::error!("ws: failed to serialize command {id}: {e}");
            true
        }
    }
}

async fn handle_text(
    text: &str,
    write: &mut Sink,
    mut events: Signal<Vec<ServerEvent>>,
    mut session_id: Signal<Option<String>>,
    subscribed: &mut HashMap<String, i64>,
    pending_task_creates: &mut HashSet<String>,
) {
    let env = match serde_json::from_str::<ServerEnvelope>(text) {
        Ok(env) => env,
        Err(_) => return, // ignore malformed server messages
    };
    match env {
        ServerEnvelope::Event {
            id,
            session_id: ev_session,
            ts,
            payload,
        } => {
            if let Ok(n) = id.parse::<i64>() {
                let cur = subscribed.get(&ev_session).copied().unwrap_or(0);
                if n > cur {
                    subscribed.insert(ev_session.clone(), n);
                }
            }
            let mut buf = events.write();
            buf.push(ServerEvent {
                id,
                session_id: ev_session,
                ts,
                payload,
            });
            if buf.len() > EVENTS_CAP {
                let overflow = buf.len() - EVENTS_CAP;
                buf.drain(0..overflow);
            }
        }
        ServerEnvelope::Ack {
            reference,
            ok,
            session_id: ack_session,
            ..
        } => {
            // A successful task_create ack carries the freshly-assigned
            // session_id; capture it so the rest of the UI subscribes.
            if ok && pending_task_creates.remove(&reference) {
                if let Some(sid) = ack_session {
                    session_id.set(Some(sid));
                }
            }
        }
        ServerEnvelope::Ping { .. } => {
            let _ = write
                .send(Message::Text(r#"{"type":"pong","ts":0}"#.to_string()))
                .await;
        }
        // pong / error: no-op (server errors are not yet surfaced in the UI).
        _ => {}
    }
}
