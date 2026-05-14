//! Intake event persistence (V008 `intake_events` table).
//!
//! Owns the [`IntakeEventStore`] that round-trips
//! [`crate::channel::IntakeEvent`] payloads through SQLite. The shared
//! intake types (`IntakeEvent`, `DeliveryTarget`, `IntakeProvider`)
//! continue to live in the `channel` module — story 2.4 landed them
//! there and the routing layer (story 2.5) references them by their
//! channel-side path.
//!
//! refs: /specs/phase-2/architecture.md §2.8, §3 V008
//! refs: /specs/phase-2/stories/story-2.3.md

pub mod router;
pub mod spawner;
pub mod store;

pub use router::{HandleOutcome, IntakeRouter, IntakeRouterError, RejectionReason};
pub use spawner::{InitializerSpawner, SpawnError, SpawnReceipt, SpawnSpec};
pub use store::{IntakeEventRow, IntakeEventStore, IntakeStoreError};

#[cfg(test)]
mod tests;
