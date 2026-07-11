# Revised Implementation Plan — After Planner Grill Round 001

> **สถานะ: REVISED BUT BLOCKED — NOT YET APPROVED — NON-EXECUTABLE BY CODER**
>
> คำตอบ Q1, Q2, Q4 และ Q5 ถูกล็อกแล้ว แต่ Q3 เรื่อง full rewrite ยังขัดกับข้อกำหนด migration/Match ID และคำตอบเรื่อง additive versioned API จึงต้องตอบ Round 002 ก่อน ห้ามสร้าง implementation task files จากเอกสารนี้

## Goal

สร้าง EarnScore ใหม่เป็นระบบ event-driven end-to-end สำหรับ **Football และ Basketball** โดยใช้ shared domain core และ source adapter แยกกีฬา รับ event จาก standalone headless browser ที่ระบบเป็นเจ้าของ สร้าง canonical Match เฉพาะคู่ที่ระบบพบ Live หรือมีหลักฐานว่าเริ่มแล้ว เก็บ state/score/odds แบบ idempotent ดึง detail/asset แบบ section-aware โดยไม่โหลดซ้ำ ทำ recovery/finalization หลัง feed หยุด และส่งข้อมูลผ่าน SQLite transactional outbox ไป Cloudflare D1/R2, versioned REST และ SSE เพื่อขับ Live/History/Match Detail Dashboard

ระบบต้อง fail closed เมื่อ source เปลี่ยน, ไม่ persist source image URL หรือ secret, ไม่สร้าง Scheduled/Finished Match ที่ไม่เคย Live และไม่ถือว่างานเสร็จจนกว่าจะมี migration/restart/recovery/API/UI evidence ครบตาม requirement

## Decisions from user answers

| Round 001 | User choice | Locked decision | Planning consequence |
|---|---:|---|---|
| Q1 | 1 | Football และ Basketball ต้องทำ end-to-end ใน delivery เดียว ใช้ shared core + sport adapters | Schema/state/jobs/API/UI ต้องรองรับทั้งสองกีฬา; test fixtures ต้องครอบคลุม period/score/odds ของแต่ละกีฬา |
| Q2 | 4 | ใช้ standalone headless browser ที่ระบบจัดการทั้งหมด | Feed pages เป็น system-owned targets ไม่เกี่ยวกับ browser ผู้ใช้; ownership/restart tests ตรวจ process/page IDs ของ collector |
| Q3 | 4 | อนุญาต full rewrite, legacy import แบบ best-effort และยอมรับ breaking changes | Architecture เดิมไม่ใช่ compatibility constraint แต่ยังไม่ชัดว่าข้อมูล/Match IDs/routes ใดต้องรักษาตาม requirement; เป็น blocker รอบ 002 |
| Q4 | 1 | H2H historical matches เป็น non-canonical external references ใต้ `match_h2h` | ห้าม insert H2H finished rows เข้า canonical `matches`; feed/recovery/detail jobs ไม่เห็น external references เป็น Match |
| Q5 | 1 | ใช้ versioned additive REST + SSE, ป้องกัน Admin API และส่งงานเป็น vertical slices | ต้องมี snapshot/resync route, SSE cursor/gap recovery, admin auth/audit และ API/UI contract tests ในแต่ละ slice |

## Explicit assumptions

สมมติฐานต่อไปนี้ไม่ใช่ blocker และจะถูกตรวจระหว่าง targeted source inventory:

