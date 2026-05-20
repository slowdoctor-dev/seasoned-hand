-- Phase 5 / story 5.19
-- - User invitation token persistence.
-- - Deferred tenant_id NOT NULL flips for phase-2 channel tables.
--
-- V013 backfilled tenant_id values, so channel-table flips are schema-only.

PRAGMA foreign_keys = OFF;

CREATE TABLE IF NOT EXISTS user_invitation_tokens (
    token_hash  TEXT PRIMARY KEY,
    user_id     TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at  INTEGER NOT NULL,
    consumed_at INTEGER
);
CREATE INDEX IF NOT EXISTS idx_user_invitation_tokens_user_created
    ON user_invitation_tokens(user_id, created_at);

CREATE TABLE intake_events_v020_new (
    id           TEXT PRIMARY KEY,
    tenant_id    TEXT NOT NULL DEFAULT 'legacy-default',
    channel      TEXT NOT NULL,
    intake_id    TEXT NOT NULL,
    brief_input  TEXT NOT NULL,
    reply_target TEXT,
    metadata     TEXT,
    task_id      TEXT REFERENCES tasks(id),
    received_at  INTEGER NOT NULL,
    UNIQUE (channel, intake_id)
);
INSERT INTO intake_events_v020_new (
    id, tenant_id, channel, intake_id, brief_input, reply_target, metadata, task_id, received_at
)
SELECT
    id, tenant_id, channel, intake_id, brief_input, reply_target, metadata, task_id, received_at
FROM intake_events;
DROP TABLE intake_events;
ALTER TABLE intake_events_v020_new RENAME TO intake_events;
CREATE INDEX idx_intake_task ON intake_events(task_id);
CREATE INDEX idx_intake_channel ON intake_events(channel, received_at);

CREATE TABLE delivery_events_v020_new (
    id             TEXT PRIMARY KEY,
    tenant_id      TEXT NOT NULL DEFAULT 'legacy-default',
    task_id        TEXT NOT NULL REFERENCES tasks(id),
    deliverable_id TEXT NOT NULL REFERENCES deliverables(id),
    channel        TEXT NOT NULL,
    target         TEXT NOT NULL,
    ok             INTEGER NOT NULL,
    external_id    TEXT,
    error          TEXT,
    delivered_at   INTEGER NOT NULL
);
INSERT INTO delivery_events_v020_new (
    id, tenant_id, task_id, deliverable_id, channel, target, ok, external_id, error, delivered_at
)
SELECT
    id, tenant_id, task_id, deliverable_id, channel, target, ok, external_id, error, delivered_at
FROM delivery_events;
DROP TABLE delivery_events;
ALTER TABLE delivery_events_v020_new RENAME TO delivery_events;
CREATE INDEX idx_delivery_task ON delivery_events(task_id);
CREATE INDEX idx_delivery_deliv ON delivery_events(deliverable_id);

CREATE TABLE notifications_sent_v020_new (
    id           TEXT PRIMARY KEY,
    tenant_id    TEXT NOT NULL DEFAULT 'legacy-default',
    task_id      TEXT,
    trigger_kind TEXT NOT NULL,
    channel      TEXT NOT NULL,
    target       TEXT,
    payload      TEXT,
    ok           INTEGER NOT NULL,
    error        TEXT,
    sent_at      INTEGER NOT NULL
);
INSERT INTO notifications_sent_v020_new (
    id, tenant_id, task_id, trigger_kind, channel, target, payload, ok, error, sent_at
)
SELECT
    id, tenant_id, task_id, trigger_kind, channel, target, payload, ok, error, sent_at
FROM notifications_sent;
DROP TABLE notifications_sent;
ALTER TABLE notifications_sent_v020_new RENAME TO notifications_sent;
CREATE INDEX idx_notifs_task ON notifications_sent(task_id);

PRAGMA foreign_keys = ON;
