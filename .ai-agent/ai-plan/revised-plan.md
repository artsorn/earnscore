# Revised Implementation Plan — After Planner Grill Round 002

> **สถานะ: GRILL COMPLETE — NO BLOCKERS — PLANNING INPUT ONLY**
>
> ผู้ใช้เลือกคำแนะนำทั้งหมดใน Round 002 แล้ว แผนนี้จึงไม่มี decision blocker เหลือ แต่ยังไม่ใช่ implementation task สำหรับ Coder; ต้องทำ targeted source inventory และสร้าง task files ใน invocation ที่อนุญาตก่อน

## Goal

สร้าง EarnScore ใหม่เป็นระบบ event-driven end-to-end สำหรับ Football และ Basketball ด้วย shared domain core และ source adapter แยกกีฬา รับ event จาก standalone headless browser ที่ระบบเป็นเจ้าของ สร้าง canonical Match เฉพาะคู่ที่พบ Live หรือมีหลักฐานว่าเริ่มแข่งขันแล้ว เก็บ score/state/odds แบบ idempotent ดึง detail และ asset แบบ section-aware โดยไม่โหลดซ้ำ ทำ recovery/finalization หลัง feed หยุด และส่งข้อมูลจาก SQLite ผ่าน transactional outbox ไป Cloudflare D1/R2, versioned REST และ SSE เพื่อขับ Live, History และ Match Detail Dashboard

Architecture ภายใน rewrite ได้ทั้งหมด แต่ migration ต้องไม่ทำลาย legacy data, ต้องรักษา Match IDs, ต้องมี verified backup/rollback และต้องคง legacy API routes เป็น compatibility aliases ชั่วคราวจน versioned API ผ่าน cutover verification

## Decisions from user answers

| Round | Answer | Locked decision | Consequence |
|---|---:|---|---|
| 001 Q1 | 1 | Football และ Basketball ต้องรองรับ end-to-end ใน delivery เดียว | Shared schema/domain rules; sport-specific adapters/fixtures/UI states |
| 001 Q2 | 4 | ใช้ standalone headless browser ที่ระบบจัดการทั้งหมด | Collector เป็นเจ้าของ process/pages; ห้ามแตะ browser/profile/tab ของผู้ใช้ |
| 001 Q3 | 4 | Rewrite internals ได้และยอมรับ breaking internal architecture | Code/architecture เดิมไม่ใช่ constraint แต่ data/ID/cutover ถูกจำกัดด้วย Round 002 |
| 001 Q4 | 1 | H2H history เป็น non-canonical external references | H2H finished rows ห้ามเข้า canonical matches หรือ recovery flow |
| 001 Q5 | 1 | ใช้ versioned additive REST + SSE, protected Admin API และ vertical slices | ต้องมี cursor/resync, auth/audit, compatibility tests และ incremental rollout |
| 002 Q1 | USE ALL AI RECOMMENDATIONS | Migrate legacy data แบบไม่ทำลาย รักษา Match IDs และคง legacy routes จน cutover ผ่าน | Legacy route removal เป็น release แยกหลัง shadow verification และ rollback window |

## Explicit assumptions

