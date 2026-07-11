# Migration, backup and rollback procedure

## Local SQLite

1. Stop crawler writers and confirm the database path is the intended local
   file. Never run this procedure against a production or user database.
2. Create a separate verified backup:

       cargo run -- --db-path local.db backup --destination backups/local-before-v3.db

   The command rejects the source path as a destination, checkpoints WAL,
   compares byte size and checksum, and runs SQLite integrity_check.
3. Apply the migration, optionally creating the backup in the same command:

       cargo run -- --db-path local.db migrate --backup-destination backups/local-before-v3.db

   The migration sets WAL, foreign_keys and a 5 second busy timeout, archives
   unscoped legacy tables instead of dropping them, preserves match IDs, and
   records migration_audit.report_json.
4. Verify the logical result:

       cargo test schema_
       cargo test migration_
       cargo test backup_
       cargo test legacy_conversion_
       cargo test

   Check migration_audit, match_detail_sections, odds_history and the retained
   legacy archive tables before restarting writers.

Restore is also atomic from the application point of view. It copies the
backup to a temporary destination, verifies integrity, and replaces only the
requested destination:

    cargo run -- --db-path local.db restore \
      --backup backups/local-before-v3.db \
      --destination local-restored.db

## D1 fresh and legacy verification

Use different Wrangler persistence directories for fresh and legacy paths so
their counts cannot contaminate one another:

    cd dashboard
    npm run d1:init -- --persist-to .wrangler/state/fresh
    npm run d1:migrate -- --persist-to .wrangler/state/fresh
    npm run d1:verify -- --persist-to .wrangler/state/fresh

schema.sql is deliberately migration-compatible: the legacy columns added by
0001 are absent from its base-table definitions so the historical ALTER TABLE
statements can run once. The final fresh shape is the database after the three
steps above; do not treat d1:init alone as the final schema.

For the legacy path, load dashboard/test/legacy-schema.sql into a separate
directory, apply the historical 0001 and 0002 migrations, then apply 0003:

    npx wrangler d1 execute earnscore-db --local \
      --persist-to .wrangler/state/legacy \
      --file=test/legacy-schema.sql
    npx wrangler d1 migrations apply earnscore-db --local \
      --persist-to .wrangler/state/legacy

The fixture comments list the assertions: both legacy match IDs must remain,
both match_detail rows must remain, the invalid stats section must be
UNPARSEABLE, and the parseable odds record must have legacy provenance. Compare
the fresh and legacy table/count outputs before enabling sync.

For a remote D1 backup, use the Wrangler D1 export command into a separate
artifact and keep the export with its command output and checksum:

    npx wrangler d1 export earnscore-db --remote --output backups/earnscore-d1.sql

Do not edit 0001 or 0002 to roll back. Migration 0003 is additive. To roll
back application behavior, stop writers, deploy the prior application version,
and restore the SQLite backup or import the D1 export into a new database.
Switch the binding only after comparing match IDs and counts. A rollback does
not delete v3 rows; it changes the active database/binding to a verified
pre-migration copy.

## Failure handling

If checksum, file-size, integrity_check or foreign_key_check fails, treat the
backup as unusable and stop. Do not overwrite the destination with a failed
temporary copy. Inspect migration_audit.error_text and the deterministic
unmappable_fields report for data that needs a manual follow-up.
