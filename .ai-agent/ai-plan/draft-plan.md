# Draft Implementation Plan — Event-Driven Live Sports Feed

> **สถานะ: DRAFT ONLY — NOT APPROVED — NON-EXECUTABLE BY CODER**
>
> เอกสารนี้มีไว้สำหรับ Planner Grill รอบแรกเท่านั้น ห้ามใช้เป็น task implementation และยังห้ามสร้างไฟล์ `.ai-agent/ai-plan/tasks/task-*.md` จนกว่าจะตอบ blocker questions และจัดทำ revised plan

## Goal

ปรับ EarnScore ให้รับและประมวลผลการแข่งขันฟุตบอลและบาสเก็ตบอลแบบ event-driven โดยสร้าง canonical Match เฉพาะคู่ที่ระบบพบหลักฐานว่าเริ่มแข่งขันแล้ว เก็บ score/state/odds แบบ idempotent ดึงรายละเอียดและ asset แบบ section-aware โดยไม่โหลดซ้ำ ฟื้นตัวหลัง feed หยุดได้อย่างถูกต้อง sync จาก SQLite ไป Cloudflare D1/R2 ผ่าน transactional outbox และให้ Dashboard/API แสดง Live/History/Match Detail แบบ real-time โดยไม่ hotlink หรือคัดลอก source UI

ผลลัพธ์ต้องรักษาข้อมูลเดิมและ Match ID เดิมเท่าที่ทำได้ มี migration/rollback ที่ปลอดภัย และมีหลักฐานทดสอบ invariants สำคัญ ไม่ถือว่าการ compile ผ่านเพียงอย่างเดียวเป็นการส่งมอบ

## Planning status and context limits

- งานถูกจัดเป็น `LARGE` และ `database`; runtime context อยู่ที่ Level 0 (Minimal Context)
- รอบนี้อ่านเฉพาะ requirement และ compact runtime context ตาม allowlist จึงยังไม่ได้สำรวจ source tree, schema เดิม, package/runtime, routes หรือ test harness
- รายชื่อไฟล์ implementation ด้านล่างจึงระบุเป็น **candidate modules** ไม่ใช่ path ที่อนุมัติแล้ว ต้องทำ targeted source/schema inventory หลัง grill จึงจะแปลงเป็น exact file paths และ Allowed/Required Files
- Requirement อนุญาตให้ rewrite ได้ แต่ไม่ได้อนุมัติ destructive migration, route breakage หรือการลบข้อมูลเดิมโดยปริยาย

## Known constraints

### Data admission and state

- สร้าง canonical Match ได้เฉพาะ source บอกว่า Live หรือมีหลักฐานว่าเริ่มแล้ว เช่น clock, score หรือ current period
- ห้ามสร้าง canonical Match ใหม่จาก Scheduled/Upcoming/Finished/Cancelled/Postponed ถ้าระบบไม่เคยพบ Live
- Match ที่เคย Live เท่านั้นจึงเปลี่ยนไปเป็น terminal state และเข้าสู่ recovery/finalization ได้
- ต้องมี internal state machine แยกจาก source status และห้าม event เก่า overwrite snapshot ใหม่
- Finalized Match ต้อง immutable ยกเว้น explicit admin force action ที่มี audit trail

### Event, odds, and transaction integrity

- Feed event เป็น append-only และ deduplicate ด้วย deterministic event key; source ที่ไม่มี timestamp ต้องใช้ normalized payload hash
- Score/state/odds current/history และ outbox event ที่เกี่ยวข้องต้อง commit ใน transaction เดียวกัน
- Odds ต้องแยก current กับ history รองรับหลาย bookmaker/market/period/selection โดยไม่ hard-code
- SQLite ต้องใช้ WAL, job lease/reclaim และ idempotent workers เพื่อรองรับ process ถูก kill/restart กลางงาน

### Detail, recovery, and assets

