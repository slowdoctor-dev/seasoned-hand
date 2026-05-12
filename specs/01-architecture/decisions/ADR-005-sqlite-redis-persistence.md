# ADR-005: SQLite WAL + Redis for persistence

Status: Accepted
Date: 2026

## Context

Two distinct storage needs:

1. **Durable event stream** — append-only, FTS-searchable, must survive restart
2. **Real-time pub/sub** — WebSocket fanout, ephemeral queues, low latency

These have different access patterns. Single-database solutions exist but
make trade-offs.

## Decision

Use **SQLite WAL mode** for durable storage + **Redis** for pub/sub and
ephemeral queues.

SQLite holds: events, sessions, playbooks, glossary, SOPs (via FTS5 for
search).
Redis holds: pub/sub channels (event stream subscribers), task queues
(Tokio + Redis Streams).

Self-hosting friendly: both run in single Docker containers, both have tiny
footprints, both are battle-tested.

## Consequences

**Positive:**
- SQLite needs zero configuration
- FTS5 gives free full-text search for playbook/session retrieval
- Redis pub/sub is fast (microsecond latency)
- Both fit on a $5 VPS
- Single-machine deployment is the common case for self-hosting

**Negative:**
- Two storage systems = two operational concerns (backup, version, etc.)
- No automatic replication (single-machine assumption)
- Limited concurrent writers (SQLite WAL handles dozens, not thousands)

**Neutral:**
- Multi-machine deployment (Phase 5+) requires storage upgrade path
  (Postgres + Redis cluster) — but not yet

## Alternatives considered

### Alternative A: PostgreSQL only
Industry standard. But:
- Heavier ops (replication, vacuum, tuning)
- Overkill for single-user / small-team self-hosting
- Worse default story for FTS (need pg_trgm or pg_search)

Rejected for v1. Will reconsider when scale demands it.

### Alternative B: Redis + RedisJSON only (no SQLite)
Single store. But:
- Durability story weaker (AOF/RDB vs WAL)
- FTS via RediSearch is good but requires extra module
- Loses SQL queryability for analytics

Rejected on durability concerns.

### Alternative C: SQLite only (no Redis)
Simplest. But:
- SQLite pub/sub is awkward (polling or LISTEN/NOTIFY tricks)
- Redis is small, well-understood, and shines at pub/sub

Rejected on real-time UX.

## References

- SQLite WAL: https://sqlite.org/wal.html
- Redis Streams: https://redis.io/docs/data-types/streams/
- Event stream design: `/specs/01-architecture/ARCHITECTURE.md` § 2.1
