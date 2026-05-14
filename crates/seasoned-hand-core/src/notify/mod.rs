//! Notification log persistence (V008 `notifications_sent` table).
//!
//! Distinct from [`crate::channel::notify`]: that module owns the
//! `NotifySink` trait surface (channel routing), while this module owns
//! the audit log of every notify dispatch the NotifyWorker emits.
//! Story 2.5 wires this into the NotifyWorker dispatch path.
//!
//! refs: /specs/phase-2/architecture.md §2.7, §3 V008
//! refs: /specs/phase-2/stories/story-2.3.md

pub mod config;
pub mod listener;
pub mod store;
pub mod worker;

pub use config::{KNOWN_TRIGGERS, NotifyConfig, NotifyConfigError};
pub use listener::{NotifyDispatch, NotifyEventListener, RedisNotifyDispatch};
pub use store::{
    NewNotificationSent, NotificationSentRow, NotificationsSentStore, NotifyStoreError,
};
pub use worker::{
    NOTIFY_CONSUMER_GROUP, NOTIFY_STREAM, NotifyRequest, NotifyRuntimeConfig, NotifyWorker,
    StaticTargetResolver, TargetResolver,
};

#[cfg(test)]
mod tests;