- Detail ทำงานระดับ section และ unique ตาม `match_id + section_name + load_phase`; completed section ห้ามโหลดซ้ำเพราะ score/odds/reconnect/restart
- Initial กับ Final detail ของ Match เดียวกันห้ามทำพร้อมกัน; final refresh ได้สูงสุดหนึ่งครั้งต่อ finalization version
- Recovery ตรวจเฉพาะ Match ที่เคย Live, ยังไม่ Finalized และ feed หยุดก่อนทราบผลสุดท้าย
- Source image URL ต้องมีเฉพาะใน memory ของ download attempt; ห้าม persist ในคอลัมน์หลัก, JSON payload, job payload, logs, diagnostics หรือ outbox
- Dashboard ใช้เฉพาะ local/R2 asset route; asset deduplicate ด้วย content hash และ publish แบบ atomic

### Browser, API, UI, and operations

- Feed ต้องใช้ event interception/state subscription ก่อน DOM fallback; polling เป็น watchdog เท่านั้นและห้าม full-page reload loop
- ระบบต้องเป็นเจ้าของ target/tab ที่สร้างเอง ไม่ focus/ปิด/เปลี่ยน tab ของผู้ใช้
- API ต้องมี live/detail/assets/feed/admin endpoints ตาม requirement; admin endpoints ต้องมี authorization และ mutation audit
- Dashboard ต้อง patch เฉพาะ card/section ที่เปลี่ยนผ่าน SSE หรือ WebSocket โดยมี reconnect/cursor/resync และ polling fallback
- UI เลียนแบบได้เฉพาะ information hierarchy/interaction; ห้าม copy code, iframe, proxy HTML, hotlink หรือใช้เครื่องหมายการค้าโดยไม่ได้รับอนุญาต
- ต้องมี metrics, sanitized diagnostics และ fail-closed `SOURCE_CHANGED`; extraction failure ห้ามตีความเป็น Finished

## Scope boundaries

### In scope

- Source adapters สำหรับ Football และ Basketball ตามขอบเขตที่ได้รับคำตอบใน Q1
- Feed session/heartbeat/target ownership/reconnect/watchdog และ source-change detection
- Canonical event normalization, match admission/state machine, odds processing และ stale-event protection
- SQLite schema/repositories/migrations สำหรับ event log, current/history, section jobs, recovery, assets และ outbox
- Section-based initial detail, selective retry, final reconciliation และ manual admin actions
- Asset validation/dedup/local storage/R2 upload โดยไม่มี persisted source URL
- Outbox sync ไป D1/R2, idempotent Worker API, real-time stream และ admin authorization
- Live/History/Match Detail dashboard, responsive states และ visual regression fixtures
- Unit, integration, browser/fixture, migration, restart/recovery, security และ visual tests
- Architecture/ER diagrams, runbooks, rollback และ Definition-of-Done evidence

### Out of scope unless explicitly approved

- การ scrape หรือสร้าง canonical records สำหรับ schedule/upcoming/finished matches ที่ไม่เคย Live
- Chat, comments, messages, ads, tracking และ user-generated content
- การทำ sportsbook/betting transactions, prediction, notification หรือ account features ที่ requirement ไม่ได้ขอ
- การคัดลอก proprietary source code/assets/branding หรือ bypass authentication/anti-bot controls
- การ redesign ระบบ unrelated เช่น account, billing, unrelated admin pages หรือ deployment infrastructure ที่ไม่จำเป็นต่อ feed path
- การ backfill ประวัติการแข่งขันทั้งระบบจาก finished pages
- destructive cleanup ของ legacy data หรือ removal ของ legacy routes ก่อน compatibility/cutover decision

## Affected files/modules (candidate; exact paths pending source inventory)

