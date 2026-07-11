# Task 01: Event-Driven Persistence and Non-Destructive Migration

## Status
Passed

## Outcome

SQLite and D1 have a repeat-safe event-driven v3 schema, verified backup/restore workflow and deterministic legacy conversion that preserves public Match IDs without deleting legacy rows or rewriting historical D1 migrations.

## Dependencies

None.

## Affected Files

### Required Files

- src/main.rs — storage module wiring and migration/backup CLI integration only
- src/storage/mod.rs — new storage public interface
- src/storage/schema.rs — new SQLite v3 schema and indexes
- src/storage/migration.rs — new repeat-safe local migration and legacy conversion
- src/storage/backup.rs — new verified backup/restore helpers
- dashboard/schema.sql — fresh D1 v3 bootstrap
- dashboard/migrations/0003_event_driven_live.sql — new additive D1 migration
- dashboard/test/legacy-schema.sql — legacy seed rows and migration assertions
- dashboard/package.json — fresh/migrate/verify scripts only
- docs/architecture.md — approved component/data-flow diagram
- docs/database-er.md — canonical/event/detail/job/asset/outbox ER diagram
- docs/migration-and-rollback.md — backup, migration, verification and rollback procedure

### Allowed Files

- Cargo.toml — only if backup checksum or migration tests require a dependency unavailable in the current crate
- Cargo.lock — only as the mechanical result of an approved Cargo.toml change

## Forbidden Files

- dashboard/migrations/0001_full_payload.sql
- dashboard/migrations/0002_dataset_generation.sql
- dashboard/src/index.js
- tests/fixtures/source-filter-states.json
- tests/fixtures/chrome-targets.json
- all browser/feed/detail/asset/recovery implementation files
- any production or user SQLite/D1 database
- all agent framework/runtime/task-state files

## Scope

- Add v3 tables/columns/indexes for sports, competitions, teams, players, coaches, venues, referees, Matches, state history, current/history odds, detail sections/data, incidents, statistics, lineups, non-canonical H2H references, assets/links, feed sessions/events, detail/asset/recovery jobs, sync outbox, settings and migration audit.
- Preserve existing Match IDs and dataset metadata.
- Convert legacy Match detail into section rows with provenance and explicit empty/unparseable reporting.
- Convert parseable legacy odds without inventing values.
- Configure SQLite WAL, foreign keys and busy timeout.
- Create a verified SQLite backup before migration and document D1 export/rollback steps.
- Keep all new source image URL fields out of the v3 schema.
- Do not implement processors, workers, APIs or UI.

## Implementation Steps

1. Extract SQLite initialization/migration responsibilities from src/main.rs into the new storage modules without moving unrelated crawler logic.
2. Define the final v3 logical contract and indexes. Use additive ALTER/CREATE operations for local SQLite; do not drop legacy tables or rows.
3. Add migration version/audit records and a deterministic legacy conversion report containing counts, preserved IDs and unmappable fields.
4. Implement backup creation with destination separation, checksum/size verification and restore verification on a temporary copy.
5. Add dashboard migration 0003. Historical migrations 0001 and 0002 remain byte-for-byte unchanged.
6. Align dashboard/schema.sql with the final post-0003 fresh schema.
7. Extend dashboard/test/legacy-schema.sql so a legacy path proves Match ID and row preservation.
8. Add focused tests for fresh init, repeat migration, concurrent connections, backup verification, restore verification, section conversion and legacy odds provenance.
9. Document architecture, ER model and safe migration/rollback commands.

## Acceptance Criteria

- Fresh SQLite and legacy SQLite converge on the same v3 logical schema.
- Running local migration twice produces no duplicate tables, sections, odds history or audit rows.
- Every mappable legacy Match retains its original Match ID.
- Existing rows remain queryable; unmappable detail/odds fields appear in a report and are not fabricated.
- SQLite uses WAL, foreign keys and a bounded busy timeout on every opened connection.
- Backup verification fails closed on a truncated/corrupt copy.
- Fresh D1 bootstrap and legacy D1 plus migrations 0001, 0002 and 0003 converge logically.
- Migration 0003 does not drop tables or delete rows.
- No new v3 column stores a source image URL, cookie, token or auth header.
- Architecture, ER and rollback documents match the implemented schema.

## Validation Commands

    cargo fmt -- --check
    cargo test schema_
    cargo test migration_
    cargo test backup_
    cargo test legacy_conversion_
    cargo test

    cd dashboard && npm run d1:init
    cd dashboard && npm run d1:migrate
    cd dashboard && npm run d1:verify

Reviewer must run fresh and legacy D1 commands against separate temporary persist directories and compare Match IDs/counts before and after migration.

## Reference Map

- Cargo.toml — current Rust dependency baseline
- src/main.rs — open_db, init_db, run_migrations and existing schema tests
- dashboard/schema.sql — current fresh D1 bootstrap
- dashboard/migrations/0001_full_payload.sql — read-only migration predecessor
- dashboard/migrations/0002_dataset_generation.sql — read-only migration predecessor
- dashboard/test/legacy-schema.sql — current legacy D1 fixture
- dashboard/package.json — current Wrangler scripts
