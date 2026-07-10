## Status
Pending

# Task 07: Dedicated Chrome Targets, Readiness Delays, and Parallel Feed Capture

## Outcome

แต่ละ sport session สร้าง dedicated Live tab ของตนเอง, detail feeds ใช้ crawler-owned new tabs แบบ bounded parallelism, ทุก navigation รอ minimum delay + verified readiness/stability และข้อมูลจาก tab/match อื่นไม่ถูกบันทึกปะปน

## Dependencies

- Task 05 source contract/adapters and fixtures
- Task 06 DB identity guard available for persistence boundary

## Implementation Scope

### Required Files

- `src/main.rs` — CLI readiness/concurrency options, Chrome browser/target lifecycle, list/detail coordinators, wait predicates, cancellation/cleanup and tests
- `tests/fixtures/chrome-targets.json` — new target-list fixture with owned/unowned/football/basketball/detail cases
- `tests/fixtures/source-filter-states.json` — readiness state transitions from Task 05
- `tests/fixtures/football-detail.json` — match-specific readiness fixture
- `tests/fixtures/basketball-detail.json` — match-specific readiness fixture

### Allowed Files

- `Cargo.toml` — only if current Tokio/WebSocket/HTTP crates cannot implement target lifecycle or deterministic tests

### Out of Scope

- D1 dataset activation/API filtering
- dashboard UI
- schema changes beyond Task 06

## Implementation Steps

1. เพิ่ม CLI options พร้อม safe defaults/ranges เช่น list minimum settle delay, detail minimum settle delay, readiness timeout, probe interval และ detail concurrency. ระบุหน่วยใน help/log และ reject zero/unbounded values ที่ทำให้ tight loop/tab explosion
2. ใช้ Chrome DevTools browser target APIs/HTTP endpoint ที่ตรวจจริงเพื่อ create dedicated list target ต่อ process. ติด owner/session/sport marker (เช่น window name + tracked target ID) และห้าม fallback ไป hijack arbitrary tab
3. แยก target registry/lifecycle จาก page command router: track target ID, websocket URL, role, sport, requested match, creation time และ owner. Cleanup เฉพาะ owned targets ใน normal exit, timeout, reconnect และ cancellation
4. หลัง navigate ให้รอ lifecycle events/document readiness, apply minimum delay, activate Live เสมอ แล้ว probe DOM active marker + adapter source. ต้องได้ stable ready snapshot สอง probesติดต่อกันก่อน queue persistence; state ที่ non-null แต่ filter ไม่ใช่ Live ต้องถูก reject/retry
5. Replace hidden iframe detail helper ด้วย crawler-owned detail targets หรือ target pool ตาม structure ที่ยืนยันใน Task 05. เปิดได้พร้อมกันไม่เกิน configured concurrency, แต่ละ target navigate URL ของ match เดียวและรอ match-specific hydrated ID/sections
6. Pair results ด้วย target/request/match IDs. ก่อน `save_match_detail` ตรวจ sport, requested match, hydrated match และ current dataset ID; stale response หลัง timeout/reconnect ต้อง discard
7. Detail failure ต้อง close/quarantine target, retry with bounded backoff และไม่ block list reconciliation. List target failure re-create เฉพาะ session target โดยไม่ปิด unowned tabs
8. Coalesce list change events โดยไม่ serialize detail pool ทั้งหมด; persistence transactions ยังสั้นและ DB identity guard ทำงานก่อน save
9. เพิ่ม deterministic tests สำหรับ target selection/ownership, readiness sequence (loading→All→Live unstable→Live stable), timeout, concurrency cap, wrong-match discard, cleanup on cancellation และ two sport sessions isolation

## Acceptance Criteria

- start football/basketball สร้าง dedicated list target คนละ target และไม่ navigate/close existing user tabs
- initial DB write เกิดหลัง correct Live marker และ stable snapshot เท่านั้น; non-null All state ไม่ผ่าน
- configured minimum delay ถูกใช้ แต่ timeout/success อาศัย readiness predicates ไม่ใช่ sleep อย่างเดียว
- detail targets ทำงานพร้อมกันได้ถึง cap และ never exceed cap; result ทุกตัวตรง requested sport/match/dataset
- timeout/reconnect/CTRL-C ไม่ทิ้ง owned detail tabs และไม่ปิด unowned tabs
- list reconciliation ดำเนินต่อระหว่าง detail fetch; match หนึ่งล้มเหลวไม่ block match อื่น
- source/DB/chat/sync regression tests เดิมยังผ่าน

## Validation

```bash
cargo fmt -- --check
cargo test target_ownership_
cargo test readiness_
cargo test detail_concurrency_
cargo test wrong_match_
cargo test crawler_
```

```bash
cargo run -- --chrome-url http://127.0.0.1:9223 --db-path /tmp/earnscore-tabs.db --page-ready-delay-ms 1500 --detail-ready-delay-ms 2000 --detail-concurrency 3 football
cargo run -- --chrome-url http://127.0.0.1:9223 --db-path /tmp/earnscore-tabs.db --page-ready-delay-ms 1500 --detail-ready-delay-ms 2000 --detail-concurrency 3 basketball
```

Chrome DevTools manual validation ต้องบันทึกก่อน/ระหว่าง/หลัง target inventory:

- tabs ที่มีอยู่ก่อน start คง URL และยังเปิดอยู่
- มี owned list target หนึ่งตัวต่อ session
- detail targets พร้อมกันไม่เกิน 3 และถูกปิด/reuse หลัง save
- force slow network/reload แล้วไม่มี row ถูก save ก่อน Live/detail readiness
- เปิด wrong detail/state จำลองแล้ว DB ไม่มี detail ผิด match

## Reference Map

### Generated Knowledge/Cache

- `.ai-agent/generated/knowledge/api.md` — current `get_websocket_url`, Runtime.evaluate and router call sites
- `.ai-agent/generated/knowledge/architecture.md` — async/task structure and `main` entrypoint
- `.ai-agent/generated/cache/codegraph-project.md` — target/router/event/detail relationships
- `.ai-agent/generated/cache/symbol-index.md` — exact `WsRouter`, `send_command`, `main` and tests

### Exact Source Files

- `src/main.rs` — `Cli`, target selection, `WsRouter`, injected helper, event/reconciliation loop and tests
- `Cargo.toml` — Tokio/WebSocket/HTTP dependencies
- `tests/fixtures/chrome-targets.json` — task-produced target inventory fixture
- `tests/fixtures/source-filter-states.json`
- `tests/fixtures/football-detail.json`
- `tests/fixtures/basketball-detail.json`

### External Runtime References

- Chrome remote debugging target list/browser websocket exposed by configured `--chrome-url`
- `https://m.aiscore.com/`
- `https://m.aiscore.com/basketball`