| Area | Candidate responsibility | Expected change type |
|---|---|---|
| Database schema/migrations | match admission fields, state history, feed events/sessions, odds current/history, section/job/recovery tables, assets/links, outbox | additive and repeat-safe migration first |
| Local data access/unit of work | monotonic writes, transactions, event-key uniqueness, job leases, finalization locks | extend or replace behind stable interfaces |
| Browser/feed runtime | Chrome ownership registry, Football/Basketball adapters, network/store/DOM capture, heartbeat/reconnect | new isolated adapters and session manager |
| Domain processors | event normalization, state machine, score/odds processors, source-change guard | new shared core with sport-specific normalization |
| Detail/recovery workers | section planner, retry/backoff, initial/final mutual exclusion, grace-period reconciliation | section-aware durable jobs |
| Asset pipeline/storage | ephemeral URL handling, validation, SHA-256 dedup, atomic local publish, R2 upload | new constrained pipeline |
| Sync/Cloudflare Worker | transactional outbox consumer, D1 upsert/event stream, R2 asset serving | idempotent sync and versioned contracts |
| REST/admin/realtime API | required routes, cursor/resync, auth/audit, error contracts | additive/versioned until compatibility is known |
| Dashboard | live list/history/detail tabs, partial updates, incomplete/empty states, local assets | component-level changes with responsive tests |
| Tests/fixtures/operations/docs | event sequences, source fixtures, migration/rollback, restart tests, visual baselines, metrics/runbooks | new or extended verification assets |

## Forbidden files/modules

### Forbidden during this grill round

- `.ai-agent/ai-plan/tasks/task-*.md` — ห้าม create/edit/delete
- `.ai-agent/ai-plan/overview.md` และ `.ai-agent/ai-plan/context.md` — ห้าม finalize หรือแก้ให้เป็นแผนอนุมัติ
- Source code, schema, migration, tests, package/deployment files และ generated artifacts ทั้งหมด — รอบนี้ห้าม implement
- Agent/runtime logs, token/jsonl/cache/tmp/history — runtime policy ไม่อนุญาตให้อ่านหรือแก้
- Framework state นอกสาม output ที่ผู้ใช้กำหนด เช่น root `AGENTS.md`, `.agent/loop-verdict.txt`, `.ai-agent/generated/**`

### Proposed implementation exclusions pending Q3

- Unrelated account/auth/billing/business modules ต้องเป็น forbidden-by-default เว้นแต่จำเป็นต่อ admin authorization และระบุ exact paths ใน revised plan
- Existing public routes, Match IDs และ legacy data ห้ามลบหรือเปลี่ยน semantics จนกว่าจะอนุมัติ compatibility/cutover strategy
- Build output, dependency vendor directories, browser profiles, database backups และ captured source samples ห้าม commit เข้า source control
- Source-site HTML/CSS/JS, remote logos/branding และ unsanitized diagnostics ห้ามนำเข้า production modules หรือ test baselines

## Design invariants to lock before implementation

1. `match_id` เป็น canonical identity เดียวข้าม feed, detail, recovery, outbox และ cloud; ห้ามสร้าง duplicate เพราะ reconnect หรือ sport adapter ต่างกัน
2. Admission gate ทำงานก่อน persistence ของ canonical match และก่อน enqueue detail/asset jobs
3. Persisted event/job/diagnostic JSON ต้องผ่าน sanitizer ที่ลบ source image URLs, cookies, tokens และ authentication data ก่อน transaction
4. Snapshot writes ต้องมี ordering rule จาก source timestamp, received timestamp และ deterministic tie-breaker; event log เก็บ duplicate-free history แยกจาก snapshot
5. Detail completeness เป็นราย section และ phase; score/odds processors ไม่มีสิทธิ์ enqueue detail โดยตรง
6. Finalization เป็น resumable transaction/workflow มี version/lock และ completion marker; failure กลางทางกลับมาทำต่อโดยไม่สร้าง final history ซ้ำ
7. Outbox delivery เป็น at-least-once แต่ D1/R2 consumers ต้อง idempotent; dashboard stream ต้องรองรับ cursor gap และ full resync
8. `SOURCE_CHANGED` เป็น fail-closed: หยุด mutation จาก parser ที่ไม่เชื่อถือ แต่รักษา heartbeat/diagnostics/recovery state โดยไม่ fabricate terminal result

