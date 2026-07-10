## Status
Pending

# Task 06: Local Database Identity, Dataset Generation, and Reset Recovery

## Outcome

SQLite file ใหม่มี dataset generation ใหม่ที่ตรวจสอบได้, สอง sport sessions บน file เดียวใช้ generation เดียวกัน และ process ไม่อ่าน/ส่ง handle ของ DB เก่าต่อเมื่อ `local.db` ถูกลบหรือแทนที่ระหว่างรัน

## Dependencies

- Task 01 schema/migration baseline

## Implementation Scope

### Required Files

- `src/main.rs` — canonical DB path, dataset metadata initialization, file identity checks, reset recovery, row generation fields และ focused tests
- `dashboard/schema.sql` — final D1 schema เพิ่ม dataset metadata/columns/indexes
- `dashboard/migrations/0002_dataset_generation.sql` — incremental D1 migration จาก 0001
- `dashboard/test/legacy-schema.sql` — extend migration fixture เพื่อพิสูจน์ 0001→0002 และ preserved legacy rows
- `tests/fixtures/sync-batch.json` — เพิ่ม dataset contract ที่ Task 08 จะใช้

### Allowed Files

- `dashboard/package.json` — migration validation script เท่านั้น
- `dashboard/wrangler.toml` — migration directory/version config เท่านั้น
- `Cargo.toml` — dependency เฉพาะเมื่อ file identity/temporary DB tests ทำด้วย standard library + current crates ไม่ได้

### Out of Scope

- Chrome target/readiness behavior
- `/api/sync` activation/filter logic
- dashboard rendering

## Implementation Steps

1. Resolve `--db-path` เป็น absolute normalized path ก่อน init/spawn และใช้ path เดียวกันทุก task; log path, dataset ID และ sport โดยไม่ log secrets. Relative paths จาก working directory ต่างกันต้องเห็นได้ชัด
2. เพิ่ม persistent dataset metadata (`dataset_id`, created timestamp, schema generation) ที่สร้าง atomic ครั้งเดียวเมื่อ SQLite ใหม่. Concurrent football/basketball startup ต้อง converge เป็น UUID เดียว ไม่ overwrite กัน
3. เพิ่ม `dataset_id` ให้ competitions, teams, matches, match_details และ indexes ที่ query/dirty sync ใช้. กำหนด migration/backfill สำหรับ existing local rows เป็น legacy/current dataset อย่าง deterministic และไม่ทำข้อมูลหาย
4. ปรับ save/select/detail scheduling/dirty queries ให้ scope ด้วย dataset ID; upstream ID เดียวกันจาก generation ใหม่ต้องไม่ดึง relation/detail/synced state ของ generation เก่าโดยบังเอิญ
5. ทำ DB identity guard ก่อน reconciliation และ sync claim: ตรวจ file existence/metadata + stored dataset ID เทียบกับ session. เมื่อ file ถูก unlink/replaced ให้หยุดใช้ connection/lease เดิม, re-run migrations, สร้าง/อ่าน generation ใหม่ และ clear in-memory pending work ของ generation เก่า หรือ terminate ด้วย actionable error หาก recovery ไม่ปลอดภัย
6. ออกแบบ D1 final schema/migration 0002 สำหรับ dataset registry/active metadata และ entity dataset columns. พิสูจน์ก่อนว่าจะใช้ existing PK + dataset filter หรือ composite identity; ต้องรองรับ overlapping IDs และ old-only IDs โดย active query ไม่ปะปน
7. Update sync fixture ให้มี dataset envelope/row association แต่ยังไม่ implement Worker activation ใน task นี้
8. เพิ่ม tests: fresh DB UUID persistence, concurrent init convergence, different files produce different IDs, legacy migration/backfill, delete/recreate while running simulation, generation-scoped relation/detail queries และ migration idempotence

## Acceptance Criteria

- เปิด SQLite file เดิมซ้ำได้ dataset ID เดิม; file ใหม่/ถูก recreate ได้ ID ใหม่
- two concurrent initializers บน path เดียวสร้าง dataset row เดียวและทั้งคู่เห็น ID เดียว
- absolute DB path และ dataset ID ปรากฏใน startup diagnostics ทำให้แยกกรณีลบผิด path ได้
- old-generation rows ไม่ถูกเลือกโดย save/detail/dirty queries ของ generation ใหม่
- delete/replacement detection ไม่ sync generation เก่าต่อ และ recovery path สร้าง schema/default settings ครบก่อน resume
- D1 fresh schema และ 0001→0002 migration preserve legacy rows พร้อม dataset backfill/indexes
- sync fixture มี dataset identity ครบทุก entity contract และไม่มี chat/secret

## Validation

```bash
cargo fmt -- --check
cargo test dataset_
cargo test db_identity_
cargo test reset_recovery_
cargo test migration_
```

```bash
cd dashboard && npx wrangler d1 execute earnscore-db --local --persist-to /tmp/earnscore-gen-fresh --file=schema.sql
cd dashboard && npx wrangler d1 execute earnscore-db --local --persist-to /tmp/earnscore-gen-fresh --command="PRAGMA table_info(matches)"
cd dashboard && npx wrangler d1 execute earnscore-db --local --persist-to /tmp/earnscore-gen-legacy --file=test/legacy-schema.sql
cd dashboard && npx wrangler d1 migrations apply earnscore-db --local --persist-to /tmp/earnscore-gen-legacy
cd dashboard && npx wrangler d1 execute earnscore-db --local --persist-to /tmp/earnscore-gen-legacy --command="SELECT id,dataset_id FROM matches WHERE id='legacy-match'"
```

Manual DB-path validation:

```bash
cargo run -- --db-path ./local.db football
cargo run -- --db-path /tmp/earnscore-second.db basketball
```

Reviewer ต้อง compare logged absolute paths/dataset IDs และทำ controlled delete/recreate test กับ temporary DB เท่านั้น; ห้ามลบ production/user DB

## Reference Map

### Generated Knowledge/Cache

- `.ai-agent/generated/knowledge/database.md` — current SQLite/D1 tables and indexes
- `.ai-agent/generated/knowledge/architecture.md` — `open_db`, `init_db`, migrations, save/sync symbols
- `.ai-agent/generated/cache/schema-index.md` — exact schema baseline before 0002
- `.ai-agent/generated/cache/symbol-index.md` — DB initialization/save/detail/lease query locations

### Exact Source Files

- `src/main.rs` — CLI path, `open_db`, `init_db`, `run_migrations`, save/detail/lease queries and tests
- `dashboard/schema.sql` — fresh D1 schema
- `dashboard/migrations/0001_full_payload.sql` — migration dependency/read-only baseline
- `dashboard/migrations/0002_dataset_generation.sql` — task-produced migration
- `dashboard/test/legacy-schema.sql` — migration seed fixture
- `dashboard/package.json` — migration commands
- `dashboard/wrangler.toml` — D1 migration binding/config
- `tests/fixtures/sync-batch.json` — dataset-aware contract fixture
- `Cargo.toml` — UUID/current dependencies