- `headless browser` หมายถึง Chromium/Chrome process และ pages ที่ collector สร้าง/ลงทะเบียนเองทั้งหมด; implementation ใช้ CDP หรือ automation interface ที่ stack เดิมรองรับ แต่ไม่แตะ browser/profile ของผู้ใช้
- Football และ Basketball มี feed target แยกกัน แต่ส่ง normalized envelope เดียวกันเข้าสู่ shared admission/state/odds pipeline
- หาก source ปฏิเสธ headless หรือ payload ไม่ผ่าน validation ระบบต้องแสดง `BROWSER_UNAVAILABLE` หรือ `SOURCE_CHANGED`, หยุด mutation จาก parser นั้น และเก็บเฉพาะ sanitized diagnostics; ห้ามแอบ fallback ไปใช้ browser ผู้ใช้
- Network response/WebSocket interception มาก่อน state-store subscription, DOM observer เป็น fallback และ polling เป็น watchdog เท่านั้น
- Read APIs เป็น same-origin/read-only ตาม deployment เดิม; Admin APIs reuse auth ของโปรเจ็กต์ถ้ามี มิฉะนั้นใช้ secret จาก environment พร้อม constant-time verification, authorization test และ audit log ห้าม hard-code secret
- SSE ใช้ durable event/cursor ID จาก outbox หรือ cloud event store; client ตรวจ gap แล้ว REST resync ก่อนกลับมารับ incremental events
- ค่า default ที่ requirement ให้เป็นตัวอย่างจะเป็น config: finished-card grace `3 นาที`, heartbeat `5 วินาที`, stale timeout `20 วินาที`, recovery grace `6 ชั่วโมง`, concurrency detail/asset/recovery `3/5/2` และ retry `10s/30s/2m/5m/15m` พร้อม jitter
- `UNKNOWN_TERMINAL` เป็น recovery outcome แยกจาก `internal_status = UNKNOWN` จนกว่า schema inventory จะแสดงว่ามี enum/column ที่เหมาะกว่า
- Finalization/force action มี `finalization_version`, lock/lease และ audit record; Final Refresh สำเร็จได้หนึ่งครั้งต่อ Match ต่อ version
- Source image URLs, cookies, tokens และ auth data ต้องถูกลบก่อน persistence ทุกชนิด รวมถึง JSON, jobs, outbox, logs และ diagnostics ไม่ใช่เฉพาะตารางหลัก
- Visual baselines สร้างจาก component ของ EarnScore และ deterministic fixtures; ใช้ต้นฉบับเป็น reference ด้าน hierarchy/interaction เท่านั้น ไม่ commit screenshot/code/branding ของต้นฉบับ
- Exact paths ยังไม่ระบุเพราะ runtime context อยู่ Level 0 และ search allowlist ไม่อนุญาต source inventory ใน grill รอบนี้; ก่อนสร้าง task files ต้องมี targeted inventory และ exact Allowed/Required Files
- การเลือก full rewrite **ยังไม่อนุญาต** ให้ลบ legacy DB/IDs/routes จนกว่าจะตอบ Round 002 และ revised plan รอบถัดไปล็อก compatibility boundary

## Scope

### In scope

- Standalone headless browser lifecycle, system-owned Football/Basketball feed pages, heartbeat, reconnect, wrong-page/sport detection และ watchdog
- Sport-specific source extraction/normalization สำหรับ network, store และ DOM fallback โดยมี shared event envelope
- Live-only canonical admission, internal state machine, stale-event rejection, event append log, score/state history และ current/history odds
- SQLite WAL, transaction boundaries, unique event/job keys, lease/reclaim, config และ transactional sync outbox
- Initial detail per section, missing-only retry, H2H external references, final refresh, recovery grace และ admin manual phases
- Ephemeral asset URL pipeline, image validation, SHA-256 dedup, atomic local publish, R2 upload และ internal asset routes
- Cloudflare D1/R2 sync, versioned REST, SSE, admin authorization/audit, feed status และ required metrics
- Live, History และ Match Detail dashboard สำหรับสองกีฬา รวม incomplete/empty/finished/stale states และ responsive visual regression
- Additive or replacement migration/import/rollback ตามคำตอบ Round 002 พร้อม Definition-of-Done evidence

### Out of scope

