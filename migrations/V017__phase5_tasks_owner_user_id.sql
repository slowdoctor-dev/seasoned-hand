-- V017: Phase 5 story 5.9 — add `tasks.owner_user_id` for hand-off lifecycle.
-- refs: /specs/phase-5/stories/story-5.9.md
-- refs: /specs/phase-5/architecture.md §5
--
-- The Phase 2 `tasks` table predates the org/user model. Hand-off lifecycle
-- needs a `users.id` FK on every task so reassignment can move ownership
-- atomically. Backfill every existing row to the V013 sentinel
-- `user-legacy-admin` so the column can be queried immediately;
-- per-domain stories that own task creation (Phase 6+) can flip to NOT
-- NULL once they always resolve a non-sentinel owner.

ALTER TABLE tasks ADD COLUMN owner_user_id TEXT REFERENCES users(id);

-- Backfill: every existing task is owned by the legacy admin sentinel until
-- explicit hand-off reassigns them.
UPDATE tasks SET owner_user_id = 'user-legacy-admin' WHERE owner_user_id IS NULL;

CREATE INDEX IF NOT EXISTS idx_tasks_owner_user_id ON tasks(owner_user_id);
