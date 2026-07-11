# EarnScore persistence architecture

Task 01 defines the persistence boundary only. Feed processors, workers, APIs and
the dashboard remain outside this change; they consume the durable contracts
described here.

## Component and data flow

    Source Feed
        |
        v
    Feed session + append-only feed_events
        |
        +--> matches (latest snapshot)
        |       |
        |       +--> match_state_history
        |       +--> odds_current / odds_history
        |       +--> sync_outbox
        |
        +--> detail_jobs --> match_detail_sections + match_detail_data
        |                       |
        |                       +--> incidents / statistics / lineups / H2H
        |
        +--> asset_jobs --> assets --> asset_links
        |
        +--> recovery_jobs
        |
        v
    Local SQLite (WAL, foreign keys, 5 second busy timeout)
        |
        v
    D1/R2 sync boundary

Each feed event has an event key, payload hash, source timestamp and received
timestamp. The unique event key makes replay safe. State and odds histories are
append-only; current tables are conflict-safe projections. Detail and asset
jobs use a unique match/section or asset key so reconnects do not enqueue
completed work again.

## Persistence invariants

1. A migration never deletes a legacy table or row. An unscoped legacy table is
   renamed to a retained legacy archive, then copied into the v3 table.
2. Match IDs are copied as-is and are listed in the migration audit report.
3. Empty and invalid detail JSON is represented explicitly as EMPTY or
   UNPARSEABLE. Invalid content is not coerced into structured values.
4. Legacy odds are converted only when bookmaker, market, selection and a
   numeric odds value are present. The source provenance is retained.
5. New asset records contain local storage keys and content hashes. The v3
   asset contract has no source image URL, cookie, token or authentication
   header field. Existing logo columns remain only for generation-2 sync
   compatibility and are not part of the new asset relationship.
6. A verified backup is a separate file with matching size/checksum and a
   successful SQLite integrity check before it is used for restore.

## Versioning

Local SQLite records user_version 3 and one row in migration_audit with the
deterministic conversion report. D1 starts from the migration-compatible
schema.sql, then d1:migrate applies historical 0001 and 0002 before
0003_event_driven_live.sql completes the same event/detail/job/asset/outbox
contract. Migrations 0001 and 0002 are historical inputs and remain unchanged.
