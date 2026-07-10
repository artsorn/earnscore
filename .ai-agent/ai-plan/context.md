# Planning Context

## Current Requirement Interpretation

Requirement รอบนี้เป็นการซ่อม implementation ที่ผ่านแผนเดิมแล้ว ไม่ใช่ขยาย feature ใหม่ทั่วไป โดยมี observable failures สามกลุ่ม:

1. ข้อมูลผิด/relations ผิดเมื่อเปิด feed — ต้องกลับไปยืนยัน source runtime ด้วย Chrome DevTools และสร้าง fixtures จาก shape จริง
2. ข้อมูลถูกอ่านเร็วเกินไป — ต้องมี minimum delay + readiness/stability predicates และ match-specific validation
3. ลบ `local.db` แล้วยังเห็นข้อมูลเดิม — ต้องแยก local SQLite lifecycle ออกจาก D1/dashboard lifecycle ด้วย dataset generation และ no-cache behavior

คำว่า “new tab เพื่อเก็บข้อมูลพร้อมๆ กัน” ตีความเป็น crawler-created Chrome targets: dedicated list tab ต่อ sport session และ bounded detail/feed target pool เพื่อเก็บหลาย match พร้อมกัน โดยไม่ใช้ tab ผู้ใช้ร่วมกันและไม่สร้าง tabs แบบไร้ขีดจำกัด

## Current Project Map

- `src/main.rs` เป็น monolithic Rust entrypoint (~2,000 lines) มี adapters, target selection, SQLite migrations/upserts, D1 sync, CDP router/event loop, iframe detail helper และ unit tests
- `Cargo.toml` มี `tokio`, `tokio-tungstenite`, `reqwest`, `rusqlite`, `serde`, `uuid`, `rand` ซึ่งเพียงพอสำหรับ target orchestration/generation โดยควรหลีกเลี่ยง dependency ใหม่ถ้าไม่จำเป็น
- `dashboard/src/index.js` รวม Worker routes และ embedded dashboard; `/api/matches/live` อ่าน D1 และไม่มี concept ของ local DB generation
- `dashboard/schema.sql` และ `dashboard/migrations/0001_full_payload.sql` เป็น schema ปัจจุบัน; entity PK ยังเป็น upstream ID และไม่มี `dataset_id`
- fixtures ปัจจุบันมี football/basketball live/finished และ sync batch แต่เป็น simplified shapes; ยังไม่มี target-list, Live-filter readiness หรือ real detail hydration fixtures

## Evidence from Current Source

- `FootballAdapter::extract_state_js` อ่าน `football/home` หรือ `home`; `BasketballAdapter` อ่าน `basketball` หรือ `basketball/player` และเลือก `matchesData_* | matches | list` โดยไม่มี source-version/readiness metadata
- `activate_live_js()` หา element ที่ text เท่ากับ `live` และ click แต่ caller เรียกเฉพาะเมื่อ extracted state เป็น null; state ที่มีข้อมูลแต่เป็น filter อื่นจึงผ่านได้
- initial event ถูก queue ทันทีหลัง script injection; navigation รอ fixed 5 วินาที, iframe detail รอ fixed 3 วินาทีหลัง load และ timeout 15 วินาที
- `get_websocket_url()` ใช้ matching tab ที่มีอยู่ก่อน แล้ว fallback ไป generic tab; ไม่มี target owner/session ID
- detail helper อยู่ใน list tab และอ่าน module ตาม `sportId`; response ยืนยันเพียงว่ามี `matchId`, แต่ lifecycle ของ iframe/store reuse ยังไม่แยก target
- CLI log `cli.db_path` ตาม input ไม่ resolve absolute path/DB identity. Dashboard อ่าน D1 จึงเป็นคนละ persistence layer กับ file ที่ผู้ใช้ลบ

## Source Discovery Rules

