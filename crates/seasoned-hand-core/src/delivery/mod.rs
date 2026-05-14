//! Delivery event persistence (V008 `delivery_events` table).
//!
//! Owns the [`DeliveryEventStore`] that records each
//! `DeliverySink::deliver` outcome — successful sends and failures
//! both. Story 2.5 wires this into the DeliveryRouter; story 2.15
//! reads from it when composing the provenance manifest's
//! `delivered_to[]` array.
//!
//! refs: /specs/phase-2/architecture.md §2.9, §3 V008
//! refs: /specs/phase-2/stories/story-2.3.md

pub mod router;
pub mod store;

pub use router::{DEFAULT_RETRY_DELAY, DeliveryRouter, DeliveryRouterError};
pub use store::{DeliveryEventRow, DeliveryEventStore, DeliveryStoreError, NewDeliveryEvent};

#[cfg(test)]
mod tests;
