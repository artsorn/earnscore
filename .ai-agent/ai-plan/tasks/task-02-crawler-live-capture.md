## Status
Passed

# Task 02: Two-Session Live Capture and Detail Reconciliation

## Outcome

Rust CLI `football` และ `basketball` ยึด Chrome Live tab ของตนเอง, เก็บ match/entity/detail ที่เข้าถึงได้ทั้งหมดแบบ normalized + sanitized raw JSON, ตรวจพบคู่ใหม่และการเปลี่ยนแปลงจนถึง finished state และเขียน SQLite เฉพาะเมื่อ content เปลี่ยนจริง

## Dependencies

- Task 01 schema/data contract เสร็จและ tests ผ่าน

## Implementation Scope

### Required Files

- `src/main.rs` — CLI target selection, CDP lifecycle, sport adapters, extraction/sanitization, reconciliation, detail scheduling, SQLite upserts และ crawler tests
- `tests/fixtures/football-live.json` — sanitized minimal fixture จาก runtime shape ที่ยืนยันแล้ว
- `tests/fixtures/football-finished.json` — sanitized final-state fixture
- `tests/fixtures/basketball-live.json` — sanitized minimal fixture จาก runtime shape ที่ยืนยันแล้ว
- `tests/fixtures/basketball-finished.json` — sanitized final-state fixture

### Allowed Files

- `Cargo.toml` เฉพาะ dependency ที่จำเป็นต่อ CDP/canonical JSON/testing และอธิบายไม่ได้ด้วยของเดิม

### Out of Scope

- D1 upload/Worker API implementation
- Dashboard rendering
- กีฬาอื่นและ chat capture

## Implementation Steps

1. ใช้ Chrome DevTools MCP/CDP ตรวจ `https://m.aiscore.com/` และ `https://m.aiscore.com/basketball` ขณะเลือก Live เพื่อยืนยัน:
   - page target URL/title ที่แยกสองกีฬาได้แน่นอน
   - network responses/store modules ที่มี match, team, competition และ detail
   - status IDs/clock/score arrays ของ scheduled, live, interval และ finished แยกตามกีฬา
   - detail sections ที่มีจริง และ endpoint/state keys ที่เป็น chat ซึ่งต้อง ignore
2. แยก `SportAdapter`/equivalent สำหรับ football และ basketball ให้รวม target matching, Live navigation/filter activation, list extraction, detail URL/shape, score/status mapping. ห้าม fallback ไป hijack tab ของอีกกีฬา; ถ้าไม่พบ target ที่ถูกต้องให้ retry พร้อม actionable log
3. เปลี่ยน trigger จาก mutation-name อย่างเดียวเป็น CDP event strategy ที่ทนต่อ source changes: initial fetch, relevant network/runtime notification และ periodic full reconciliation. Coalesce bursts และรักษา WebSocket ownership ไม่ให้มีหลาย connection แย่ง response ID
4. Extract list response/state ให้รวมทุก match/entity ที่หน้า Live โหลดแล้ว แม้ collection เป็น array/map หรือ field name ต่างระหว่างกีฬา. ใช้ runtime state fallback เฉพาะ shape ที่ยืนยัน และ log metric/count เมื่อ source shape ไม่ตรงแทน silently returning empty
5. ทำ recursive sanitization ก่อน persistence: ตัด known chat/message/comment-room keys และไม่อ่าน response URL ที่จัดเป็น chat; preserve non-chat unknown fields ใน canonical `raw_json`. Unit test ต้องมี nested chat-like section เพื่อพิสูจน์ว่าถูกลบโดยข้อมูลกีฬาอื่นไม่หาย
6. Upsert competition/team/match ใน transaction สั้น. เปรียบเทียบ normalized + canonical raw content กับ row เดิม; เปลี่ยน `updated_at`/dirty เฉพาะเมื่อข้อมูลเปลี่ยน. ต้อง update relation/time/status ด้วย ไม่ใช่เฉพาะ score/status และต้องไม่ย้อน finished row ด้วย stale payload
7. ทำ detail scheduler สำหรับ new/live/just-finished matches: fetch ครั้งแรก, refresh active ตาม setting, final fetch หลัง finished และหยุด polling final row เมื่อ final detail บันทึกสำเร็จ. Failure ของหนึ่ง match ต้องไม่หยุด reconciliation ทั้งรอบ; จำกัด concurrency/rate และมี timeout/retry backoff
8. บันทึก known detail fields เพื่อ compatibility พร้อม full sanitized detail payload. Pair response กับ requested match ID อย่างชัดเจน; ห้ามใช้ response ID คงที่ซ้ำใน concurrent flow
9. รักษา two-session CLI UX (`football`, `basketball`, shared `--db-path`, `--chrome-url`) และ log sport/target/reconcile counts โดยไม่ log token/full payload
10. เพิ่ม fixture-based tests สำหรับ extraction ทั้งสองกีฬา, new match insert, unchanged no-op, live score update, finished transition, final-detail scheduling และ chat exclusion

