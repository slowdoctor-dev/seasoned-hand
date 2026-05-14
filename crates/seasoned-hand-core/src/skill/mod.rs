//! Skill / playbook persistence — reservation only in Phase 2.
//!
//! Both stores are empty types that hold a [`crate::db::DbPool`] handle;
//! Phase 2 logic never writes to the V009 `skills` / `playbooks` tables
//! (see Phase 2 DEBT #6). The Arcs threaded through `AppState` reserve
//! the slots so Phase 3 (Curator + post-task playbook extraction) lands
//! as pure logic, not schema + wiring.
//!
//! refs: /specs/phase-2/architecture.md §2.12, §3 V009
//! refs: /specs/phase-2/stories/story-2.3.md

use crate::db::DbPool;

#[derive(Clone)]
pub struct SkillStore {
    #[allow(dead_code)]
    pool: DbPool,
}

impl SkillStore {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    #[cfg(test)]
    pub(crate) fn pool_for_test(&self) -> &DbPool {
        &self.pool
    }
}

#[derive(Clone)]
pub struct PlaybookStore {
    #[allow(dead_code)]
    pool: DbPool,
}

impl PlaybookStore {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    #[cfg(test)]
    pub(crate) fn pool_for_test(&self) -> &DbPool {
        &self.pool
    }
}

#[cfg(test)]
mod tests;