- Headless runtime คือ Chromium/Chrome process และ feed/detail pages ที่ collector สร้างและลงทะเบียนเองผ่าน CDP หรือ automation interface ที่ stack รองรับ
- Football และ Basketball มี feed target แยก แต่ใช้ normalized event envelope, admission gate, state machine, odds processor, jobs และ persistence core ร่วมกัน
- หาก source ปฏิเสธ headless หรือ payload ไม่ผ่าน validation ระบบแสดง BROWSER_UNAVAILABLE หรือ SOURCE_CHANGED, หยุด mutation และเก็บเฉพาะ sanitized diagnostics; ไม่ fallback ไป browser ผู้ใช้
- Network/XHR/Fetch/WebSocket interception มาก่อน state-store subscription; DOM observer เป็น fallback และ polling เป็น watchdog เท่านั้น
- Read APIs ใช้ same-origin/read-only policy ของ deployment; Admin APIs reuse auth เดิมถ้ามี มิฉะนั้นใช้ environment secret, constant-time verification และ audit record
- SSE ใช้ durable cursor/event ID; client ตรวจ gap และ REST resync ก่อนรับ incremental eventsต่อ
- Defaults เป็น config: finished grace 3 นาที, heartbeat 5 วินาที, stale timeout 20 วินาที, recovery grace 6 ชั่วโมง, concurrency detail/asset/recovery 3/5/2 และ retry 10s/30s/2m/5m/15m พร้อม jitter
- UNKNOWN_TERMINAL เป็น recovery outcome แยกจาก internal status UNKNOWN เว้นแต่ source inventory พบ model เดิมที่เหมาะกว่า
- Final Refresh สำเร็จได้หนึ่งครั้งต่อ Match ต่อ finalization version; manual force action สร้าง version และ audit ใหม่
- Source image URLs, cookies, tokens และ auth data ถูก sanitize ก่อน persistence ทุกชนิด รวม JSON, jobs, outbox, logs และ diagnostics
- Asset retry หลัง process restart ต้อง reacquire source URL จาก detail section; ห้าม persist URL เพื่อใช้ retry
- Visual baselines มาจาก EarnScore components และ deterministic fixtures; source ใช้เป็น reference เฉพาะ hierarchy/interaction
- Legacy routes เป็น aliases ชั่วคราวไปยัง service ใหม่และต้องให้ผลเชิงความหมายเทียบเท่าเดิมระหว่าง cutover; removal ต้องมี usage evidence, rollback window และอนุมัติ release แยก
- Exact implementation paths ยังไม่ระบุเพราะ grill ใช้ Minimal Context; targeted source inventory จะกำหนด Allowed/Required/Forbidden Files ก่อน task creation

## Scope

### In scope

- Standalone headless browser lifecycle, system-owned Football/Basketball targets, heartbeat, reconnect, watchdog และ source-change detection
- Sport adapters สำหรับ network/store/DOM extraction และ normalized events
- Live-only admission, internal state machine, event log, score/state history, current/history odds และ stale-event protection
- SQLite WAL, atomic unit-of-work, unique event/job keys, leases, finalization versions และ transactional outbox
- Non-destructive legacy migration/import, Match ID preservation, backup/restore, shadow verification และ temporary route aliases
- Initial detail per section, missing-only retry, non-canonical H2H references, recovery และ final reconciliation
- Ephemeral asset download, validation, SHA-256 dedup, atomic local storage, R2 upload และ internal asset routes
- D1/R2 synchronization, versioned REST, SSE, Admin API authorization/audit, status และ metrics
- Two-sport Live/History/Match Detail Dashboard, responsive states และ Playwright visual regression
- Unit, integration, browser, migration, restart, recovery, API security, realtime, visual tests และ Definition-of-Done evidence

### Out of scope

- Schedule/upcoming crawler, finished-page backfill หรือ canonical Match ที่ไม่เคยพบ Live
- Canonical matches สำหรับ H2H history
- Chat, comments, ads, tracking, betting transactions, predictions และ unrelated user features
- Copying source code/HTML/CSS/assets/branding, iframe, proxy HTML, hotlink หรือ bypass source controls
- User-browser fallback, browser extension หรือการจัดการ tabs/processes ที่ collector ไม่ได้สร้าง
- Unrelated account, billing, business หรือ deployment redesign
- Destructive legacy migration, Match ID remapping, deletion of backup หรือ legacy route removalใน delivery นี้
- Committing browser profiles, DB backups, raw source samples, vendor/build outputs หรือ secrets

## Design invariants

