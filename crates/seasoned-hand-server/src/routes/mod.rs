//! Per-domain HTTP route modules (issue #43 — god-file decomposition).
//! Pure code moves out of `lib.rs`; `app()` in `lib.rs` stays the wiring spine.

pub(crate) mod admin;
pub(crate) mod auth_routes;
pub(crate) mod channels;
pub(crate) mod events;
pub(crate) mod intake;
pub(crate) mod org;
pub(crate) mod projects;
pub(crate) mod sessions;
pub(crate) mod verifications;