- Schedule/upcoming crawler, finished-page backfill หรือ canonical Match ที่ระบบไม่เคยพบ Live
- Canonical rows สำหรับผลย้อนหลังใน H2H; เก็บได้เฉพาะ non-canonical `match_h2h` references/aggregates
- Chat, comments, messages, ads, tracking, betting transactions, prediction และ unrelated notification/account features
- การ copy source code/CSS/HTML, iframe/proxy HTML, hotlink, unauthorized branding หรือ bypass source access controls
- การเปลี่ยน unrelated account/billing/business/deployment modules นอกจาก integration point ที่จำเป็นและระบุใน task
- Automatic headed/user-browser fallback เมื่อ headless ใช้งานไม่ได้
- Legacy cleanup/destructive deletion ก่อนคำตอบ Round 002, verified backup และ rollback gate

## Design and data boundaries

1. **Canonical identity:** `match_id + sport_id` ต้อง map ได้แน่นอน; adapter/reconnect ห้ามสร้าง duplicate identity
2. **Admission first:** ห้าม persist canonical match, enqueue detail หรือ create asset ownership ก่อน source evidence ผ่าน live/started gate
3. **Event idempotency:** event key ใช้ `match_id + event_type + source_timestamp + payload_hash`; เมื่อไม่มี timestamp ใช้ normalized payload hash
4. **Monotonic snapshot:** event เก่าบันทึกใน log ได้ถ้า unique แต่ไม่มีสิทธิ์ถอย snapshot/current odds; tie ใช้ deterministic comparator
5. **Atomic mutation:** snapshot/history/current odds/job intent/outbox ที่เกิดจาก event เดียวกันอยู่ใน SQLite transaction เดียว
6. **Section isolation:** detail uniqueness เป็น `match_id + section_name + load_phase`; score/odds handlers ไม่มีสิทธิ์ trigger completed detail
7. **Phase exclusion:** Match เดียวกันมี Initial และ Final worker พร้อมกันไม่ได้; expired lease reclaim ได้หลัง process kill
8. **H2H separation:** external match reference ไม่มี canonical foreign-key behavior ที่ทำให้ feed/recovery/API history มองเป็น collected Match
9. **Asset secrecy:** source URL อยู่เฉพาะ memory ของ active download attempt; retry หลัง process restart ต้อง reacquire URL จาก source section ไม่อ่าน persisted URL
10. **Final immutability:** Finalized data immutable ยกเว้น audited admin action ที่เพิ่ม version ใหม่
11. **At-least-once sync:** outbox ส่งซ้ำได้ แต่ D1/R2 consumers idempotent; partial upload มี reconciliation state แยก
12. **Fail closed:** parser confidence/schema mismatch หยุด state mutation และไม่เปลี่ยน Match เป็น terminal เพียงเพราะ extract ไม่สำเร็จ

## Affected files/modules

ยังเป็น module map จนกว่าจะผ่าน source inventory; revised plan ถัดไปต้องแทนด้วย exact paths:

| Module boundary | Required responsibility | Change boundary |
|---|---|---|
| Schema and migration/import | canonical/live fields, histories/current odds, sections/jobs/recovery/assets/outbox, H2H refs, backup/version markers | full replacement allowed only within Round 002 data boundary |
| SQLite repositories/unit of work | WAL, atomic event mutations, monotonic compare, uniqueness, leases/finalization locks | no browser/UI concerns |
| Shared feed domain | normalized envelope, admission/state/odds processors, sanitizer, source-change guard | source-neutral core only |
| Headless feed runtime | process/page ownership, one feed target per sport, network/store/DOM capture, heartbeat/reconnect/watchdog | no user browser/profile access |
| Football adapter | status/period/score/odds/source identity mapping and fixtures | Football-specific parsing only |
| Basketball adapter | status/period/score/odds/source identity mapping and fixtures | Basketball-specific parsing only |
| Detail collector | section planner, per-section extraction, retries, H2H references and content hashes | no score/odds-triggered reload |
| Asset worker/storage | ephemeral URL, validation/hash/dedup/atomic files/R2 state | no persisted source URL |
| Recovery/finalizer | pending selection, still-live resumption, missing grace, terminal reconciliation, one final refresh/version | no global old-match crawl |
| Cloud sync/Worker | outbox consumer, D1 upsert, R2 serving, replay/reconciliation | idempotent consumers only |
| REST/SSE/admin | versioned read/detail/assets/status routes, cursor/resync, auth/audit/admin jobs | compatibility alias pending Round 002 |
| Dashboard | Live/History/detail tabs, partial patches, two-sport states, internal assets, responsive/visual tests | project-owned UI only |
| Fixtures/tests/ops/docs | deterministic sequences, live smoke where allowed, restart/migration/security/visual evidence and runbooks | sanitized, no source secrets/branding |