## Proposed architecture (pending grill decisions)

```text
System-owned Chrome target(s)
  -> sport source adapter
  -> normalized feed envelope + sanitizer
  -> admission gate / monotonic state machine / odds processor
  -> SQLite transaction
       - feed_events
       - matches + histories/current odds
       - section/recovery/asset jobs
       - sync_outbox
  -> leased workers
       - detail section collector
       - recovery/finalizer
       - ephemeral asset downloader
       - outbox publisher
  -> Cloudflare D1 + R2
  -> versioned REST + realtime stream
  -> Live / History / Match Detail dashboard
```

Sport adapters ควรรับผิดชอบเฉพาะ source-specific extraction/status mapping ส่วน admission, state machine, idempotency, jobs และ persistence ใช้ shared domain core เพื่อป้องกันกฎของ Football/Basketball แตกต่างกันโดยไม่ตั้งใจ

## Migration and rollout outline

แผนเบื้องต้นเสนอ additive, repeat-safe migration และยังไม่อนุมัติจนตอบ Q3:

1. Inventory schema/data volume/index/foreign keys/route consumers และทำ preflight report แบบ read-only
2. สร้าง verified backup และทดสอบ restore ก่อน migration จริง
3. เพิ่มตาราง/คอลัมน์/index ใหม่โดยไม่ drop/rename ของเดิม; migration ทุกขั้นมี version marker และ rerun guard
4. แปลง legacy `match_details` เป็น section rows ด้วย deterministic mapping และรักษา Match ID
5. แยก odds จาก legacy raw payload โดยบันทึก provenance/parse failures; ห้ามแต่งข้อมูลที่แปลงไม่ได้
6. Backfill เฉพาะ existing canonical matches; ห้ามใช้ migration เป็นช่องทางสร้าง finished/scheduled matches ใหม่
7. เปิด dual-read/shadow verification ตาม compatibility strategy แล้วเปรียบเทียบ counts/hashes/invariants
8. Cut over เป็นขั้นพร้อม rollback procedure; legacy removal เป็นงานภายหลังและต้องได้รับอนุมัติแยก

## Risks and mitigations

| Risk | Consequence | Draft mitigation / stop condition |
|---|---|---|
| Football/Basketball payload และ lifecycle ต่างกัน | state/odds mapping ผิดหรือ schema hard-code | แยก adapter + shared contract fixtures; หยุด sport rollout หาก invariant suite ไม่ผ่าน |
| Source network/store/DOM เปลี่ยน | บันทึกข้อมูลผิดหรือ mark จบผิด | schema validation, confidence checks, fail-closed `SOURCE_CHANGED`, sanitized samples |
| Event ไม่มี timestamp/มาถึงผิดลำดับ | duplicate history หรือ snapshot ถอยหลัง | normalized hash + monotonic comparator + tie-break tests |
| Initial/Final jobs race กัน | detail ซ้ำหรือ finalize ไม่ครบ | DB uniqueness, lease, per-match phase lock และ restart tests |
| Migration legacy raw data ไม่สม่ำเสมอ | data loss/ID break/API regression | backup+restore drill, additive mapping, quarantine report, shadow verification |
| H2H มี finished matches ที่ไม่เคย Live | ละเมิด admission invariant | แยก non-canonical reference modelตาม Q4 |
| Persisted raw payload มี source image URL/secret | acceptance/security failure | sanitize before persistence, schema scan tests, log redaction |
| Outbox replay/partial Cloudflare failure | D1/R2 ซ้ำหรือไม่ตรงกัน | idempotency keys, per-object state, retry/dead-letter, reconciliation metrics |
| Realtime disconnect/cursor gap | Dashboard แสดง snapshot เก่า | event IDs, reconnect cursor, gap detection, REST resync, stale badge |
| Visual similarityกลายเป็น copied UI/branding | legal/maintainability risk | project-owned components/tokens/assets; baseline เฉพาะ approved reference states |
| งานใหญ่เกิน reviewable task | scope leak และตรวจ invariant ไม่ครบ | แตก workstream ไม่เกิน 8 task หลัง grill แต่ละ taskมี allowed files/tests/rollback |

