# Planning Context

## Requirement Interpretation

คำว่า “ดึงข้อมูลทั้งหมดเท่าที่จะสามารถดึงได้” ใช้แนวทางสองชั้น:

- normalize field ที่ระบบต้อง query/render ได้แก่ sport, competition, teams, match time/status, scores และ detail กลุ่ม incidents, stats, lineups, odds, h2h
- เก็บ payload list/detail ที่เหลือเป็น sanitized raw JSON เพื่อไม่ทิ้ง field ที่ AiScore เพิ่มหรือเปลี่ยน โดยต้องตัด key/section/endpoint ที่เป็น chat ก่อน persistence

คำว่า “event เปลี่ยนก็บันทึก” หมายถึงเก็บ latest state และทำ row dirty เมื่อเนื้อหาเปลี่ยนจริง ไม่จำเป็นต้องสร้าง event-history warehouse. สถานะ finished ต้องเป็น latest state ที่ persist/sync/render ได้ แม้หลัง match หลุดจาก Live list; crawler จึงต้องทำ final reconciliation ให้ match ที่เคย active

## Verified Project Map

- Rust package/entrypoint: `Cargo.toml`, `src/main.rs`
- Cloudflare Worker/API/embedded UI: `dashboard/src/index.js`
- D1 bootstrap schema: `dashboard/schema.sql`
- Worker configuration/scripts: `dashboard/wrangler.toml`, `dashboard/package.json`
- ไม่มี test directory หรือ migration directory ใน baseline

Generated project knowledge ระบุ schema/function หลักตรงกับ source ปัจจุบัน: `init_db`, `save_competitions`, `save_teams`, `save_matches`, `sync_worker`, `trigger_state_fetch`, `/api/sync`, `/api/matches/live`, `/api/matches/detail`, `/api/settings`, และ embedded `loadMatches()`

## Source-Site Observations and Discovery Boundary

- URL ต้นทางตาม requirement คือ `https://m.aiscore.com/` และ `https://m.aiscore.com/basketball`; ทั้งสองหน้ามี filter `All / Live / Finished / Schedule`
- HTML ที่อ่านได้จากภายนอกไม่ยืนยัน Vuex module, response endpoint, status mapping หรือ detail shape ที่ runtime ใช้จริง จึงห้ามวางแผนให้ hard-code ชื่อ state จากการคาดเดา
- ใน Task 2 Coder ต้องใช้ Chrome DevTools MCP/CDP กับทั้งสอง URL ขณะเลือก Live เพื่อยืนยัน target selection, network response shape, state fallback, status IDs และ detail sections. เก็บ fixture แบบย่อ/ตัดข้อมูลส่วนบุคคลเพื่อทำ regression test; ไม่บันทึก chat fixture
- ถ้า source shape ต่างระหว่างกีฬา ให้ใช้ sport adapter แยกกันและ contract กลางหลัง extraction แทน conditional ที่กระจายทั่วไฟล์

## Data and Migration Decisions

- รักษาตาราง normalized เดิมเพื่อ compatibility กับ Worker/UI
- เพิ่ม sanitized `raw_json` (และ metadata ที่จำเป็นต่อ change/sync) ให้ entity/match/detail แทนการเพิ่ม column ทุก field ของ AiScore
- SQLite migration ใน `init_db` ต้อง idempotent ด้วย schema inspection ก่อน `ALTER TABLE`; เปิด WAL, busy timeout และ transaction สั้นเพื่อให้สอง process เขียน file เดียวกันได้
- `dashboard/schema.sql` เป็น final bootstrap schema สำหรับ D1 ใหม่; migration file ใหม่ใช้กับ D1 เดิมและต้องไม่ drop/rename ตารางเดิม
- canonical JSON comparison หรือเทียบค่าที่ persist แล้วต้องทำให้ unchanged reconciliation ไม่ reset `synced`/`updated_at`
- chat exclusion เป็น allow/deny boundary ก่อน DB write: ตัด known chat keys/sections และไม่รับ network responses จาก chat endpoints. Worker ยังต้อง reject/strip chat-like top-level sections เพื่อเป็น defense in depth

## Sync and Settings Decisions

- ทั้งสอง CLI process อาจชี้ SQLite เดียวกัน แต่ต้องมี lease/claim ที่ atomic เพื่อให้ process เดียวส่ง batch ณ เวลาใดเวลาหนึ่ง
- competitions, teams, matches และ details ต้องมี dirty lifecycle ของตนเอง ไม่อาศัยเฉพาะ relation จาก dirty matches
- D1 ingestion ใช้ upsert และ bounded batches; response ระบุรายการ/จำนวนที่รับสำเร็จและ authoritative `sync_interval_mins`
- Rust อ่านค่า interval local ตอนเริ่ม และ refresh จาก acknowledgement/settings ของ Worker เพื่อให้ค่าที่แก้บน dashboard มีผลจริง; invalid/missing value ใช้ safe default และมี minimum bound ป้องกัน tight loop
- settings write ที่เปลี่ยน sync/token ต้องไม่เผย token ใน GET response/log และต้องคง auth contract ที่ Rust ใช้อยู่

## UI Decisions

- List API ใช้ `LEFT JOIN match_details` หรือไม่ join detail เพื่อให้คู่ใหม่แสดงได้ทันที
- UI ใช้ sport-aware score/status/logo renderer และแสดง fallback เมื่อ field/detail ไม่มี แทนการซ่อน match
- detail renderer แสดงกลุ่ม known ก่อน แล้วแสดง extra non-chat sections แบบ generic JSON-safe view เมื่อเหมาะสม
- escape text/attribute และ validate asset URLs จาก payload ก่อนประกอบ HTML เพื่อไม่ให้ raw upstream data กลายเป็น script injection

## Constraints

- implementation ใช้ Rust เป็นหลัก; Worker/UI ยังอยู่ใน JavaScript ตามโครงสร้างเดิม
- task size เป็น MEDIUM, detail level เป็น high, จำกัดไม่เกิน 4 tasks
- แผนนี้ไม่เพิ่ม implementation file นอก scope ที่ระบุในแต่ละ task และไม่รวม `.ai-agent/**`/agent state เป็น implementation scope
- การทดสอบ live source ต้องใช้ browser ที่เปิด remote debugging; automated tests ใช้ sanitized fixtures เพื่อไม่ผูก CI กับ availability ของ AiScore