## Forbidden files/modules

### During this planner grill

- `.ai-agent/ai-plan/tasks/task-*.md` — ห้าม create/edit/delete
- `.ai-agent/ai-plan/overview.md` และ `.ai-agent/ai-plan/context.md` — ห้าม finalize/edit เป็น approved plan
- Implementation code, schema, migration, tests, packages, deployment files และ generated build artifacts
- Runtime logs/token/jsonl/cache/tmp/history และ framework/generated state ที่ไม่ได้ระบุเป็น required output

### During implementation unless an exact task allows it

- Unrelated account, billing, unrelated admin/business pages และ unrelated deployment infrastructure
- User Chrome profile, user tabs, browser extensions และ arbitrary processes ที่ collector ไม่ได้สร้าง
- Legacy data/database backups, public IDs และ routes ตาม boundary ที่ยังรอ Round 002
- Vendor/dependency directories, build outputs, browser profiles, local DB backups และ raw captured source samplesใน version control
- Source-site code/HTML/CSS/screenshots/logos/branding, iframe/proxy/hotlink และ unsanitized source payloads
- Any persistence path capable of retaining source image URL, cookie, token หรือ authentication header
- Any task changing more than its named module boundary; cross-cutting changes require an explicit dependency task or revised Allowed Files

## Risks

| Risk | Impact | Required mitigation / stop condition |
|---|---|---|
| Full rewrite answer conflicts with migration/ID retention and additive API answer | irreversible data/route regression | Round 002 must lock preserved data/IDs/routes before any schema task exists |
| Headless source behavior differs from normal Chrome | no live events or misleading empty feed | validate network/store/DOM contract; mark unavailable/source-changed and stop writes; fixture suite remains deterministic |
| Football/Basketball lifecycle differs | wrong period/terminal/admission mapping | independent adapter fixtures + shared invariant suite; one sport failure cannot corrupt the other |
| Source events missing timestamps/out of order | duplicate or regressed snapshots | normalized hashes, monotonic comparator and stale-event tests |
| Detail/asset URL cannot survive restart by policy | retry could be impossible without reacquisition | persist section intent only; reacquire URL in new page/session; test kill between extraction/download |
| Initial/final/manual jobs race | duplicate detail or premature immutable final state | DB unique keys, per-match phase lock, version and lease recovery tests |
| H2H references leak into canonical flows | creates prohibited finished matches | separate table/type/repository and negative feed/recovery/API tests |
| Outbox/D1/R2 partial failure | cloud snapshot and asset mismatch | idempotent keys, per-record sync state, replay/dead-letter/reconciliation metrics |
| SSE gap or reconnect | stale cards | cursor, heartbeat, gap detection, REST resync and UI stale badge |
| Raw JSON/log stores source URL or secret | acceptance/security breach | sanitize before persistence, recursive scans in tests and redacted diagnostics |
| UI scope hides incomplete/error states | misleading dashboard | deterministic fixtures for live/finished/incomplete/no-lineup/stale/source-changed |
| Eight-task cap still creates oversized rounds | incomplete Coder round and scope leaks | each task below has one primary deliverable, embeds only local tests, and forbids unrelated modules; split again before task creation if source inventory exceeds boundary |

## Acceptance criteria

### Feed and admission

- Standalone headless process owns and recovers Football/Basketball feed pages without touching user tabs
- Both adapters emit required normalized events and heartbeat at least every 5 seconds
- Scheduled/upcoming and never-seen-live finished/terminal rows do not create canonical Match or detail jobs
- Score, clock, period, status and odds update from events without full-page reload; duplicate/stale events do not duplicate history or regress snapshot
- Wrong page/sport, lost listener and source mismatch produce explicit feed status and fail closed