## Acceptance Criteria

- สอง CLI processes ที่ชี้ SQLite เดียวกันเลือกคนละ Chrome tab และทำงานต่อได้เมื่ออีก tab reconnect/reload
- initial fetch count ตรงกับทุก match ที่ Live source adapter เห็น; match ที่เพิ่มหลัง start ถูก insert โดยไม่ restart
- score/time/status/entity/detail change ทำ row dirty ครั้งเดียวต่อ content version; unchanged periodic fetch ไม่แก้ `updated_at` และไม่ reset synced
- football และ basketball finished fixtures เปลี่ยนสถานะ final และทำ final detail fetch ก่อนหยุด polling
- normalized fields และ sanitized raw payload ถูกบันทึกครบตาม fixture; nested chat content ไม่ปรากฏใน DB JSON
- source-shape mismatch, iframe/detail timeout หรือ malformed match หนึ่งรายการไม่ทำให้ process ตายหรือทำให้ทั้ง batch หาย

## Validation

```bash
cargo fmt -- --check
cargo test crawler_
cargo test extractor_
cargo test chat_exclusion_
cargo run -- --help
cargo run -- football --help
cargo run -- basketball --help
```

Manual live validation ต้องรันสอง terminal กับ browser remote debugging และ DB เดียว:

```bash
cargo run -- --chrome-url http://127.0.0.1:9223 --db-path /tmp/earnscore-live.db football
cargo run -- --chrome-url http://127.0.0.1:9223 --db-path /tmp/earnscore-live.db basketball
sqlite3 /tmp/earnscore-live.db "SELECT sport_id,status_id,COUNT(*) FROM matches GROUP BY sport_id,status_id ORDER BY sport_id,status_id;"
sqlite3 /tmp/earnscore-live.db "SELECT COUNT(*) FROM matches WHERE raw_json LIKE '%chat%' OR raw_json LIKE '%messageRoom%';"
sqlite3 /tmp/earnscore-live.db "SELECT match_id,last_updated FROM match_details ORDER BY last_updated DESC LIMIT 10;"
```

Reviewer ต้องสังเกตอย่างน้อยหนึ่ง change/new match หรือ replay fixture ผ่าน reconciliation และยืนยัน status/detail row แบบ file/query-specific; จำนวน chat query ต้องเป็นศูนย์ โดย unit test เป็นหลักฐานเชิงโครงสร้างเพิ่มเติม ไม่ใช้ substring query เพียงอย่างเดียว

## Reference Map

### Generated Knowledge/Cache

- `.ai-agent/generated/knowledge/architecture.md` — Rust entrypoint/functions และ current dependency markers
- `.ai-agent/generated/knowledge/api.md` — CDP target/state/detail/sync call sites ใน baseline
- `.ai-agent/generated/cache/symbol-index.md` — exact Rust symbols ที่ต้องแยก responsibility
- `.ai-agent/generated/cache/codegraph-project.md` — relation ของ `main`, state fetch และ persistence ในไฟล์เดียว

### Exact Source Files

- `src/main.rs` — `Commands`, `get_websocket_url`, save functions, `trigger_state_fetch`, injected JS และ main event loop
- `Cargo.toml` — async/CDP/SQLite dependencies
- `dashboard/schema.sql` — destination contract จาก Task 01 เพื่อให้ fixture/upsert fields ตรงกัน
- `tests/fixtures/football-live.json` — task-produced sanitized football live contract
- `tests/fixtures/football-finished.json` — task-produced sanitized football final contract
- `tests/fixtures/basketball-live.json` — task-produced sanitized basketball live contract
- `tests/fixtures/basketball-finished.json` — task-produced sanitized basketball final contract

### External Runtime References

- `https://m.aiscore.com/` — football page with Live filter
- `https://m.aiscore.com/basketball` — basketball page with Live filter
