# Task 05: Ephemeral Asset Pipeline and Local Storage

## Status
Pending

## Outcome

All collected images are validated, content-addressed, deduplicated and stored locally with R2-ready metadata while source URLs remain only in active memory and legacy remote image fields are removed from the active database after safe conversion.

## Dependencies

- Task 01 complete and reviewed.
- Task 04 complete and reviewed.

## Affected Files

### Required Files

- src/main.rs — asset worker composition/configuration only
- src/assets/mod.rs — new asset queue/worker exports
- src/assets/download.rs — new bounded HTTP download and byte validation
- src/assets/store.rs — new SHA-256 dedup and atomic local file publication
- src/assets/types.rs — new non-serializable source candidate and persisted asset metadata
- src/detail/types.rs — handoff contract to the asset worker only
- src/detail/jobs.rs — section completion barrier integration only
- src/storage/repositories.rs — asset/link/job/outbox methods and legacy asset conversion queries
- docs/assets-and-storage.md — storage layout, legacy conversion and troubleshooting

### Allowed Files

- Cargo.toml — image-byte validation or SHA-256 dependencies only
- Cargo.lock — only as the mechanical result of an approved dependency change

## Forbidden Files

- dashboard/src/index.js
- dashboard/schema.sql
- dashboard/migrations/**
- src/feed/**
- src/recovery/**
- src/sync/**
- committed files under data/assets or temporary download directories
- persisted source image URLs in any new row/job/log/outbox
- all agent framework/runtime/task-state files

## Scope

- Accept source URLs only through the in-memory candidate type from Task 04.
- Download with configured concurrency, delay, timeout and retry.
- Validate HTTP status, Content-Type and actual image bytes.
- Calculate SHA-256 and deduplicate identical content.
- Publish files atomically under data/assets by asset type, owner and content hash.
- Store only asset ID/type/owner/local path/storage key/hash/MIME/size/dimensions/timestamps.
- Create idempotent R2 upload outbox intent without the source URL.
- Reacquire source URLs from a detail page after restart rather than persisting them.
- Convert legacy logo/image fields in memory; clear/replace active remote URLs only after verified asset storage, with the original retained in the Task 01 verified backup.
- Do not implement R2 network delivery or Dashboard rendering.

## Implementation Steps

1. Define an in-memory-only source candidate that cannot derive Serialize or enter debug logs.
2. Implement bounded download, retry and validation with temporary files under the configured asset root.
3. Hash validated bytes, reuse existing content and atomically rename into the final path.
4. Write asset, owner link, job completion and upload outbox intent transactionally.
5. Make section completion wait for required asset candidates to reach stored or explicit unavailable status without retaining URLs.
6. On restart, rebuild source candidates by reloading the relevant detail section.
7. Add a one-time legacy conversion that reads old logo fields into memory, stores verified assets, updates active owner references and produces an owner-ID-only failure report.
8. Recursively scan active DB JSON/job/outbox/log fixtures for source-domain URLs.
9. Document path permissions, disk cleanup, dedup behavior and recovery from partial files.

## Acceptance Criteria

- Valid images are stored under deterministic content-addressed paths and metadata matches the bytes.
- A duplicate hash reuses one physical file and creates only required owner links.
- Non-2xx, wrong Content-Type, malformed bytes and size-limit violations are rejected.
- Temporary files never become visible as final assets and are cleaned after failure/restart.
- Asset concurrency and delay settings are enforced.
- Persisted jobs, outbox rows, logs and diagnostics contain no source URL.
- Restart reacquires a URL from detail state and does not depend on a stored URL.
- Legacy active image fields contain internal asset references or explicit unavailable state after conversion; original values remain only in the verified backup.
- Required section completion does not occur before its required assets are stored/unavailable according to policy.
- An upload outbox replay does not create duplicate asset metadata.

## Validation Commands

    cargo fmt -- --check
    cargo test asset_download_
    cargo test asset_validation_
    cargo test asset_dedup_
    cargo test asset_atomic_
    cargo test asset_restart_
    cargo test legacy_asset_
    cargo test source_url_
    cargo test

Reviewer must inspect a temporary asset root and query the temporary active DB for source-domain URLs. Production asset directories and databases are forbidden.

## Reference Map

- src/detail/types.rs — Task 04 in-memory asset candidate boundary
- src/detail/jobs.rs — Task 04 section lifecycle
- src/storage/schema.rs — Task 01 asset/job/outbox tables
- src/storage/repositories.rs — Task 02/04 transaction patterns
- src/main.rs — current competition/team logo fields and sanitizer behavior
- dashboard/schema.sql — final metadata contract, read-only in this task
