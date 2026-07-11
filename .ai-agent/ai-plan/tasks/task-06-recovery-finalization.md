# Task 06: Feed Recovery and Versioned Finalization

## Status

Pending

## Outcome

Feed disconnect/restart marks only unresolved previously-Live Matches for recovery, resumes still-Live Matches without repeating completed detail, and finalizes offline-finished Matches exactly once per finalization version.

## Dependencies

- Task 02 complete and reviewed.
- Task 04 complete and reviewed.
- Task 05 complete and reviewed.

## Affected Files

### Required Files

- src/main.rs — recovery worker composition and feed disconnect hooks only
- src/recovery/mod.rs — new recovery manager exports
- src/recovery/manager.rs — candidate selection, scheduling, grace and retry
- src/recovery/finalizer.rs — new phase locking, final section plan and immutable completion
- src/storage/repositories.rs — recovery jobs, phase locks, versions and terminal transition methods
- src/detail/jobs.rs — FINAL and MANUAL phase planning hooks only
- src/feed/mod.rs — disconnect/reconnect notification hooks only
- tests/fixtures/recovery-still-live.json — new disconnect/reconnect sequence
- tests/fixtures/recovery-finished-offline.json — new offline terminal/final-detail sequence
- tests/fixtures/recovery-not-found.json — new grace/terminal-unknown sequence
- docs/recovery-and-finalization.md — state flow, operator actions and troubleshooting

### Allowed Files

- Cargo.toml — only if deterministic time/failure-injection tests require support unavailable in current dependencies
- Cargo.lock — only as the mechanical result of an approved dependency change

## Forbidden Files

- dashboard/**
- src/sync/**
- schema and migration files
- broad finished/history crawling
- H2H refresh when a completed H2H section has no explicit reason to change
- mutation of Finalized rows without a new audited Manual version
- all agent framework/runtime/task-state files

## Scope

- Record feed disconnect and mark currently unresolved previously-Live Matches RECOVERY_PENDING.
- On startup/reconnect select only previously-Live, non-Finalized Matches whose final result was unknown.
- Reconcile still-Live score/clock/period/odds and load only missing sections.
- Reconcile Matches that finished while offline and plan required FINAL sections.
- Handle Cancelled, Postponed, Abandoned and not-found grace outcomes.
- Enforce one successful final refresh per Match per finalization version.
- Prevent Initial, Final and Manual phases from running concurrently.
- Reclaim interrupted recovery/finalization leases.
- Add audited Admin force-finalize/retry domain commands for Task 07 to expose.
- Do not implement Admin HTTP routes or cloud delivery.

## Implementation Steps

1. Add durable recovery job states, reasons, scheduling and lease transitions.
2. Hook valid feed disconnect/stale events without treating parser failure as Match finish.
3. Implement candidate query with the exact previously-Live/non-Finalized/unknown-result predicate.
4. Reconcile still-Live Matches through Task 02 event processing and Task 04 missing-only planning.
5. Implement the required FINAL section plan: Overview, closing/final Odds, final Stats, Incidents, missing Lineups and period scores.
6. Enforce phase exclusion and a finalization version/completion marker.
7. Retry not-found detail within configured grace; record terminal UNKNOWN outcome after expiry without creating a replacement Match.
8. Implement audited Manual version creation for force actions.
9. Add deterministic clock/failure-injection tests for process kill before/after each finalization step.
10. Document state transitions and safe operator actions.

## Acceptance Criteria

- Disconnect marks only unresolved currently-Live Matches RECOVERY_PENDING.
- Restart never scans or reloads all historical Matches.
- A still-Live Match resumes current state and does not reload completed sections.
- A Match finished while offline gets final state/period scores and exactly one successful FINAL refresh for the version.
- Replaying recovery/restarting after completion does not repeat final detail or odds history.
- Cancelled/Postponed/Abandoned are recorded only for existing previously-Live Matches.
- Not-found recovery retries within grace and ends with UNKNOWN terminal outcome without a new canonical Match.
- Initial, Final and Manual jobs cannot be claimed concurrently for one Match.
- Process kill during recovery/finalization is reclaimable and does not leave a partial immutable Match.
- Admin force action creates an audited new version; it never silently mutates the existing finalized version.

## Validation Commands

    cargo fmt -- --check
    cargo test recovery_candidate_
    cargo test recovery_still_live_
    cargo test recovery_finished_
    cargo test recovery_not_found_
    cargo test finalization_
    cargo test phase_lock_
    cargo test restart_
    cargo test

## Reference Map

- src/domain/events.rs — Task 02 normalized reconnect/state events
- src/domain/match_state.rs — Task 02 internal transitions and immutability
- src/detail/jobs.rs — Task 04 section/phase job planner
- src/assets/mod.rs — Task 05 required asset completion contract
- src/storage/schema.rs — Task 01 recovery/job/version tables
- src/storage/repositories.rs — existing durable repository boundary
- tests/fixtures/football-finished.json
- tests/fixtures/basketball-finished.json