### Detail, H2H, assets, and recovery

- First Live discovery creates only missing section jobs; completed sections do not reload on score/odds/reconnect/restart
- H2H historical results remain non-canonical and never enter live/history/recovery as collected matches
- Initial/Final jobs are mutually exclusive; retry targets only failed/missing sections and expired leases reclaim after kill
- Still-live recovery resumes current score/odds and missing sections only; finished-offline recovery performs exactly one final refresh per version
- Asset files are validated, hash-deduplicated and atomically published locally/R2; dashboard/database/logs/jobs contain no source image URL

### Persistence, migration, and cloud

- SQLite uses WAL and atomic event/history/current/outbox transactions; restart at detail/asset/finalization boundaries is idempotent
- Migration/import reruns safely, starts from verified backup and satisfies the Round 002 legacy/ID boundary
- Outbox replay and D1/R2 partial failures converge without duplicate domain history
- Required metrics/status fields reveal stale feed, event delay, jobs/failures and last heartbeat/odds event

### API, SSE, authorization, and Dashboard

- Required live/detail/assets/feed/admin behavior is exposed through versioned REST; legacy aliases/removal follow Round 002
- SSE updates only affected cards/sections, supports heartbeat/cursor/gap detection and REST resync; polling is fallback only
- Admin mutations require authorization and leave audit evidence; read access follows deployment policy
- Live page shows both sports, live score/period/main odds/feed freshness, final result for configured grace then History
- `/matches/{match_id}` shows header, Overview, Current Odds vs Movement, H2H, Lineups, Stats and Timeline with incomplete/no-lineup states
- Desktop/tablet/mobile and required match states pass Playwright visual checks; browser requests contain no source-domain asset calls

### Delivery evidence

- Unit/integration/browser/restart/migration/API auth/realtime/visual suites map to all 24 required automated scenarios
- Architecture and ER diagrams, migration/rollback, changed-file list, sample feed/odds/recovery records, desktop/mobile screenshots, no-URL/no-repeat evidence and install/Chrome troubleshooting guides are present
- At least a permitted live smoke test or a fixture sequence covering discovery, score/odds changes, disconnect, still-live recovery, offline finish and finalization passes

## Pre-task planning gate

ก่อนสร้าง task files Planner ต้อง:

1. รับคำตอบ Round 002 และล็อก data/ID/API compatibility boundary
2. ทำ targeted inventory เฉพาะ schema, runtime entrypoints, APIs, dashboard และ tests ภายใต้ search allowlist/context escalation
3. ระบุ exact Allowed/Required/Forbidden Files, validation commands และ dependency order ต่อ task
4. ตรวจว่าทุก task ด้านล่างแก้ primary module เดียวและ finish ได้หนึ่ง Coder round; ถ้าไม่ผ่านให้ลด scope ก่อนเขียน task file

## Proposed task breakdown — REVISED DRAFT ONLY

> สูงสุด 8 tasks ตาม runtime policy; แต่ละ taskมี primary deliverable เดียวและรวมเฉพาะ tests ของ boundary นั้น ห้าม Coder เริ่มจนกว่า blocker จะปิด

### T1 — Persistence schema and legacy migration/import

- Primary deliverable: SQLite/D1-compatible data contract, repeat-safe migration/import, backup/preflight/rollback และ repositories ขั้นพื้นฐาน
- Includes: canonical tables/fields, section/job/recovery/asset/outbox/H2H-ref schema, unique/FK/index/lease/version markers และ legacy mapping ตาม Round 002
- Excludes: browser, source parsing, workers, APIs และ UI
- Acceptance: rerun/restore/ID boundary/count/hash/no-new-non-live/no-source-URL migration tests ผ่าน

### T2 — Event, match-state, odds, and transactional outbox core

