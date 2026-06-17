# Migrations

SQLite schema migrations, applied by [Refinery](https://crates.io/crates/refinery)
at startup (`db::run_migrations`). Files are `V<NNN>__<name>.sql`, applied once in
version order and tracked in Refinery's `refinery_schema_history` table.

## Conventions / gotchas

- **Append-only — never edit an applied migration.** Refinery records a checksum of
  each migration's content; editing a file that has already been applied to any
  existing database fails checksum validation at startup. Fix-ups go in a *new*
  `V<next>` migration. (This is why, e.g., the historical V013–V020 table-rebuild
  migrations were *not* retrofitted with `PRAGMA foreign_key_check` — issue #23;
  newer rebuilds like V025/V026 include it.)

- **Backfill `UPDATE`/`INSERT` migrations are NOT replay-safe** (issue #23). The
  data-backfill steps in **V011** (Phase 4 curator) and **V013** (Phase 5 tenant
  surfaces), among others, assume they run exactly once. That assumption is
  satisfied by Refinery's once-only application semantics, so it is safe in
  practice — but the SQL itself is not idempotent and must not be re-run manually
  against an already-migrated database.

- **Table-rebuild migrations** (rename → recreate → `INSERT … SELECT` → drop old)
  run under `PRAGMA foreign_keys = OFF`. New ones should re-enable foreign keys and
  run `PRAGMA foreign_key_check;` before finishing (see V025/V026).
