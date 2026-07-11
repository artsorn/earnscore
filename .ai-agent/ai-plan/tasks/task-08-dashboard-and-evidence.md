# Task 08: Live, History, Match Detail Dashboard and Delivery Evidence

## Status

Pending

## Outcome

A responsive EarnScore-owned Dashboard renders Football and Basketball Live/History/Detail views, applies SSE updates without full-page reloads, uses only internal assets, passes visual/accessibility/E2E tests, and provides the complete handoff evidence index.

## Dependencies

- Task 07 complete and reviewed.

## Affected Files

### Required Files

- dashboard/src/index.js — root route and server-side shell integration only
- dashboard/src/ui.js — new EarnScore-owned HTML/CSS/client JavaScript module
- dashboard/package.json — Playwright/UI scripts
- dashboard/package-lock.json — dependency lock updates
- dashboard/playwright.config.js — new deterministic desktop/tablet/mobile configuration
- dashboard/test/dashboard.spec.js — new functional, accessibility and visual E2E tests
- dashboard/test/visual/live-desktop.png — approved generated baseline
- dashboard/test/visual/live-tablet.png — approved generated baseline
- dashboard/test/visual/live-mobile.png — approved generated baseline
- dashboard/test/visual/finished-match.png — approved generated baseline
- dashboard/test/visual/incomplete-detail.png — approved generated baseline
- dashboard/test/visual/no-lineup.png — approved generated baseline
- tests/fixtures/dashboard-e2e.json — new deterministic two-sport UI/API sequence
- docs/installation.md — install/configure/run guide
- docs/evidence.md — Definition-of-Done evidence index and results
- docs/operations.md — monitoring, stale/source-changed and routine operations

### Allowed Files

- dashboard/wrangler.toml — local preview variables only; no secret or binding redesign
- docs/chrome-and-feed-operations.md — cross-link corrections only
- docs/migration-and-rollback.md — cross-link corrections only
- docs/recovery-and-finalization.md — cross-link corrections only
- docs/assets-and-storage.md — cross-link corrections only
- docs/api-sync-and-security.md — cross-link corrections only

## Forbidden Files

- Cargo.toml
- Cargo.lock
- src/**
- dashboard/schema.sql
- dashboard/migrations/**
- source-site HTML/CSS/JavaScript/screenshots/logos/branding
- remote image URLs or source-domain requests
- backend API contract expansion beyond Task 07
- legacy route removal
- all agent framework/runtime/task-state files

## Scope

- Render two-sport Live page with competition/country/team assets, score, period/clock, period scores, main current odds, update time and feed freshness.
- Show final score for the configured grace then remove from Live while retaining History/detail access.
- Render /matches/{match_id} with Header, Overview, Current Odds vs Movement, H2H, Lineups, Stats and Timeline.
- Handle loading, empty, stale, disconnected, source-changed, incomplete detail, no-lineup and malformed optional fields safely.
- Consume SSE with cursor/gap recovery and patch only affected cards/sections.
- Use REST snapshot/resync and polling fallback without overlapping requests.
- Use internal asset routes only with safe fallbacks.
- Implement responsive, keyboard, focus, semantic and reduced-motion behavior.
- Create deterministic Playwright visual baselines and end-to-end fixture sequences.
- Assemble installation, operations and Definition-of-Done evidence.
- Do not change backend behavior or source extraction.

## Implementation Steps

1. Extract the embedded Dashboard shell/client code from dashboard/src/index.js into dashboard/src/ui.js while leaving API handlers unchanged.
2. Build keyed Live/History cards and preserve active sport, scroll, expanded state and selected detail during updates.
3. Implement SSE connection, heartbeat/stale display, Last-Event-ID reconnect, gap-triggered REST resync and bounded polling fallback.
4. Implement the Match detail route/tabs and explicit missing/incomplete states.
5. Render upstream text through safe text APIs/escaping and validate internal asset URLs.
6. Add responsive layouts for desktop/tablet/mobile plus keyboard/focus/accessibility behavior.
7. Seed deterministic fixture sequences for new Live Match, score/odds changes, disconnect, resume, finish, finalization and removal from Live.
8. Add Playwright functional/visual tests for every required viewport/state and generate named baselines.
9. Verify Network requests contain no source-domain images and updates never reload the whole page.
10. Complete installation/operations docs and an evidence index linking architecture, ER, migrations, test results, sample event/odds/recovery data, screenshots, no-URL and no-repeat proofs and rollback procedure.

## Acceptance Criteria

- Football and Basketball appear with separate counts/filters and sport-correct period/score presentation.
- A newly Live Match appears through SSE; score/odds changes patch its card without full-page reload or losing UI state.
- Finished score appears for the configured grace, leaves Live afterward and remains in History/detail.
- Match detail contains all required sections and clearly distinguishes Current Odds from Odds Movement.
- Incomplete detail and no-lineup fixtures render explicit safe states.
- SSE disconnect/gap displays stale state, performs REST resync and resumes without duplicate cards.
- No source-domain image/network request occurs; broken internal assets use an EarnScore-owned fallback.
- Malicious fixture text cannot inject HTML/script.
- Desktop, tablet, mobile, live, finished, incomplete and no-lineup visual baselines pass.
- Keyboard navigation, focus visibility, semantic labels and reduced motion pass the agreed checks.
- docs/evidence.md maps all 24 automated scenarios and all 15 Definition-of-Done deliverables to concrete commands/artifacts.
- Installation, Chrome/feed, migration/rollback, recovery, asset and API/security guides are cross-linked and consistent.

## Validation Commands

    node --check dashboard/src/index.js
    node --check dashboard/src/ui.js
    cd dashboard && npm run test:ui
    cd dashboard && npm run test:visual
    cd dashboard && npx playwright test
    cd dashboard && npx wrangler deploy --dry-run

Reviewer must inspect Playwright traces/network assertions in addition to screenshots. Screenshot approval alone is insufficient for realtime, security and no-remote-asset criteria.

## Reference Map

- dashboard/src/index.js — current embedded GUI, loadMatches, renderMatchCard, renderDetails, settings and API handlers
- dashboard/package.json — current Wrangler-only scripts
- dashboard/wrangler.toml — local Worker/D1/R2 bindings from Task 07
- dashboard/schema.sql — field meanings, read-only in this task
- tests/fixtures/sync-batch.json — two-sport API seed
- tests/fixtures/sync-batch-next-generation.json — generation transition seed
- docs/architecture.md
- docs/database-er.md
- docs/migration-and-rollback.md
- docs/chrome-and-feed-operations.md
- docs/recovery-and-finalization.md
- docs/assets-and-storage.md
- docs/api-sync-and-security.md

