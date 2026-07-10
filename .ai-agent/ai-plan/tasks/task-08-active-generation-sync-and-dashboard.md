## Status
Pending

# Task 08: Active-Generation D1 Sync, APIs, and Dashboard Freshness

## Outcome

Rust sync และ Cloudflare Worker activate dataset generation ใหม่อย่างปลอดภัย, list/detail APIs และ dashboard แสดงเฉพาะ generation ปัจจุบันโดยไม่ใช้ cached/old D1 rows และผู้ใช้เห็นว่า data source ใหม่กำลังรอหรืออัปเดตล่าสุดเมื่อใด

## Dependencies

- Task 06 dataset schema/local generation
- Task 07 correct Live/detail capture and DB reset guard

## Implementation Scope

### Required Files

- `src/main.rs` — dataset-aware sync envelope, acknowledgement validation, generation-safe dirty marking and end-to-end sync tests
- `dashboard/src/index.js` — `/api/sync`, `/api/matches/live`, `/api/matches/detail`, response cache headers and embedded dashboard generation/freshness states
- `dashboard/schema.sql` — align final active-generation metadata/query indexes from Task 06 only
- `dashboard/migrations/0002_dataset_generation.sql` — align incremental migration from Task 06 only
- `tests/fixtures/sync-batch.json` — current-generation sync fixture
- `tests/fixtures/sync-batch-next-generation.json` — new generation with overlapping and old-only match cases

### Allowed Files

- `dashboard/package.json` — local Worker/D1 integration scripts
- `dashboard/wrangler.toml` — non-secret test configuration only
- `Cargo.toml` — dependency only when current HTTP/serde tooling cannot test protocol

### Out of Scope

- AiScore source extraction/tab lifecycle
- destructive automatic deletion of old D1 generations
- redesign of dashboard visual language unrelated to freshness/provenance

## Implementation Steps

1. Version sync envelope with `dataset_id`, dataset creation metadata and sport/batch identity. Every row must agree with envelope generation; Worker rejects mixed/missing generation before executing D1 writes
2. Rust dirty selection scopes current generation; after response, mark synced only IDs/content versions whose acknowledgement contains the same dataset ID. Response from previous generation after DB reset must be discarded and not mutate new DB rows
3. In `/api/sync`, upsert dataset registry + rows and activate new generation atomically enough that APIs never combine generations. Define ordering/idempotency so replay of current batch is safe and a stale older process cannot silently reactivate old generation
4. Handle two sport sessions in same generation: activating first sport must hide old generation immediately, while API reports the second sport as pending/empty until its first current-generation batch arrives; it must never fall back to old basketball/football rows
5. Update `/api/matches/live` to filter active dataset on matches and all joins, return dataset/freshness/sport readiness metadata, and set `Cache-Control: no-store, no-cache, must-revalidate` plus appropriate `Pragma/Expires`
6. Update `/api/matches/detail` to require that match and detail belong to active dataset; old-generation match ID returns 404/409 as defined, not old detail. Apply same no-store headers to success/error JSON
7. Update dashboard fetches with `cache: 'no-store'`. Render explicit states: waiting for new generation, sport not synced yet, active dataset with last capture/sync time, and request failure. Clear old card/detail DOM as soon as API generation changes
8. Keep dataset ID presentation abbreviated/non-sensitive; do not expose API token or internal lease owner. Expanded card state may carry over only within same dataset ID
9. Add protocol/integration tests using generation A then generation B fixture: B overlaps one ID, omits an A-only match and initially contains one sport. Prove list/detail never return A rows after B activation and second-sport pending does not fall back
10. Run full regression suite for chat sanitization, lease, settings, dashboard escaping/status and local DB recreation end-to-end

## Acceptance Criteria

- deleting/recreating local SQLite produces generation B; first successful B sync makes all list/detail API responses exclude generation A rows
- overlapping match IDs return B data; A-only match/detail are absent without physically deleting A rows
- after only B football sync, basketball response is explicit pending/empty and never shows A basketball; after B basketball sync both sports share B
- mixed-generation payload is rejected atomically; stale acknowledgement cannot mark new-generation local rows synced
- list/detail responses and dashboard requests use no-store semantics; generation change clears old cards/details immediately
- dashboard distinguishes waiting, empty-live, stale/error and active-fresh states and shows last source/sync timestamps
- two-session lease/settings/security/chat behavior from completed tasks remains intact

## Validation

```bash
cargo fmt -- --check
cargo test sync_generation_
cargo test reset_recovery_
cargo test sync_
cargo test lease_
node --check dashboard/src/index.js
cd dashboard && npx wrangler deploy --dry-run
```

Local D1/Worker generation sequence:

```bash
cd dashboard && npm run d1:init
cd dashboard && npm run dev
curl -i -X POST http://127.0.0.1:8080/api/sync -H 'Authorization: Bearer super-secret-token' -H 'Content-Type: application/json' --data-binary @../tests/fixtures/sync-batch.json
curl -i http://127.0.0.1:8080/api/matches/live
curl -i -X POST http://127.0.0.1:8080/api/sync -H 'Authorization: Bearer super-secret-token' -H 'Content-Type: application/json' --data-binary @../tests/fixtures/sync-batch-next-generation.json
curl -i 'http://127.0.0.1:8080/api/matches/live?sport_id=1'
curl -i 'http://127.0.0.1:8080/api/matches/live?sport_id=2'
curl -i 'http://127.0.0.1:8080/api/matches/detail?match_id=fixture-generation-a-only'
```

Reviewer ต้องตรวจ response bodies และ headers ทีละ call, query D1 เพื่อพิสูจน์ว่า A rows ยังอยู่แต่ API filter ออก, จากนั้นเปิด dashboard ระหว่าง A→B เพื่อยืนยันว่า DOM เก่าหายทันทีและ second sport แสดง pending ไม่ใช่ข้อมูลเก่า

## Reference Map

### Generated Knowledge/Cache

- `.ai-agent/generated/knowledge/api.md` — current sync/list/detail/settings/client call sites
- `.ai-agent/generated/knowledge/database.md` — schema/index baseline
- `.ai-agent/generated/knowledge/frontend.md` — dashboard client baseline
- `.ai-agent/generated/cache/api-index.md` — exact Worker routes/client fetches
- `.ai-agent/generated/cache/schema-index.md` — active-generation columns/indexes from Task 06
- `.ai-agent/generated/cache/symbol-index.md` — `sync_worker`/DTO/test symbols

### Exact Source Files

- `src/main.rs` — `sync_worker`, sync DTO/ack handling, DB generation guard and tests
- `dashboard/src/index.js` — `/api/sync`, `/api/matches/live`, `/api/matches/detail`, `loadMatches`, detail state
- `dashboard/schema.sql`
- `dashboard/migrations/0002_dataset_generation.sql`
- `dashboard/package.json`
- `dashboard/wrangler.toml`
- `tests/fixtures/sync-batch.json`
- `tests/fixtures/sync-batch-next-generation.json` — task-produced generation transition fixture
