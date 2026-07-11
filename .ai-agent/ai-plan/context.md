# EarnScore Event-Driven Live Feed — Execution Context

## Project shape

The current repository is intentionally small but concentrated:

- src/main.rs contains CLI parsing, Chrome target management, Football/Basketball adapters, extraction, SQLite migration/persistence, detail collection, sync and Rust tests.
- dashboard/src/index.js contains Worker routes, D1 operations and the embedded Dashboard HTML/CSS/client JavaScript.
- dashboard/schema.sql is the fresh D1 bootstrap.
- dashboard/migrations/0001_full_payload.sql and 0002_dataset_generation.sql are historical applied migrations.
- dashboard/test/legacy-schema.sql seeds the old D1 shape for migration verification.
- tests/fixtures contains sanitized Football, Basketball, detail, filter, target and sync fixtures.

The approved implementation progressively extracts Rust responsibilities into new modules. Do not perform a broad rewrite of unrelated code in one task.

## Approved architecture

    owned headless Chrome/Chromium
      -> Football and Basketball feed adapters
      -> normalized, sanitized event envelope
      -> admission/state/odds processors
      -> SQLite transaction
           feed event log
           Match snapshot/history/current odds
           durable section/recovery jobs
           asset metadata
           sync outbox
      -> leased detail, asset, recovery and sync workers
      -> Cloudflare D1 and R2
      -> versioned REST + required aliases + SSE
      -> Live / History / Match Detail Dashboard

## Identity and migration contract

- Existing Match IDs remain public canonical IDs.
- Existing dataset metadata remains supported, but dataset generation cannot bypass the live-only admission rule.
- Historical migrations 0001 and 0002 are read-only references.
- New D1 changes start in dashboard/migrations/0003_event_driven_live.sql.
- Local SQLite migration is repeat-safe and additive.
- Migration begins with a verified backup and supports restore.
- Legacy Match detail is converted into section status/content rows without inventing unavailable values.
- Legacy raw odds are converted with provenance; unparseable values are reported, not fabricated.
- Legacy image URLs may be read only during the Task 05 one-time in-memory asset conversion. They are removed from the active DB only after the corresponding asset is safely stored; the verified backup retains the original legacy data.
- Legacy API aliases remain until a separately approved removal release.

## Event and state contract

Required normalized event types:

- FEED_CONNECTED
- FEED_DISCONNECTED
- FEED_HEARTBEAT
- MATCH_DISCOVERED_LIVE
- MATCH_SCORE_CHANGED
- MATCH_CLOCK_CHANGED
- MATCH_PERIOD_CHANGED
- MATCH_STATUS_CHANGED
- MATCH_ODDS_CHANGED
- MATCH_REMOVED_FROM_LIVE
- MATCH_FINISHED

Every envelope carries event ID, optional source event ID, Match ID, sport ID, event type, source timestamp, received timestamp, payload hash, sanitized payload and feed session ID.

When a source timestamp is absent, idempotency uses Match ID, event type and normalized payload hash. Snapshot ordering uses source time when reliable, then received time and a deterministic tie-breaker.

## Internal Match states

- DISCOVERED_LIVE
- LIVE
- HALF_TIME
- PAUSED
- FINISHING
- FINISHED
- CANCELLED
- POSTPONED
- ABANDONED
- RECOVERY_PENDING
- FINALIZED
- UNKNOWN

UNKNOWN_TERMINAL is modeled as a recovery outcome, not a new canonical Match admission state.

## Detail and recovery contract

Detail sections include Overview, Odds, H2H, Lineups, Stats, Incidents/Timeline and related competition/team/venue/referee/player data.

Section states:

- PENDING
- LOADING
- COMPLETED
- EMPTY_CONFIRMED
- FAILED_RETRYABLE
- FAILED_PERMANENT
- FINAL_REFRESH_PENDING
- FINAL_COMPLETED

Job identity is Match ID + section + phase, where phase is INITIAL, RETRY, FINAL or MANUAL.

Recovery selects only Matches that were previously Live, are not Finalized and lost feed before a known terminal result. It never crawls all historical Matches.

## Asset contract

- Source URLs exist only in an active in-memory download attempt.
- New job/event/outbox rows never contain source image URLs.
- Downloads validate status, Content-Type and actual image bytes.
- Assets are SHA-256 deduplicated and atomically renamed.
- Active DB fields store asset IDs, local paths, storage keys and image metadata.
- Dashboard requests only internal asset routes or the configured internal R2 domain.
- Retry after restart reacquires the URL from the source detail page.

## API contract

Canonical versioned routes use the /api/v1 prefix.

Required read routes also remain available at the requirement paths:

- /api/live/matches
- /api/live/events
- /api/matches/{match_id} and section subroutes
- /api/assets/{asset_id}
- /api/feed/status

Existing routes such as /api/matches/live and /api/matches/detail remain compatibility aliases during cutover.

Admin routes:

- /api/admin/feed/restart
- /api/admin/matches/{match_id}/retry-missing
- /api/admin/matches/{match_id}/force-finalize
- /api/admin/assets/{asset_id}/retry
- /api/admin/recovery-jobs
- /api/admin/detail-jobs

SSE uses durable cursor IDs and supports heartbeat, Last-Event-ID, gap detection and REST resync.

## Configuration defaults

- detail concurrency: 3
- asset concurrency: 5
- recovery concurrency: 2
- heartbeat: 5 seconds
- stale timeout: 20 seconds
- recovery grace: 6 hours
- finished-card grace: 3 minutes
- retry schedule: 10 seconds, 30 seconds, 2 minutes, 5 minutes, 15 minutes with jitter

All values are configurable and validated to prevent zero-delay loops or unbounded concurrency.

## Security and legal boundaries

- No source cookies, tokens, auth headers or unsanitized HTML/state/network samples in DB or logs.
- Admin secrets come from existing auth or environment configuration; never from committed defaults.
- Do not copy source HTML, CSS, JavaScript, screenshots, logos or trademarks.
- Do not iframe or proxy source HTML.
- Do not bypass source access controls.
- Visual tests use EarnScore-owned components and deterministic fixtures.

## Validation conventions

Rust tasks run formatting and focused test filters before the full suite:

    cargo fmt -- --check
    cargo test

Worker tasks run:

    node --check dashboard/src/index.js
    cd dashboard && npx wrangler deploy --dry-run

D1 migration tests use separate temporary persist directories for fresh and legacy paths. Destructive commands are prohibited against production/user databases.

Dashboard tasks use Playwright with deterministic D1/HTTP fixtures and must test Desktop, Tablet, Mobile, Live, Finished, incomplete detail and no-lineup states.

## Review stop conditions

Reviewer must stop the task if any of the following occurs:

- a Scheduled or never-seen-live Finished Match is created
- Match IDs change during migration
- a completed detail section reloads without an allowed phase
- a source image URL or secret persists in active storage/logs
- a parser mismatch writes state or marks a Match terminal
- an old event overwrites newer state
- initial and final jobs run concurrently
- final refresh repeats in the same version
- an outbox replay duplicates domain history
- a legacy route changes semantics before cutover approval
- implementation touches a Forbidden File