- Primary deliverable: source-neutral normalized event processor ที่ admission, deduplicate และเขียน state/odds/history/outbox แบบ atomic/monotonic
- Includes: internal state transitions, event keys, stale comparator, bookmaker/market identities และ failure-safe transactions
- Excludes: browser extraction, detail/recovery workers, cloud delivery และ UI
- Acceptance: scheduled/never-live-finished rejection, duplicate/stale score/odds, current/history และ rollback tests ผ่านทั้งสอง sport fixtures

### T3 — Headless feed runtime with Football and Basketball adapters

- Primary deliverable: system-owned standalone headless process/pages ที่ส่ง normalized events ของสองกีฬาเข้าสู่ T2
- Includes: network/store/DOM priority, heartbeat, reconnect, watchdog, ownership, wrong page/sport และ fail-closed source diagnostics
- Excludes: schema mutation logicนอก T2, detail pages, asset downloads, APIs และ UI
- Acceptance: adapter contract fixtures, process/page restart, no user-browser interaction และ source-mismatch tests ผ่าน

### T4 — Section-based initial detail collector

- Primary deliverable: Initial detail planner/worker สำหรับ Overview, Odds, H2H, Lineups, Stats, Incidents และ related entities โดยโหลดเฉพาะ section ที่ขาด
- Includes: section states/content hashes, unique phase keys, retry/jitter/lease reclaim และ non-canonical H2H references
- Excludes: binary asset download, final recovery, cloud/API และ UI
- Acceptance: first discovery, completed/no-repeat, missing-only retry, empty/permanent failure และ kill/reclaim tests ผ่าน

### T5 — Ephemeral asset pipeline and storage

- Primary deliverable: image download/validation/dedup/atomic local storage/R2-ready metadata โดย source URL ไม่ออกจาก active memory
- Includes: reacquisition contract from T4, MIME/image verification, SHA-256 reuse, asset links, upload state และ internal asset serving boundary
- Excludes: detail section parsing, recovery decisions, general APIs และ Dashboard components
- Acceptance: duplicate assets reuse one file, restart reacquires URL, corrupt/non-image rejection และ recursive database/log/job no-source-URL scans ผ่าน

### T6 — Feed recovery and one-version finalization

- Primary deliverable: reconnect/startup recovery สำหรับ previously-live unresolved Matches และ resumable final reconciliation
- Includes: pending capture, still-live resume, missing-match grace, terminal states, phase exclusion, final content hashes, admin retry/finalize versioning
- Excludes: global history crawl, completed H2H refresh, cloud transport และ UI
- Acceptance: still-live/no-repeat, offline finish/one final refresh, not-found grace, cancel/postpone/abandon, concurrent jobs และ kill/restart tests ผ่าน

### T7 — Cloud sync, versioned REST, SSE, and admin security

- Primary deliverable: idempotent SQLite outbox to D1/R2 delivery surface พร้อม required read/admin APIs และ realtime stream
- Includes: replay/reconciliation, cursor/gap/resync, feed metrics/status, authorization/audit และ compatibility behavior ตาม Round 002
- Excludes: visual Dashboard implementation
- Acceptance: outbox replay/partial failure convergence, API contract/auth, SSE reconnect/gap, asset route และ rate/error tests ผ่าน

### T8 — Live/History/Match Detail Dashboard and delivery evidence

- Primary deliverable: project-owned responsive UI สำหรับ two-sport Live/History/detail ที่ใช้ T7 contracts และ internal assets เท่านั้น
- Includes: partial card/section updates, finished grace, stale/error/incomplete/no-lineup states, current vs movement odds, Playwright visuals/accessibility และ final evidence/runbook assembly
- Excludes: source-site code/assets, schema/parser changes และ backend contract expansionนอก T7
- Acceptance: desktop/tablet/mobile + required state baselines, no full reload/source-domain request, E2E discovery-to-finalization fixtures และ Definition-of-Done evidence index ผ่าน

## Remaining blocker

Round 002 เหลือคำถามเดียว: full rewrite จะรักษา legacy data, Match IDs และ routes ถึงระดับใด เพื่อไม่ให้ Q3 ขัดกับ requirement และ Q5