- Task 05 Coder ต้องใช้ Chrome DevTools MCP กับ `https://m.aiscore.com/` และ `https://m.aiscore.com/basketball` ขณะเลือก All และ Live เพื่อเปรียบเทียบ DOM active marker, network responses, Vue/Nuxt store และ detail page hydration
- Prefer network response contract เป็น primary source เมื่อ response body/IDs เชื่อม relations ได้; store/DOM เป็น readiness/fallback เท่านั้น. ถ้าต้องใช้ store ให้ระบุ exact module path และ invariant ที่ fixture/tests ยืนยัน
- ต้องสร้าง sanitized fixtures จาก actual shape โดยลดข้อมูลเหลือ minimum reproducible fields และตัด chat. ห้าม commit full production payload, token, cookie หรือ personal/browser data
- status IDs และ score indices ต้องแยกต่อกีฬาและยืนยันด้วย live + finished examples; ห้ามใช้ mapping เดิมโดยไม่มี evidence

## Dataset Generation Decision

- SQLite file แต่ละไฟล์มี persistent `dataset_id` UUID ใน settings/metadata; สร้างครั้งเดียวเมื่อ DB ใหม่และคงเดิมตลอดอายุ file
- normalized rows และ sync payload ทุกชนิดต้องระบุ `dataset_id`. D1 เก็บ generation บน rows และมี active dataset metadata
- เมื่อ sync generation ใหม่ครั้งแรก Worker เปลี่ยน active generation แบบ atomic; list/detail APIs filter active generation จึงไม่แสดง unmatched old rows
- Entity PK เดิมอาจคงไว้ได้หาก upsert เปลี่ยน `dataset_id` ของ rows ที่เห็นใน generation ใหม่ และ filter ซ่อน rows ที่ไม่ถูกส่งใหม่. Migration design ต้องพิสูจน์ behavior นี้กับ overlapping IDs และ old-only IDs ก่อนเลือก composite key
- สอง CLI processes ที่เปิด DB เดียวกันต้องอ่าน generation เดียวกันภายใต้ transaction. DB ต่าง path คือคนละ generation และต้อง log/warn เพื่อให้ operator เห็น
- DB unlink/replacement ขณะรันต้องตรวจด้วย canonical path + file identity/generation probe ก่อน persistence/sync. ห้ามใช้ connection/claimed rows จาก generation เก่าหลัง detect replacement

## Readiness and Concurrency Decisions

- minimum navigation/detail delay เป็น CLI-configurable milliseconds แต่ success ต้องขึ้นกับ predicate: correct URL, document ready, Live active marker, expected store/response present และ snapshot stable อย่างน้อยสอง probes
- timeout ต้องคืน typed failure/retry ไม่ persist empty/partial payload. Logs ระบุ target ID, sport, match ID, stage และ elapsed time
- list target หนึ่งตัวต่อ process; detail targets มี bounded concurrency ด้วย semaphore/queue. Target ที่สร้างต้อง track owner/session ID และปิดใน success, timeout, cancellation และ reconnect paths
- detail save ต้องตรวจ requested match ID == hydrated match ID และ relation to current dataset/sport; response จาก match ก่อนหน้าต้อง discard

## D1 and Dashboard Decisions

- D1 เป็น durable remote store จึงไม่ควรถูกลบอัตโนมัติเมื่อ local file หาย; correctness มาจาก active-generation filtering
- `/api/sync` ต้องรับ/validate generation, activate safely และ acknowledge generation เดียวกับ batch. Rust mark synced เฉพาะ acknowledgement ที่ตรงกัน
- `/api/matches/live` response ควรมี dataset ID, generation activation/freshness timestamps และ empty state เมื่อ generation ใหม่ยังไม่มี sport data
- `/api/matches/detail` ต้อง join/validate match กับ active generation ไม่ใช่ lookup `match_id` อย่างเดียว
- API/client fetch ใช้ `Cache-Control: no-store`/`cache: 'no-store'`; UI แสดง source generation/freshness เพื่อแยก “ไม่มีข้อมูลใหม่” จาก “โหลดล้มเหลว”

## Constraints

- Current runtime classification: LARGE, detail level high, maximum 8 task files
- Task 01–04 เดิมมีสถานะ Passed; active repair uses Task 05–08 เพื่อให้รวมทั้งหมดไม่เกิน 8
- implementation ใช้ Rust เป็นหลักและคง JavaScript Worker/UI ตามโครงสร้างเดิม
- ไม่อ่าน runtime logs/history ใน repair mode false; plan อาศัย compact package, generated knowledge และ targeted current sources เท่านั้น
- implementation scope ของ task ต้องไม่รวม `.ai-agent/**`, root `AGENTS.md`, `.gitignore` หรือ agent state