1. Admission gate ทำงานก่อน canonical persistence และก่อน enqueue detail/asset jobs
2. Canonical identity รักษา Match ID เดิมและไม่แตกต่างเพราะ sport adapter, reconnect หรือ migration
3. Event key ใช้ match, type, source timestamp และ payload hash; เมื่อไม่มี timestamp ใช้ normalized payload hash
4. Event เก่าบันทึก append-only ได้ถ้า unique แต่ห้าม overwrite snapshot/current odds ที่ใหม่กว่า
5. Snapshot, histories, current odds, job intent และ outbox จาก mutation เดียวกัน commit ใน transaction เดียว
6. Detail uniqueness เป็น match + section + load phase; score/odds handlers ห้าม reload completed detail
7. Initial, Final และ Manual phases ของ Match เดียวกันต้องมี lock/lease และห้ามทำพร้อมกัน
8. H2H external references ไม่ถูก feed/recovery/history APIs ตีความเป็น collected canonical Matches
9. Source asset URL อยู่ใน memory ของ active attempt เท่านั้น
10. Finalized data immutable ยกเว้น audited versioned admin action
11. Outbox เป็น at-least-once; D1/R2/API projections ต้อง idempotent และ reconcile partial failure ได้
12. Parser mismatch fail closed และห้าม fabricate terminal state
13. Legacy aliases และ versioned APIs อ่าน projection เดียวกันระหว่าง cutover เพื่อป้องกัน divergence
14. Legacy route removal ห้ามรวมใน implementation tasks ชุดนี้

## Affected files/modules

Exact paths ต้องได้จาก targeted inventory; module boundaries ที่อนุมัติในระดับแผนมีดังนี้:

| Module | Responsibility | Boundary |
|---|---|---|
| Schema/migration/import | New event/detail/recovery/asset/outbox/H2H schema, legacy mapping, ID preservation, backup/version markers | Non-destructive only |
| SQLite repositories/unit of work | WAL, atomic writes, monotonic compare, uniqueness, leases and locks | No browser/UI logic |
| Shared feed domain | Event envelope, sanitizer, admission, state/odds processors, source-change guard | Source-neutral |
| Headless runtime | Process/page ownership, network/store/DOM capture, heartbeat/reconnect/watchdog | No user browser |
| Football/Basketball adapters | Sport-specific identity/status/period/score/odds mapping and fixtures | No persistence policy |
| Detail collector | Section planner/extraction/retry/content hashes/H2H refs | No score-triggered reload |
| Asset pipeline | Ephemeral URL, validation, hash/dedup, atomic files and upload state | No persisted source URL |
| Recovery/finalizer | Pending selection, resume, grace, terminal reconciliation and final versions | No global history crawl |
| Cloud sync/Worker | Outbox consumer, D1 projection, R2 serving and reconciliation | Idempotent only |
| REST/SSE/admin | Versioned routes, legacy aliases, cursor/resync, auth/audit and metrics | Alias removal excluded |
| Dashboard | Live/History/detail tabs, partial updates, internal assets and visual states | Project-owned UI |
| Tests/fixtures/docs | Deterministic event sequences, migration/restart/security/visual evidence and runbooks | Sanitized only |

## Forbidden files/modules

### During this planner turn

- .ai-agent/ai-plan/tasks/task-*.md
- .ai-agent/ai-plan/overview.md and .ai-agent/ai-plan/context.md
- Implementation code, schema, migrations, tests, packages, deployments and generated artifacts
- Runtime logs, token/jsonl/cache/tmp/history and unrelated framework/generated state

### During implementation unless an exact task allows it

- Unrelated account, billing, business, admin and deployment modules
- User Chrome profile/tabs, browser extensions and arbitrary processes
- Legacy source database, verified backups and public Match IDs
- Legacy route deletion or semantic breakage during the compatibility window
- Vendor directories, build output, browser profiles, DB backups and raw captures in source control
- Source-site code, HTML, CSS, screenshots, logos, branding, iframe/proxy/hotlink
- Any persistence/log path retaining source image URLs, cookies, tokens or authentication headers
- Cross-module edits outside each task's explicit Allowed Files

## Risks

