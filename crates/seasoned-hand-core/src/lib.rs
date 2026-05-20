//! Seasoned Hand core library.
//! refs: /specs/phase-0/architecture.md §2

pub mod agent;
pub mod auth;
pub mod browser;
pub mod capability;
pub mod channel;
pub mod checkpoint;
pub mod cost;
pub mod curator;
pub mod db;
pub mod deliverable;
pub mod delivery;
pub mod dispatch;
pub mod events;
pub mod intake;
pub mod llm;
pub mod matcher;
pub mod notify;
pub mod org;
pub mod plan;
pub mod project;
pub mod provenance;
pub mod pubsub;
pub mod router;
pub mod routes;
pub mod sandbox;
pub mod search;
pub mod sharing;
pub mod task;
pub mod time;
pub mod tools;
pub mod verifier;

/// Returns the core crate version.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::version;

    #[test]
    fn version_matches_package_version() {
        assert_eq!(version(), "0.1.0");
    }
}
