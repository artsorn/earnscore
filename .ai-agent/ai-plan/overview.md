# EarnScore Event-Driven Live Feed — Implementation Overview

## Status

PLAN APPROVED and frozen for Coder/Reviewer execution.

This plan replaces the previous task sequence. It does not authorize destructive legacy cleanup or removal of compatibility routes.

## Objective

Deliver an event-driven Football and Basketball pipeline that:

- admits only Matches observed Live or with reliable started evidence
- records score, clock, period, status and odds changes idempotently
- loads initial detail once per missing section
- performs selective recovery and one final refresh per finalization version
- downloads and serves assets locally or from R2 without persisted source image URLs
- syncs SQLite changes to D1/R2 through a transactional outbox
- exposes versioned REST, temporary legacy aliases, protected Admin APIs and SSE
- renders responsive Live, History and Match Detail views without full-page reloads

## Locked decisions

- Both Football and Basketball are end-to-end scope.
- The collector owns a standalone headless Chrome/Chromium process and all feed/detail pages it creates.
- Internal architecture may be rewritten.
- Legacy data migration is non-destructive, preserves Match IDs and begins with a verified backup.
- Existing API routes remain compatibility aliases until the versioned API and Dashboard pass cutover verification.
- H2H historical matches are non-canonical references and never enter the collected Matches workflow.
- SSE is the primary Dashboard update channel; REST resync and polling fallback remain available.
- Admin routes require authorization and audit evidence.
- Legacy route removal is outside this plan.

## Non-negotiable invariants

1. Admission precedes canonical Match persistence and job creation.
2. Scheduled/upcoming and never-seen-live finished Matches are not created.
3. Event and job identities are deterministic and idempotent.
4. Old events cannot overwrite newer snapshots.
5. State/history/current odds/outbox writes are atomic.
6. Completed detail sections do not reload because of score, odds, reconnect or restart.
7. Initial, Final and Manual detail phases for one Match cannot run concurrently.
8. Final refresh succeeds once per Match per finalization version.
9. Source image URLs and secrets never persist in active DB rows, jobs, outbox, logs or diagnostics.
10. H2H references are not canonical Matches.
11. Parser mismatch fails closed and never fabricates a terminal result.
12. D1/R2 consumers tolerate at-least-once outbox delivery.
13. Legacy and versioned routes read the same projection during cutover.
14. Public Match IDs remain stable through migration.

## Current implementation baseline

- Rust binary: src/main.rs, currently monolithic
- Rust dependencies: Cargo.toml
- Cloudflare Worker and embedded UI: dashboard/src/index.js
- D1 bootstrap: dashboard/schema.sql
- Existing D1 migrations: dashboard/migrations/0001_full_payload.sql and 0002_dataset_generation.sql
- D1 legacy fixture: dashboard/test/legacy-schema.sql
- Worker config/scripts: dashboard/wrangler.toml and dashboard/package.json
- Existing source/sync fixtures: tests/fixtures/*.json

The implementation tasks introduce focused Rust modules while retaining src/main.rs as the CLI/composition root. Applied migrations 0001 and 0002 are historical and must not be rewritten; the new work starts at migration 0003.

## Task sequence

| Task | Primary deliverable | Depends on |
|---|---|---|
| 01 | Event-driven schema, non-destructive migration and backup/rollback contract | none |
| 02 | Admission, state, odds and transactional outbox domain core | 01 |
| 03 | Owned headless browser runtime and two sport adapters | 02 |
| 04 | Section-based initial detail collector | 02, 03 |
| 05 | Ephemeral asset pipeline and local asset store | 01, 04 |
| 06 | Recovery manager and versioned finalization | 02, 04, 05 |
| 07 | D1/R2 sync, versioned/legacy APIs, SSE and Admin security | 01, 02, 05, 06 |
| 08 | Live/History/Detail Dashboard, visual tests and handoff evidence | 07 |

Tasks are sequential unless the orchestrator proves their Required Files do not overlap. Each task must be completed and reviewed before a dependent task begins.

## Cross-task validation gates

### After Task 01

- Fresh and legacy SQLite/D1 paths converge on the v3 logical schema.
- Match IDs and rows survive migration.
- Backup and restore commands are documented and tested on temporary data only.

### After Task 03

- Both sports produce normalized Live events from owned headless targets.
- No existing user browser process or tab is reused.
- Wrong page, wrong sport and source-shape changes fail closed.

### After Task 06

- Discovery, detail, disconnect, still-live recovery, offline finish and one-version finalization pass deterministic fixture sequences.
- Restart tests pass at detail, asset and finalization boundaries.

### After Task 07

- Outbox replay and D1/R2 partial failures converge.
- Versioned routes, required unversioned routes and legacy aliases are contract-tested.
- SSE cursor gaps recover through REST resync.
- Admin routes reject unauthorized calls and audit accepted mutations.

### After Task 08

- Desktop, tablet and mobile visual states pass.
- No source-domain image request occurs.
- All 24 required automated scenarios have an evidence mapping.
- Installation, Chrome/feed troubleshooting, migration and rollback documentation is complete.

## Completion rule

Compile success is insufficient. The plan completes only when automated fixtures or a permitted live smoke test prove the full event sequence and all Definition-of-Done evidence is indexed in docs/evidence.md.

