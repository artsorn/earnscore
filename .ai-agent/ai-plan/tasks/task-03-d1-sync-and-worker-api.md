## Status
Passed

# Task 03: Concurrent-Safe D1 Sync and Worker APIs

## Outcome

ข้อมูล dirty ทุกชนิดถูกส่งจาก shared SQLite ไป D1 แบบ bounded/idempotent ตาม interval ที่ตั้งใน D1, สอง CLI ไม่แข่งกันส่ง batch เดียว และ Worker list/detail/settings APIs คืน contract ที่ dashboard ใช้ได้แม้ detail ยังมาไม่ถึง

## Dependencies

- Task 01 schema/migration เสร็จ
- Task 02 crawler upsert/dirty lifecycle เสร็จ

## Implementation Scope

### Required Files

- `src/main.rs` — `sync_worker`, lease/claim, payload batching, acknowledgement, interval refresh และ sync tests
- `dashboard/src/index.js` — เฉพาะ authentication helper และ routes `/api/sync`, `/api/matches/live`, `/api/matches/detail`, `/api/settings`
- `dashboard/schema.sql` — แก้เฉพาะเมื่อ query/index/default contract จาก Task 01 ต้อง align; ห้ามออกแบบ schema ใหม่ซ้ำ
- `dashboard/migrations/0001_full_payload.sql` — align เฉพาะกรณีพบ mismatch กับ Task 01
- `tests/fixtures/sync-batch.json` — sanitized versioned payload ที่มีสองกีฬา, entity, match และ detail สำหรับ Worker integration validation

### Allowed Files

- `dashboard/package.json` — เพิ่ม validation script/test harness ที่จำเป็นต่อ Worker API เท่านั้น
- `dashboard/wrangler.toml` — เพิ่ม non-secret variable/binding ที่ route tests ต้องใช้เท่านั้น
- `Cargo.toml` — dependency เฉพาะที่จำเป็นต่อ sync protocol/testing

### Out of Scope

- CDP extraction logic
- Embedded dashboard HTML/CSS/rendering หลัง `// 4. GUI`
- External scheduler service; cadence ยังคุมโดย Rust sync worker/settings

## Implementation Steps

1. กำหนด versioned sync request/response contract สำหรับ competitions, teams, matches และ match details พร้อม bounded batch size, batch/request ID และ sport/content identity. Worker ต้อง validate array/object/type/required IDs และ strip/reject chat sections ก่อนสร้าง D1 statements
2. ทำ SQLite lease/claim ด้วย transaction atomic และ expiry เพื่อให้ uploader เดียวทำงานข้ามสอง CLI processes. Process crash ต้อง recover หลัง lease หมดอายุ; ห้าม mark row ก่อน Worker acknowledgement
3. อ่าน dirty rows ของทุก entity type เป็น deterministic batches. ส่ง payload นอก SQLite transaction, retry ด้วย exponential backoff/jitter และหลัง success mark เฉพาะ IDs/content versions ที่ response ยืนยัน เพื่อไม่กลบ change ที่เกิดระหว่าง request
4. ปรับ `/api/sync` ให้ upsert normalized + raw fields ทุก entity อย่าง idempotent, ใช้ D1 batches ไม่เกิน platform limit, ตอบ partial/invalid payload อย่างชัดเจน และไม่คืน/log API token/full raw payload
5. ให้ response ของ successful sync คืน validated authoritative `sync_interval_mins` (หรือเพิ่ม authenticated settings read ที่ชัดเจน). Rust persist/cache ค่านี้สำหรับรอบถัดไป โดย clamp minimum/maximum และ fallback เมื่อ D1/setting ไม่พร้อม
6. ปรับ `/api/settings` ให้ validate numeric ranges. GET ไม่คืน token; write ที่เปลี่ยน sensitive configuration ต้องผ่าน auth contract ที่ UI/CLI ใช้ได้จริง. การเปลี่ยน interval ใน D1 ต้องเห็นผลกับ running sync worker ภายในไม่เกินหนึ่งรอบเดิม
7. แก้ `/api/matches/live` ให้คืน matches ที่ต้องแสดงโดยไม่บังคับมี detail, รองรับ `sport_id` filter, latest/today/live-finished policy ที่ระบุชัด, stable ordering และ safe JSON parse fallback. Response ต้องมี status/score/raw-summary fields ที่ UI task ต้องใช้
8. แก้ `/api/matches/detail` ให้คืน 400/404 ถูกต้อง, parse known sections อย่างปลอดภัย และคืน sanitized extra payload โดยไม่ส่ง chat. Malformed stored JSON หนึ่ง field ต้องไม่ทำทั้ง endpoint 500
9. เพิ่ม Rust tests สำหรับ lease exclusion/recovery, dirty change during in-flight ack, batching, retry/no false-synced และ interval update. เพิ่ม Worker local integration validation สำหรับ unauthorized sync, valid upsert, idempotent replay, list-before-detail, detail parse และ setting range