| Risk | Impact | Mitigation / stop condition |
|---|---|---|
| Legacy mapping is incomplete | Data or ID loss | Preflight inventory, deterministic mapping report, quarantine unmappable fields, backup/restore and stop cutover on count/ID mismatch |
| Legacy alias differs from versioned API | Existing flow regression | Shared projection, contract snapshots, shadow traffic and equivalence tests |
| Headless behavior differs from normal Chrome | Empty or misleading feed | Source contract validation, fail-closed status and deterministic fixtures |
| Football/Basketball lifecycle differs | Wrong period/state/odds | Separate adapters and fixtures plus shared invariant suite |
| Missing/out-of-order timestamps | Duplicate/regressed snapshot | Normalized hash and monotonic comparator tests |
| Detail/asset restart loses URL | Retry stalls | Persist intent only, reacquire URL and test kill between extraction/download |
| Initial/final/manual race | Duplicate or partial finalization | Unique phase keys, per-match locks, leases and restart tests |
| H2H leaks into canonical flow | Prohibited finished Matches | Separate type/repository and negative tests |
| D1/R2 partial failure | Cloud inconsistency | Idempotency, per-object state, replay/dead-letter and reconciliation |
| SSE cursor gap | Stale Dashboard | Heartbeat, gap detection, REST resync and stale UI |
| Raw JSON/log leaks URL or secret | Security/acceptance failure | Recursive sanitizer and DB/log/job scans |
| Eight-task cap hides oversized work | Incomplete Coder round | One primary deliverable per task; targeted inventory must split boundary before task creation if file/test scope exceeds one round |

## Acceptance criteria

### Migration and compatibility

- Verified backup and restore drill complete before migration
- Migration/import reruns safely and never deletes legacy data
- All mappable legacy Matches preserve their Match IDs; unmappable fields produce an explicit report without invented values
- Legacy detail becomes section-based and legacy odds become structured current/history with provenance
- Legacy routes remain compatibility aliases until versioned API and Dashboard pass shadow verification
- Alias responses are semantically equivalent for supported legacy fields
- Rollback can return traffic to the legacy system/database without losing post-migration audit evidence

### Feed and persistence

- Headless runtime owns and reconnects Football/Basketball pages without user-browser interaction
- Both adapters emit normalized events and heartbeat every 5 seconds or faster
- Scheduled/upcoming and never-seen-live finished Matches are not created
- Score, clock, period, status and odds update event-by-event without full-page reload
- Duplicate/stale events do not duplicate history or regress current snapshot
- Important writes and outbox records are atomic under process kill/restart
- Wrong page/sport, listener loss and source mismatch fail closed with explicit status

### Detail, H2H, assets and recovery

- First Live discovery loads only missing sections
- Completed sections do not reload on score/odds/reconnect/restart
- H2H historical results remain non-canonical
- Retry targets only failed/missing sections and expired leases reclaim safely
- Still-live recovery resumes without repeating completed detail
- Offline-finished Match receives exactly one final refresh per version
- Assets validate, deduplicate and publish locally/R2; no source image URL exists in DB, jobs, logs, outbox or Dashboard requests

### API, realtime and Dashboard

- Required behavior is available via versioned REST and temporary legacy aliases
- SSE updates only affected cards/sections and supports cursor, heartbeat, gap detection and REST resync
- Admin APIs require authorization and write audit records
- Live page supports both sports, live score/period/main odds/feed freshness and configured finished grace
- Match detail shows Header, Overview, Current Odds vs Movement, H2H, Lineups, Stats and Timeline with incomplete/no-lineup states
- Desktop/tablet/mobile plus live/finished/incomplete/no-lineup fixtures pass Playwright visual tests
- No full-page reload or source-domain image request occurs during live updates

### Delivery evidence

- Tests map to all 24 automated scenarios in the requirement
- Architecture/ER diagrams, migration files, changed-file list, test results, event/odds/recovery samples, screenshots, no-URL/no-repeat evidence, install/troubleshooting guide and rollback procedure are included
- A permitted live smoke test or deterministic fixture sequence covers discovery, changes, disconnect, still-live recovery, offline finish and finalization

