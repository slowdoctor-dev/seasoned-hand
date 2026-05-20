//! Authorization core for Phase 5 multi-user controls.
//!
//! refs: /specs/phase-5/architecture.md §4
//! refs: /specs/phase-5/stories/story-5.3.md

pub mod context;
pub mod policy;
pub mod system;

pub use context::{Action, AuthContext, Role};
pub use policy::{AuthError, AuthResource, authorize};
pub use system::SystemAuth;
