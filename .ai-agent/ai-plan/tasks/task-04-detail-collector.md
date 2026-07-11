# Task 04: Section-Based Initial Detail Collector

## Status
Passed
Pending

## Outcome

A durable initial-detail pipeline loads only missing Match sections, retries only failed sections, stores non-canonical H2H references, and reclaims interrupted jobs without allowing score/odds events to reload completed detail.

## Dependencies

- Task 02 complete and reviewed.
- Task 03 complete and reviewed.

## Affected Files

### Required Files

- src/main.rs — detail worker composition and removal of monolithic direct-detail calls only
- src/detail/mod.rs — new collector/coordinator exports
- src/detail/types.rs — new section names, states, phases and non-serializable asset candidate type
- src/detail/jobs.rs — new durable job planner, leases, retry and jitter
- src/detail/extractor.rs — new section extraction and content hashing
- src/storage/repositories.rs — detail section/job/data/H2H repository methods
- src/feed/browser.rs — owned detail-target pool hooks only
- tests/fixtures/football-detail.json — hydrated Football section fixture
- tests/fixtures/basketball-detail.json — hydrated Basketball section fixture
- tests/fixtures/detail-partial.json — new partial/empty/failure fixture

### Allowed Files

- Cargo.toml — only if deterministic hashing/retry tests require support unavailable in current dependencies
- Cargo.lock — only as the mechanical result of an approved dependency change

## Forbidden Files

- dashboard/**
- src/assets/**
- src/recovery/**
- src/sync/**
- schema and migration files
- feed list/admission/status mapping outside the detail-target hook
- any persisted source image URL
- all agent framework/runtime/task-state files

## Scope

- Plan Initial jobs after MATCH_DISCOVERED_LIVE only.
- Represent Overview, Odds, H2H, Lineups, Stats, Incidents/Timeline and related entities as independent sections.
- Enforce Match + section + load phase uniqueness.
- Load only PENDING, FAILED_RETRYABLE or explicitly missing sections.
- Confirm true empty sections as EMPTY_CONFIRMED.
- Retry failed sections with configurable exponential backoff and jitter.
- Reclaim expired LOADING leases after process kill.
- Store H2H historical rows as external references, never canonical Matches.
- Extract source image candidates into an in-memory-only type for Task 05.
- Do not implement binary downloads, final recovery or Manual refresh.

## Implementation Steps

1. Extract existing detail decode/fetch/save responsibilities into detail modules.
2. Define section/phase/state enums and repository transitions.
3. Create durable Initial jobs only once per missing section after admission.
4. Use the owned detail-target pool with configured concurrency and requested Match/sport verification.
5. Compute section content hashes and write each successful section transactionally.
6. Separate H2H external references from canonical Match persistence.
7. Ensure image candidate URLs cannot be serialized or written by repository/outbox code.
8. Implement retry schedule, jitter, lease expiry and permanent/manual-retry states.
9. Add tests for mixed success, empty sections, wrong hydrated Match, duplicate discovery, process kill and completed-section no-repeat.

## Acceptance Criteria

- First Live discovery produces at most one Initial job per missing section.
- Replaying discovery or changing score/odds produces no new job for a completed section.
- A partial run retries only failed/missing sections.
- Verified empty source sections become EMPTY_CONFIRMED and do not loop.
- Wrong Match/sport detail responses are discarded.
- Job concurrency respects configured limit and expired LOADING jobs are reclaimed.
- H2H results are queryable as references but do not appear in canonical Matches, live lists or recovery candidates.
- Asset candidate URLs remain in memory and are absent from DB, jobs, outbox and logs.
- Process restart resumes unfinished jobs without repeating completed sections.
- Initial and non-Initial phases cannot be claimed concurrently for the same Match.

## Validation Commands

    cargo fmt -- --check
    cargo test detail_section_
    cargo test detail_job_
    cargo test detail_retry_
    cargo test detail_lease_
    cargo test h2h_reference_
    cargo test detail_no_repeat_
    cargo test source_url_
    cargo test

## Reference Map

- src/main.rs — current detail_activate_tabs_js, detail_extract_js, fetch_detail_from_target, decode_and_validate_detail, get_matches_needing_detail and save_match_detail
- src/feed/browser.rs — Task 03 owned target registry and detail-target hook
- src/storage/schema.rs — Task 01 detail/job/H2H schema
- src/storage/repositories.rs — Task 02 repository transaction patterns
- tests/fixtures/football-detail.json
- tests/fixtures/basketball-detail.json
- tests/fixtures/source-filter-states.json