## Pre-task gate

No user decision blocker remains. Before task creation, Planner must:

1. Perform targeted inventory of schema, runtime entrypoints, APIs, Dashboard and tests under the allowed context/search policy
2. Replace module names with exact Allowed/Required/Forbidden Files and validation commands
3. Confirm each task below fits one Coder round; reduce its file and acceptance boundary if not
4. Preserve the eight-task maximum and dependency order
5. Do not include legacy route removal in these tasks

## Proposed task breakdown — GRILL COMPLETE, TASK FILES NOT YET CREATED

### T1 — Persistence schema and non-destructive legacy migration

- Primary deliverable: SQLite/D1-compatible schema, repeat-safe import, backup/preflight/restore and foundational repositories
- Includes: ID preservation, detail/odds conversion, event/jobs/recovery/assets/outbox/H2H refs and migration reports
- Excludes: browser, workers, APIs and UI
- Acceptance: rerun, restore, ID/count/hash, no-new-non-live and no-source-URL migration tests pass

### T2 — Event, match-state, odds and transactional outbox core

- Primary deliverable: source-neutral processor for admission, deduplication and atomic/monotonic state, odds, history and outbox writes
- Excludes: extraction, detail/recovery, cloud and UI
- Acceptance: admission, duplicate/stale score/odds, current/history and transaction rollback tests pass for both sports

### T3 — Headless feed runtime and two sport adapters

- Primary deliverable: system-owned headless process/pages emitting normalized Football/Basketball events
- Includes: network/store/DOM priority, heartbeat, reconnect, watchdog, ownership and source diagnostics
- Excludes: domain persistence, detail/assets, APIs and UI
- Acceptance: adapter fixtures, page/process restart, no user-browser access and source-mismatch tests pass

### T4 — Section-based initial detail collector

- Primary deliverable: Initial section planning/extraction for Overview, Odds, H2H, Lineups, Stats, Incidents and related entities
- Includes: states, hashes, retries, jitter, leases and non-canonical H2H refs
- Excludes: binary assets, final recovery, cloud and UI
- Acceptance: first discovery, no-repeat, missing-only retry, empty/failure and kill/reclaim tests pass

### T5 — Ephemeral asset pipeline and storage

- Primary deliverable: image validation, deduplication, atomic local storage and R2-ready metadata without persisted source URL
- Excludes: section parsing, recovery, general API and UI
- Acceptance: duplicate reuse, corrupt rejection, URL reacquisition after restart and recursive no-URL scans pass

### T6 — Recovery and versioned finalization

- Primary deliverable: reconnect/startup recovery and resumable one-refresh-per-version final reconciliation
- Includes: still-live, missing grace, terminal states, locks/leases and admin force versioning
- Excludes: global history crawl, cloud transport and UI
- Acceptance: no-repeat resume, offline finish, not-found grace, terminal variants, concurrency and kill/restart tests pass

### T7 — Cloud sync, versioned REST, legacy aliases, SSE and admin security

- Primary deliverable: idempotent D1/R2 delivery plus required APIs, compatibility aliases and realtime stream
- Includes: replay/reconciliation, cursor/resync, status/metrics, authorization/audit and alias equivalence tests
- Excludes: Dashboard visuals and legacy route removal
- Acceptance: partial-failure convergence, API/auth/contracts, alias parity and SSE gap tests pass

### T8 — Live/History/Match Detail Dashboard and delivery evidence

- Primary deliverable: responsive project-owned UI using T7 contracts and internal assets
- Includes: partial updates, finished grace, stale/incomplete/no-lineup states, odds movement, visual/accessibility/E2E tests and evidence index
- Excludes: backend contract expansion, parser/schema changes and source-site assets
- Acceptance: required responsive/state baselines, no full reload/source-domain requests and end-to-end fixture evidence pass

## Remaining blockers

No blocker questions remain after Round 002.