## Open questions

Blockers ถูกเขียนใน `grill/round-001-questions.md` และสำเนา `grill/questions.md`:

- Q1: Football/Basketball delivery boundary
- Q2: Chrome runtime and tab ownership model
- Q3: legacy compatibility, migration/cutover, and forbidden-module boundary
- Q4: H2H historical rows versus live-only canonical Match invariant
- Q5: API/realtime/auth/UI release contract and task boundary

คำถามเพิ่มเติมที่ยังไม่ถือเป็น blocker รอบนี้และต้องยืนยันระหว่าง targeted inventory/revised planning:

- ค่า default ที่ requirement ยกตัวอย่างแต่ไม่ fix จะทำเป็น config: finished-card grace `3 นาที`, stale timeout `20 วินาที`, recovery grace `6 ชั่วโมง`, detail/asset/recovery concurrency `3/5/2`
- จะใช้ SSE เป็นค่าเริ่มต้นเพราะ flow หลักเป็น server-to-dashboard; เปลี่ยนเป็น WebSocket ได้หาก existing runtime หรือ Q5 บังคับ
- `UNKNOWN_TERMINAL` ปรากฏใน recovery requirement แต่ไม่อยู่ใน internal status list; draft เสนอให้เป็น terminal recovery outcome แยกจาก `internal_status = UNKNOWN` จนกว่าจะยืนยัน schema เดิม
- การ Force Refresh/Force Finalize ต้องเพิ่ม finalization version และ audit record ไม่แก้ finalized rows แบบเงียบ
- Visual baselines ต้องเป็น screenshots ของ UI ที่สร้างใหม่ด้วย fixture ที่กำหนดแน่นอน ไม่ใช้ภาพต้นฉบับเป็น committed golden file

ค่าเหล่านี้เป็น **provisional defaults** ไม่ใช่ข้อสรุป หากผู้ใช้ไม่ยอมรับให้ระบุ Custom ใน Q5 หรือยกเป็น blocker ในรอบถัดไป

## Proposed task breakdown — DRAFT ONLY

> ห้ามสร้าง task files จากรายการนี้ รายการจะถูกแก้หลังตอบ grill และหลัง targeted source inventory เท่านั้น

1. **T1 — Current-system inventory and contracts**
   - ระบุ exact stack/schema/routes/process owners/tests/deployment, data volumes และ compatibility consumers
   - สร้าง architecture/ER proposal, source event contract, fixture plan และ exact allowed/forbidden file map
   - Exit: ทุก task ถัดไปมี path boundary และ migration/cutover decision ที่อนุมัติแล้ว

2. **T2 — Additive persistence and migration foundation**
   - เพิ่ม repeat-safe schema, backup/preflight/restore, legacy detail/odds mapping, repositories, monotonic unit-of-work และ transactional outbox
   - Tests: rerun/rollback, preserve IDs, no new non-live canonical rows, no persisted remote image URLs
   - Exit: migration fixture และ legacy compatibility checks ผ่าน

3. **T3 — Browser sessions, sport adapters, and normalized events**
   - ทำ target ownership, heartbeat/reconnect/watchdog, Football/Basketball adapters, network/store/DOM priority และ `SOURCE_CHANGED`
   - Tests: ownership, reconnect, wrong page/sport, parser mismatch, event normalization/dedup
   - Exit: deterministic fixtures สร้าง event contract โดยไม่เขียน match ที่ไม่ผ่าน admission

