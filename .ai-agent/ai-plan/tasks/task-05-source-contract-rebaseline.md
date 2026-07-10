## Status
Pending

# Task 05: Re-baseline AiScore Source Relationships

## Outcome

Football/basketball adapters และ regression fixtures อ้างอิง runtime structure ที่ตรวจจริงจาก Chrome DevTools, แยก Live snapshot ออกจาก All/previous state และ map match → competition/team/status/scores/detail อย่างถูกต้องก่อนเปลี่ยน tab orchestration

## Dependencies

- Task 01–04 completed baseline

## Implementation Scope

### Required Files

- `src/main.rs` — `SportAdapter`, source extraction/normalization, Live-state validation และ source-contract tests เท่านั้น
- `tests/fixtures/football-live.json` — replace simplified fixture ด้วย sanitized actual Live list shape
- `tests/fixtures/football-finished.json` — sanitized actual finished transition/status shape
- `tests/fixtures/basketball-live.json` — sanitized actual Live list shape
- `tests/fixtures/basketball-finished.json` — sanitized actual finished transition/status shape
- `tests/fixtures/football-detail.json` — new sanitized hydrated football detail shape
- `tests/fixtures/basketball-detail.json` — new sanitized hydrated basketball detail shape
- `tests/fixtures/source-filter-states.json` — new minimal All-vs-Live/active-filter evidence สำหรับ readiness tests

### Allowed Files

- `Cargo.toml` เฉพาะหาก parser/test fixture support ต้องใช้ dependency ที่ของเดิมทำไม่ได้

### Out of Scope

- การสร้าง/ปิด Chrome tabs และ concurrency
- dataset generation/schema/D1
- dashboard/API behavior

## Implementation Steps

1. ใช้ Chrome DevTools MCP ตรวจสองหน้าต้นทางใน All และ Live states โดยบันทึกเฉพาะ:
   - network request URL/method/response envelope ที่มี matches/entities/details
   - Nuxt/Vue store paths ที่สัมพันธ์กับ response
   - DOM selector/attribute/route state ที่พิสูจน์ว่า Live active
   - football/basketball status, clock, score arrays และ detail match identity
2. สร้าง source relationship map ใน adapter code/tests: primary match collection, entity dictionaries/embedded objects, upstream IDs, sport identity, competition/team references และ detail identifiers. Network payload เป็น primary เมื่อพร้อม; store fallback ต้องมี explicit shape/version guard
3. แยก raw source decoding ออกจาก normalized persistence DTO. Decoder ต้องรองรับ dictionary/array envelope ที่พบจริง, reject cross-sport/empty/incomplete records และรายงาน structured extraction errors แทนคืน empty arrays เงียบๆ
4. เพิ่ม Live snapshot metadata/predicate: correct sport, active Live filter, source timestamp/version (ถ้ามี), match/entity counts และ relation completeness. Snapshot จาก All หรือ stale previous filter ต้องไม่ผ่าน persistence boundary
5. ยืนยัน sport-specific status/score mappings จาก fixtures; finished fixture ต้องเป็น transition ของ match ID เดียวกับ live fixture. ห้ามเก็บค่าตัวอย่างเดิมที่ไม่ได้มาจาก source runtime
6. สร้าง detail decoders จาก actual hydrated response/store โดย preserve non-chat raw fields และ map known sections. Decoder ต้องตรวจ match ID/sport และ reject detail ที่เป็น match ก่อนหน้า
7. Sanitize fixtures ให้ไม่มี cookie, headers, token, chat หรือ payload ส่วนเกิน และเพิ่ม tests สำหรับ relation integrity, Live-vs-All rejection, live→finished transition, detail match mismatch และ source-shape error

## Acceptance Criteria

- fixture แต่ละไฟล์มี provenance comment/metadata ที่ระบุ URL surface, capture layer (network/store) และ sanitized capture date โดยไม่ใส่ secret
- football/basketball Live fixtures decode ได้พร้อม competition/team relations ครบ; All/previous-state fixture ถูก reject ว่าไม่ใช่ ready Live snapshot
- status/score mapping มี test แยกสองกีฬาและ finished transition ใช้ match identity เดิม
- detail decoder ยอมรับ hydrated fixture ของ match ที่ร้องขอและ reject wrong/empty match ID
- unknown optional fields ถูกเก็บใน sanitized raw payload; missing required IDs ไม่ถูก insert เป็น empty string
- tests เดิมเรื่อง chat exclusion และ reconciliation ยังผ่าน

## Validation

```bash
cargo fmt -- --check
cargo test source_contract_
cargo test source_filter_
cargo test detail_decoder_
cargo test extractor_
cargo test chat_exclusion_
```

Chrome DevTools validation ต้องทำแยก football และ basketball:

1. เปิด All แล้วรัน extractor readiness probe — ต้องรายงาน `filter_not_live`
2. เลือก Live, รอ source settle แล้ว probe — ต้องได้ match/entity counts และ relation error เป็นศูนย์
3. เปิด detail ของ match ที่เลือกและ compare requested/hydrated ID — ต้องตรงกัน
4. เปรียบเทียบ field paths ที่เห็นกับ fixture แบบ file-specific; ห้ามยืนยันเพียงว่า JSON parse ผ่าน

## Reference Map

### Generated Knowledge/Cache

- `.ai-agent/generated/knowledge/architecture.md` — current `SportAdapter`/extractor/test symbols
- `.ai-agent/generated/knowledge/api.md` — CDP target, Runtime.evaluate และ current detail call sites
- `.ai-agent/generated/cache/codegraph-project.md` — adapters → reconciliation → persistence relationships
- `.ai-agent/generated/cache/symbol-index.md` — exact adapter/save/test symbols

### Exact Source Files

- `src/main.rs` — `SportAdapter`, `FootballAdapter`, `BasketballAdapter`, extraction tests
- `Cargo.toml` — existing JSON/async dependencies
- `tests/fixtures/football-live.json`
- `tests/fixtures/football-finished.json`
- `tests/fixtures/basketball-live.json`
- `tests/fixtures/basketball-finished.json`
- `tests/fixtures/football-detail.json` — task-produced fixture
- `tests/fixtures/basketball-detail.json` — task-produced fixture
- `tests/fixtures/source-filter-states.json` — task-produced fixture

### External Runtime References

- `https://m.aiscore.com/`
- `https://m.aiscore.com/basketball`