## Acceptance Criteria

- shared SQLite มี active uploader สูงสุดหนึ่งตัว; lease หมดอายุแล้ว process อื่นรับช่วงได้
- team/competition-only change และ match/detail change ถูกส่งได้ทั้งหมด; failed/partial request ไม่ทำ row สูญจาก dirty queue
- replay payload เดิมไม่สร้าง duplicate และ batch ใหญ่ถูกแบ่งโดยไม่เกิน D1 limit
- interval ที่แก้ใน D1 ถูก Rust process นำไปใช้จริง; invalid interval ถูก reject/clamp และไม่เกิด tight loop
- `/api/matches/live` แสดง match ใหม่ก่อนมี detail และ filter สองกีฬาได้; finished state ล่าสุดไม่ถูกซ่อนเพราะ join
- `/api/matches/detail` คืน known + extra non-chat detail และทน malformed optional JSON
- unauthorized `/api/sync`/protected settings write ได้ 401; API response/log ไม่เผย token หรือ chat payload

## Validation

```bash
cargo fmt -- --check
cargo test sync_
cargo test lease_
node --check dashboard/src/index.js
cd dashboard && npx wrangler deploy --dry-run
```

ใช้ local D1/Worker แล้วพิสูจน์ route แยก criterion (seed payload ต้องมาจาก sanitized fixtures ของ Task 02):

```bash
cd dashboard && npm run d1:init
cd dashboard && npm run dev
curl -i -X POST http://127.0.0.1:8080/api/sync -H 'Content-Type: application/json' --data-binary @../tests/fixtures/sync-batch.json
curl -i -X POST http://127.0.0.1:8080/api/sync -H 'Authorization: Bearer super-secret-token' -H 'Content-Type: application/json' --data-binary @../tests/fixtures/sync-batch.json
curl -sS 'http://127.0.0.1:8080/api/matches/live?sport_id=1'
curl -sS 'http://127.0.0.1:8080/api/matches/live?sport_id=2'
curl -sS 'http://127.0.0.1:8080/api/matches/detail?match_id=fixture-football-live'
```

Reviewer ต้องตรวจ status code/body ของแต่ละ route และ query D1 row count หลัง replay `tests/fixtures/sync-batch.json`; ห้ามใช้เพียง `node --check` เป็นหลักฐาน behavior

## Reference Map

### Generated Knowledge/Cache

- `.ai-agent/generated/knowledge/api.md` — current Worker routes และ Rust sync call sites
- `.ai-agent/generated/knowledge/database.md` — tables ที่ sync/query ใช้
- `.ai-agent/generated/cache/api-index.md` — exact route markers ใน monolithic Worker
- `.ai-agent/generated/cache/schema-index.md` — schema fields สำหรับ prepared statements
- `.ai-agent/generated/cache/symbol-index.md` — `sync_worker`/DTO symbols

### Exact Source Files

- `src/main.rs` — `sync_worker`, DTO queries และ settings reads
- `dashboard/src/index.js` — auth, `/api/sync`, `/api/matches/live`, `/api/matches/detail`, `/api/settings`
- `dashboard/schema.sql` — D1 contract/indexes/defaults
- `dashboard/migrations/0001_full_payload.sql` — existing-D1 upgrade จาก Task 01
- `dashboard/wrangler.toml` — DB binding/API token fallback
- `dashboard/package.json` — local D1/Worker commands
- `tests/fixtures/sync-batch.json` — task-produced versioned integration payload