4. **T4 — Match/odds processors and durable job orchestration**
   - ทำ admission gate, state machine, stale-event comparator, current/history odds, leases/backoff และ section job uniqueness
   - Tests: scheduled/finished rejection, score without detail reload, odds dedup/history, kill/reclaim
   - Exit: transaction/outbox invariants ผ่าน concurrency tests

5. **T5 — Detail, recovery, finalization, and asset pipelines**
   - ทำ initial/section retry, final reconciliation, not-found grace, phase locks, asset validation/dedup/atomic publish/R2 state
   - Tests: reconnect without reload, one final refresh/version, missing-section only, restart at each phase, zero persisted source URLs
   - Exit: recovery scenarios และ asset evidence ผ่าน fixture/integration suite

6. **T6 — Cloud sync, REST/admin APIs, and realtime delivery**
   - ทำ idempotent outbox consumer, D1/R2 mapping, versioned endpoints, auth/audit, stream cursor/resync และ monitoring
   - Tests: replay, partial failure, authorization, API compatibility, cursor gaps/stale status
   - Exit: local-to-cloud reconciliation และ API contract suite ผ่าน

7. **T7 — Live, History, and Match Detail dashboard**
   - ทำ Live cards, finished grace/removal, detail tabs/states, current vs movement odds, responsive layouts และ local assets only
   - Tests: component patching without full reload, missing detail/lineup, accessibility, desktop/tablet/mobile visual baselines
   - Exit: approved fixtures ครบทุก visual state และไม่มี source-domain requests

8. **T8 — End-to-end hardening, rollout, and handoff evidence**
   - รัน feed/fixture sequences, migration dry run, failure injection, performance/rate limits, security/data scans, backup/rollback drill
   - ส่ง architecture/ER, results, sample events/odds/recovery, screenshots, no-remote-URL/no-repeat evidence และ Chrome/feed runbooks
   - Exit: Acceptance Criteria และ Definition of Done มี evidence mapping ครบ; unresolved failures ห้าม waive แบบเงียบ

## Validation strategy

- **Static/schema:** migration lint, unique/FK/index checks, URL/secret scans across scalar and JSON columns
- **Unit:** admission matrix, source-to-internal transitions, event keys, stale ordering, odds identities, retry/finalization rules
- **Transactional integration:** event + snapshot/history + outbox atomicity, worker leases, concurrent initial/final claims, process-kill recovery
- **Browser fixtures:** network/store/DOM priority, reconnect, tab ownership, wrong page/sport, source mismatch and sanitized diagnostics
- **Recovery sequences:** still-live, finished while offline, not found then terminal, grace expiry, repeated restart and one final refresh/version
- **Cloud/API:** outbox replay, D1/R2 partial failure, endpoint contracts/auth, SSE/WebSocket cursor gap and REST resync
- **UI/E2E:** live score/odds patch without full reload, History/detail routes, incomplete states, no source-domain request, responsive/visual/accessibility checks
- **Migration:** production-like anonymized copy where available, backup/restore proof, repeat run, ID preservation, counts/hashes/quarantine report and rollback rehearsal

## Approval gate before task creation

Planner ต้องไม่สร้าง implementation task files จนกว่าจะ:

1. ผู้ใช้ตอบ Q1–Q5 หรือระบุ Custom ที่ชัดเจน
2. Planner จัดทำ revised plan โดยบันทึก decisions/assumptions และผู้ใช้ยอมรับ
3. Targeted source inventory ระบุ exact affected/forbidden paths ภายใต้ search budget/context escalation
4. Migration rollback, API compatibility, browser ownership และ live-only/H2H model ไม่มี blocker ค้าง
5. แต่ละ task มี goal, Allowed/Required Files, dependencies, acceptance criteria, validation commands และ rollback/stop conditions ที่ตรวจได้
