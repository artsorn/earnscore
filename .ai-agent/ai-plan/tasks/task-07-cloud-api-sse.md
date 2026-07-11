# Task 07: Cloud Sync, Versioned REST, Legacy Aliases, SSE, and Admin Security

## Status

Pending

## Outcome

SQLite outbox changes reach D1/R2 idempotently, required data is exposed through versioned and compatibility REST routes, Dashboard clients receive durable SSE updates with resync, and Admin operations are authorized and audited.

## Dependencies

- Task 01 complete and reviewed.
- Task 02 complete and reviewed.
- Task 05 complete and reviewed.
- Task 06 complete and reviewed.

## Affected Files

### Required Files

- src/main.rs — sync worker composition/configuration only
- src/sync/mod.rs — new outbox claim, batching, delivery and acknowledgement exports
- src/sync/d1.rs — new versioned D1 request/response and retry logic
- src/sync/r2.rs — new asset upload delivery and acknowledgement logic
- src/storage/repositories.rs — outbox lease/ack/dead-letter/reconciliation methods
- dashboard/src/index.js — D1/R2 ingestion, versioned/required/legacy routes, SSE, auth, audit and metrics only
- dashboard/wrangler.toml — D1/R2 bindings and non-secret configuration only
- dashboard/package.json — API integration scripts
- dashboard/package-lock.json — dependency lock updates
- dashboard/test/api.integration.test.js — new local Worker/D1 contract tests
- tests/fixtures/sync-batch.json — v3 two-sport event/data/outbox fixture
- tests/fixtures/sync-batch-next-generation.json — generation rollover fixture
- docs/api-sync-and-security.md — route matrix, auth, SSE/resync and operational behavior

### Allowed Files

- Cargo.toml — protocol/testing dependency only when current crates are insufficient
- Cargo.lock — only as the mechanical result of an approved dependency change

## Forbidden Files

- dashboard/schema.sql
- dashboard/migrations/**
- Dashboard HTML/CSS/client rendering portion of dashboard/src/index.js
- src/feed/**
- src/detail/**
- src/recovery/** except public command interfaces
- hard-coded or committed production secrets
- legacy route removal
- all agent framework/runtime/task-state files

## Scope

- Claim and deliver SQLite outbox rows with bounded batches, leases, retry/jitter and acknowledgements.
- Upsert D1 projections and R2 assets idempotently.
- Reconcile partial D1/R2 failures and preserve dirty work until acknowledged.
- Expose canonical /api/v1 routes.
- Expose the requirement routes under /api/live, /api/matches, /api/assets and /api/feed.
- Preserve existing /api/matches/live and /api/matches/detail as temporary aliases.
- Implement SSE live events with durable IDs, heartbeat, Last-Event-ID, gap detection and REST resync.
- Protect all Admin routes and sensitive settings writes; audit accepted mutations.
- Expose required feed status and metrics.
- Remove the committed API_TOKEN fallback and require secrets/environment configuration.
- Do not implement Dashboard visual changes.

## Implementation Steps

1. Extract current sync_worker and DTOs into src/sync with generation-safe leases and content-version acknowledgements.
2. Deliver database projections and asset uploads independently so one partial failure does not falsely acknowledge the other.
3. Validate versioned ingestion envelopes and reject mixed generation or malformed rows before D1 mutation.
4. Build shared route handlers/projections used by canonical versioned routes, requirement aliases and legacy aliases.
5. Implement all required Match section, asset, feed status and Admin endpoints with consistent 400/401/404/409/500 contracts.
6. Implement SSE cursor storage/replay window, heartbeat and explicit resync-required response on a gap.
7. Reuse existing project auth if present; otherwise require an environment secret with constant-time comparison. Never return/log the secret.
8. Add Admin audit entries for restart, retry-missing, force-finalize and asset retry commands.
9. Add metrics for live count, event/odds rate, delay, job/failure counts and heartbeat/odds freshness.
10. Add local D1/Worker tests for replay, partial failure, route aliases, auth, SSE reconnect/gap and generation rollover.
11. Document route/version/alias matrix, auth setup, curl examples and recovery behavior.

## Acceptance Criteria

- Replaying an outbox batch creates no duplicate domain history or asset metadata.
- Failed/partial delivery leaves work retryable; acknowledgement marks only matching generation/content versions.
- A stale generation cannot reactivate or acknowledge the current generation.
- Versioned routes provide every required live/detail/odds/H2H/lineup/stats/incidents/assets/feed behavior.
- Requirement paths and legacy aliases are semantically equivalent to the canonical projection for supported fields.
- Legacy routes remain present; this task contains no removal switch.
- SSE updates use durable IDs, heartbeat, Last-Event-ID and REST resync after detected gaps.
- Unauthorized Admin/settings writes return 401/403; accepted actions create audit records.
- API/log responses expose no token, cookie, auth header, chat payload or source image URL.
- R2 asset responses use internal keys and correct validated MIME types.
- Feed status and all required metrics are queryable.
- D1/R2 replay and generation rollover integration tests pass against local bindings.

## Validation Commands

    cargo fmt -- --check
    cargo test sync_
    cargo test outbox_
    cargo test r2_
    cargo test generation_
    cargo test
    node --check dashboard/src/index.js
    cd dashboard && npm run test:api
    cd dashboard && npx wrangler deploy --dry-run

Reviewer must run the local Worker/D1 fixture sequence twice, inspect row counts after replay, test unauthorized and authorized Admin calls, and reconnect SSE with an old Last-Event-ID.

## Reference Map

- src/main.rs — current sync_worker and sync DTO/ack logic
- src/storage/schema.rs — Task 01 outbox/audit/feed schema
- src/storage/repositories.rs — Task 02/05/06 transaction and job methods
- dashboard/src/index.js — current /api/sync, /api/matches/live, /api/matches/detail and /api/settings handlers
- dashboard/wrangler.toml — current D1 binding and insecure token fallback to remove
- dashboard/schema.sql — Task 01 final D1 contract, read-only in this task
- tests/fixtures/sync-batch.json
- tests/fixtures/sync-batch-next-generation.json

