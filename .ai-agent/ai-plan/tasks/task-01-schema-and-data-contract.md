## Status
Passed

# Task 01: Schema, Migration, and Full-Payload Contract

## Outcome

SQLite และ D1 มี schema ปลายทางที่เก็บ normalized fields เดิมพร้อม sanitized raw payload/dirty metadata ได้, อัปเกรดฐานข้อมูลเดิมโดยไม่ลบข้อมูล และรองรับ writer สอง Rust processes บน SQLite file เดียว

## Dependencies

- ไม่มี implementation task ก่อนหน้า

## Implementation Scope

### Required Files

- `src/main.rs` — data structs, `init_db`, idempotent SQLite migration helpers, connection pragmas และ schema-focused tests เท่านั้น
- `dashboard/schema.sql` — final bootstrap schema/default settings สำหรับ D1 ใหม่
- `dashboard/migrations/0001_full_payload.sql` — migration จาก schema baseline เดิมไป schema ใหม่
- `dashboard/test/legacy-schema.sql` — fixture ของ D1 schema เดิมพร้อม seed row สำหรับพิสูจน์ incremental migration
- `dashboard/package.json` — แยกคำสั่ง bootstrap DB ใหม่กับ apply migrations สำหรับ DB เดิมให้ชัด
- `dashboard/wrangler.toml` — ผูก migration directory กับ D1 binding หาก Wrangler config ต้องระบุ

### Allowed Files

- `Cargo.toml` เฉพาะเมื่อจำเป็นต่อ serialization/canonicalization/migration tests; ห้ามเพิ่ม dependency หาก standard library/ของเดิมเพียงพอ

### Out of Scope

- CDP extraction/event loop
- HTTP sync behavior และ Worker route logic
- Dashboard HTML/CSS/rendering

## Implementation Steps

1. กำหนด schema contract ของ `competitions`, `teams`, `matches`, `match_details`, `settings` โดยรักษา column เดิม และเพิ่ม field ที่จำเป็นสำหรับ sanitized full payload, change detection, per-entity dirty state และ timestamps. หลีกเลี่ยงการแตกทุก upstream field เป็น column; JSON ต้องเป็น valid JSON และไม่เก็บ chat
2. ทำ SQLite migration helper ที่ตรวจ `PRAGMA table_info` ก่อนเพิ่ม column/index และรันใน transaction. เปิด `journal_mode=WAL`, กำหนด `busy_timeout`, ใช้ foreign-key/index เฉพาะที่ไม่ทำลายข้อมูลเดิม และให้ `init_db` เรียก migration ได้ซ้ำ
3. ทำให้ local entity ทุกชนิดที่ต้อง sync มี dirty/synced lifecycle ของตัวเอง รวมถึง competition/team ไม่ใช่เฉพาะ match/detail. Default row เดิมต้องถูกจัดสถานะให้ส่งเติม raw contract ได้อย่างปลอดภัย
4. ปรับ Rust DTOs ให้ serialize contract เดียวกับ `/api/sync`: normalized fields + sanitized raw JSON; แยก local-only columns เช่น lease/synced ออกจาก wire payload
5. ปรับ `dashboard/schema.sql` ให้เป็น final fresh schema และเพิ่ม index ที่ list/detail queries ใช้ (`sport_id/status/time`, entity joins, detail lookup) พร้อม defaults สำหรับ `sync_interval_mins`, `detail_update_interval_secs`, `api_token`
6. เพิ่ม incremental D1 migration สำหรับฐานเดิม. Migration ต้องไม่ drop table/data และต้องสอดคล้องกับ final bootstrap schema; ระบุ workflow ใน npm scripts ว่า DB ใหม่ใช้ bootstrap ส่วน DB เดิมใช้ `wrangler d1 migrations apply`
7. เพิ่ม focused Rust tests สร้าง temporary legacy SQLite schema, insert ข้อมูลเดิม, เรียก migration สองครั้ง แล้วตรวจทั้ง preserved rows, new columns/defaults, indexes และ concurrent connection pragmas

## Acceptance Criteria

- fresh SQLite และ legacy SQLite หลัง migration มี column/index ปลายทางตรงกัน และข้อมูลเดิมยังอยู่
- `init_db` เรียกซ้ำได้โดยไม่ error; connection สองตัวรอ lock ตาม busy timeout แทน fail ทันที
- fresh D1 bootstrap และ incremental D1 migration ให้ final logical schema เดียวกันโดยไม่ลบข้อมูลเดิม
- entity/match/detail contract รองรับ raw JSON และ dirty state; wire DTO ไม่ส่ง local-only sync metadata
- defaults ของ interval มีชนิด/range ที่ task ถัดไปใช้ได้ และ secret token ไม่ถูกเพิ่มใน output/debug formatting

## Validation

```bash
cargo fmt -- --check
cargo test schema_
cargo test migration_
```

ตรวจ D1 ใหม่และ migration แยกกัน เพื่อพิสูจน์ทั้งสอง path:

```bash
cd dashboard && npx wrangler d1 execute earnscore-db --local --persist-to /tmp/earnscore-fresh --file=schema.sql
cd dashboard && npx wrangler d1 execute earnscore-db --local --persist-to /tmp/earnscore-fresh --command="PRAGMA table_info(matches)"
cd dashboard && npx wrangler d1 execute earnscore-db --local --persist-to /tmp/earnscore-fresh --command="PRAGMA table_info(match_details)"
cd dashboard && npx wrangler d1 execute earnscore-db --local --persist-to /tmp/earnscore-legacy --file=test/legacy-schema.sql
cd dashboard && npx wrangler d1 migrations apply earnscore-db --local --persist-to /tmp/earnscore-legacy
cd dashboard && npx wrangler d1 execute earnscore-db --local --persist-to /tmp/earnscore-legacy --command="SELECT id FROM matches WHERE id='legacy-match'"
```

Reviewer ต้องตรวจผล `PRAGMA table_info` ทีละตารางว่ามี raw/change fields ตาม contract และรัน migration test ที่ seed legacy row เพื่อยืนยันว่า row ไม่หาย; ห้ามยืนยันด้วยการ grep schema อย่างเดียว

## Reference Map

### Generated Knowledge/Cache

- `.ai-agent/generated/knowledge/database.md` — baseline tables ใน SQLite/D1
- `.ai-agent/generated/knowledge/architecture.md` — entrypoints และ project file map
- `.ai-agent/generated/cache/schema-index.md` — exact schema definitions ที่ต้องรักษา compatibility
- `.ai-agent/generated/cache/dependency-index.md` — dependency baseline ก่อนพิจารณาแก้ `Cargo.toml`

### Exact Source Files

- `src/main.rs` — structs และ `init_db`
- `dashboard/schema.sql` — D1 baseline schema/defaults
- `dashboard/migrations/0001_full_payload.sql` — task-produced incremental migration สำหรับ D1 เดิม
- `dashboard/test/legacy-schema.sql` — task-produced legacy fixture/seed สำหรับ migration validation
- `dashboard/package.json` — current D1 bootstrap scripts
- `dashboard/wrangler.toml` — D1 binding/config
- `Cargo.toml` — current Rust dependencies
