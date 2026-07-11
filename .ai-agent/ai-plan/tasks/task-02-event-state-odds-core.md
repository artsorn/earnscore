# Task 02: Event, Match-State, Odds, and Transactional Outbox Core

## Status

Pending

## Outcome

A source-neutral Rust domain core admits only Live/started Matches, deduplicates normalized events, enforces monotonic state, records score/state and odds history, and creates sync outbox records atomically.

## Dependencies

- Task 01 complete and reviewed.

## Affected Files

### Required Files

- src/main.rs — domain module wiring and replacement of direct Match/odds persistence calls only
- src/domain/mod.rs — new domain exports
- src/domain/events.rs — new normalized envelope, event types, hashing and ordering
- src/domain/match_state.rs — new admission and internal state machine
- src/domain/odds.rs — new bookmaker/market/current/history normalization
- src/storage/mod.rs — repository transaction interface
- src/storage/repositories.rs — new event, Match, odds and outbox repositories
- tests/fixtures/event-sequence-football.json — new deterministic Football event sequence
- tests/fixtures/event-sequence-basketball.json — new deterministic Basketball event sequence

### Allowed Files

- Cargo.toml — only if canonical hashing/decimal handling cannot be implemented correctly with current dependencies
- Cargo.lock — only as the mechanical result of an approved dependency change

## Forbidden Files

- dashboard/**
- src/feed/**
- src/detail/**
- src/assets/**
- src/recovery/**
- tests/fixtures/football-detail.json
- tests/fixtures/basketball-detail.json
- historical migration files 0001 and 0002
- all agent framework/runtime/task-state files

## Scope

- Define the normalized event envelope and required event types.
- Implement deterministic event keys with and without source timestamps.
- Implement admission evidence and internal state transitions.
- Reject Scheduled/Upcoming and never-seen-live Finished/Cancelled/Postponed rows.
- Permit terminal updates only for existing previously-Live Matches.
- Persist feed events append-only with unique keys.
- Update latest Match snapshot and state history monotonically.
- Normalize multi-bookmaker markets without hard-coded bookmaker names.
- Insert odds history and upsert current odds only when values change.
- Create outbox rows in the same transaction as each accepted mutation.
- Do not implement source extraction, detail jobs, asset downloads, recovery, cloud delivery or UI.

## Implementation Steps

1. Introduce source-neutral domain types separate from AiScore payload shapes.
2. Canonicalize sanitized payloads before hashing; explicitly handle missing source timestamps.
3. Implement admission evidence checks for live status, clock, score and period with sport-aware evidence passed in by adapters.
4. Encode allowed internal state transitions, terminal immutability and stale-event ordering.
5. Implement current/history odds identity using Match, bookmaker, market, period, selection and line.
6. Add a repository unit-of-work that writes event, snapshot/history/current odds and outbox atomically.
7. Ensure rejected/duplicate/stale events have explicit outcomes and do not create detail jobs.
8. Add deterministic fixture tests for both sports, duplicates, same-time tie breaks, out-of-order events and transaction rollback.

## Acceptance Criteria

- Scheduled and Upcoming fixtures create no canonical Match.
- Finished/Cancelled/Postponed fixtures create no new Match when no prior Live admission exists.
- First valid Live evidence creates one canonical Match and one discovery history/outbox sequence.
- Replaying the same event creates no new feed/state/odds/outbox rows.
- A changed score updates the snapshot and appends history without requesting detail.
- Changed odds append one history row and update current odds atomically.
- Equal odds and stale odds do not append history or regress current values.
- Old score/status events remain auditable if unique but cannot overwrite a newer snapshot.
- Finalized Match mutation is rejected unless a later task supplies an audited Manual version.
- Transaction failure leaves no partial event/history/current/outbox write.

## Validation Commands

    cargo fmt -- --check
    cargo test event_
    cargo test admission_
    cargo test match_state_
    cargo test odds_
    cargo test outbox_
    cargo test transaction_
    cargo test

## Reference Map

- src/main.rs — current Match structs, status predicates, save_matches and SQLite transaction call sites
- src/storage/schema.rs — Task 01 v3 schema contract
- src/storage/migration.rs — Task 01 compatibility rules
- tests/fixtures/football-live.json — existing Football source fixture
- tests/fixtures/football-finished.json — existing Football terminal fixture
- tests/fixtures/basketball-live.json — existing Basketball source fixture
- tests/fixtures/basketball-finished.json — existing Basketball terminal fixture

