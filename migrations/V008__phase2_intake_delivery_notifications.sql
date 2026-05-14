-- Phase 2 / story 2.3 — Intake / Delivery / Notify event log.
-- refs: /specs/phase-2/architecture.md §2.8, §2.9, §3 V008
-- refs: /specs/phase-2/stories/story-2.3.md
--
-- UNIQUE (channel, intake_id) is the idempotency key for the
-- IntakeRouter (story 2.5): replays of the same external delivery
-- (webhook retry, IMAP re-fetch) collapse onto the existing row.
-- reply_target / target / payload are JSON TEXT mirrors of the
-- channel framework's DeliveryTarget / NotifyTarget / NotifyEvent
-- shapes from §2.7.

CREATE TABLE intake_events (
    id              TEXT    PRIMARY KEY,
    tenant_id       TEXT,
    channel         TEXT    NOT NULL,
    intake_id       TEXT    NOT NULL,
    brief_input     TEXT    NOT NULL,
    reply_target    TEXT,
    metadata        TEXT,
    task_id         TEXT REFERENCES tasks(id),
    received_at     INTEGER NOT NULL,
    UNIQUE (channel, intake_id)
);

CREATE TABLE delivery_events (
    id              TEXT    PRIMARY KEY,
    tenant_id       TEXT,
    task_id         TEXT    NOT NULL REFERENCES tasks(id),
    deliverable_id  TEXT    NOT NULL REFERENCES deliverables(id),
    channel         TEXT    NOT NULL,
    target          TEXT    NOT NULL,
    ok              INTEGER NOT NULL,
    external_id     TEXT,
    error           TEXT,
    delivered_at    INTEGER NOT NULL
);

CREATE TABLE notifications_sent (
    id              TEXT    PRIMARY KEY,
    tenant_id       TEXT,
    task_id         TEXT,
    trigger_kind    TEXT    NOT NULL,
    channel         TEXT    NOT NULL,
    target          TEXT,
    payload         TEXT,
    ok              INTEGER NOT NULL,
    error           TEXT,
    sent_at         INTEGER NOT NULL
);

CREATE INDEX idx_intake_task         ON intake_events(task_id);
CREATE INDEX idx_intake_channel      ON intake_events(channel, received_at);
CREATE INDEX idx_delivery_task       ON delivery_events(task_id);
CREATE INDEX idx_delivery_deliv      ON delivery_events(deliverable_id);
CREATE INDEX idx_notifs_task         ON notifications_sent(task_id);
