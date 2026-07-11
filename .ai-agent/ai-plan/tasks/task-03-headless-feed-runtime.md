# Task 03: Owned Headless Feed Runtime and Sport Adapters

## Status

Pending

## Outcome

EarnScore launches and owns a standalone headless Chrome/Chromium process, maintains isolated Football and Basketball Live targets, captures source changes with network/store/DOM priority, and emits validated normalized events without touching a user browser.

## Dependencies

- Task 02 complete and reviewed.

## Affected Files

### Required Files

- src/main.rs — CLI/composition wiring for the owned browser and feed sessions only
- src/feed/mod.rs — new feed coordinator and session exports
- src/feed/browser.rs — new process ownership, target registry, CDP routing and cleanup
- src/feed/adapters/mod.rs — new common adapter contract
- src/feed/adapters/football.rs — Football extraction/status/score/odds mapping
- src/feed/adapters/basketball.rs — Basketball extraction/status/score/odds mapping
- tests/fixtures/chrome-targets.json — owned/unowned/process/page target cases
- tests/fixtures/source-filter-states.json — All/Live/readiness/source-change states
- tests/fixtures/football-live.json — sanitized source-contract fixture
- tests/fixtures/football-finished.json — same-identity terminal transition
- tests/fixtures/basketball-live.json — sanitized source-contract fixture
- tests/fixtures/basketball-finished.json — same-identity terminal transition
- docs/chrome-and-feed-operations.md — install, launch, health and troubleshooting guide

### Allowed Files

- Cargo.toml — only for process/CDP support not available in current Tokio/HTTP/WebSocket dependencies
- Cargo.lock — only as the mechanical result of an approved dependency change

## Forbidden Files

- dashboard/**
- src/detail/**
- src/assets/**
- src/recovery/**
- src/sync/**
- schema and migration files
- user Chrome profiles, tabs or processes
- committed browser profiles or raw unsanitized captures
- all agent framework/runtime/task-state files

## Scope

- Launch an owned headless browser with an isolated temporary profile and bounded startup timeout.
- Create one owned Live feed target per sport and record process/target/session ownership.
- Intercept XHR/Fetch/WebSocket responses first, then source store, then DOM mutation fallback.
- Use polling only as a no-event watchdog.
- Verify correct sport and active Live filter before event emission.
- Emit heartbeat at least every five seconds.
- Reconnect/recreate only owned targets after page reload, socket loss or browser failure.
- Validate source envelopes and fail closed as SOURCE_CHANGED when extraction confidence is lost.
- Convert Football/Basketball payloads into Task 02 domain events.
- Do not collect detail pages or implement recovery decisions.

## Implementation Steps

1. Extract existing target/router/adapters from src/main.rs into focused feed modules.
2. Replace normal-mode attachment to arbitrary remote Chrome with owned process launch and isolated profile. A fixture-only/test hook may emulate CDP.
3. Track browser PID, profile directory, target ID, role, sport and session ID; cleanup only owned resources.
4. Implement the source-priority ladder and stable readiness checks for correct Live filter/sport.
5. Add heartbeat, stale detection and reconnect state transitions.
6. Add schema/shape guards and sanitized diagnostics with no cookies, tokens or remote image URLs.
7. Normalize Football/Basketball status, score, clock, period and odds into Task 02 envelopes.
8. Test wrong page/sport, All filter, stale previous state, browser crash, page reload, socket loss and source-shape mismatch.
9. Document executable discovery, headless launch flags, health states and safe troubleshooting.

## Acceptance Criteria

- Normal startup creates an owned browser process and two isolated sport feed targets.
- Existing user targets in fixtures are never navigated or closed.
- Only correct-sport, active-Live, stable snapshots emit Match events.
- Network events are preferred; store and DOM fallbacks produce the same normalized contract.
- Watchdog polling does not trigger full-page reload loops.
- Heartbeat is recorded every five seconds or faster; 20 seconds without valid heartbeat becomes stale/disconnected.
- Browser/page/socket failure reconnects without duplicating already-seen events.
- Source mismatch stops domain mutation and reports SOURCE_CHANGED; it does not mark Matches Finished.
- Football and Basketball fixtures preserve identity across Live-to-terminal transitions.
- Diagnostics and committed fixtures contain no cookie, token, auth header, chat or source image URL.
- Shutdown removes the isolated profile/owned targets and never touches user resources.

## Validation Commands

    cargo fmt -- --check
    cargo test browser_ownership_
    cargo test feed_session_
    cargo test source_contract_
    cargo test source_filter_
    cargo test football_adapter_
    cargo test basketball_adapter_
    cargo test source_changed_
    cargo test
    cargo run -- --help

A reviewer may run a permitted live smoke test with a temporary profile, but deterministic fixtures remain mandatory.

## Reference Map

- src/main.rs — current Cli, OwnedTarget, TargetRole, FootballAdapter, BasketballAdapter, WsRouter and main loop
- Cargo.toml — current Tokio, reqwest and WebSocket dependencies
- tests/fixtures/chrome-targets.json — existing target ownership cases
- tests/fixtures/source-filter-states.json — existing readiness/filter cases
- tests/fixtures/football-live.json
- tests/fixtures/football-finished.json
- tests/fixtures/basketball-live.json
- tests/fixtures/basketball-finished.json
- https://m.aiscore.com/en/ — Football runtime reference
- https://m.aiscore.com/basketball — Basketball runtime reference

