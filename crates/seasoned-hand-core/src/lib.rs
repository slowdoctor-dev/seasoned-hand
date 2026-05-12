//! Seasoned Hand core library.
//! refs: /specs/phase-0/architecture.md §2

pub mod agent;
pub mod capability;
pub mod cost;
pub mod db;
pub mod dispatch;
pub mod events;
pub mod llm;
pub mod pubsub;
pub mod router;
pub mod sandbox;
pub mod search;
pub mod tools;

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
