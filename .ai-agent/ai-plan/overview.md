# Implementation Plan: AiScore Football/Basketball Live Capture

## Goal

ปรับระบบเดิมให้รัน Rust CLI แยกเป็น `football` และ `basketball` กับ Chrome tab ที่เปิดหน้า Live ค้างไว้, เก็บรายการแข่งขันและรายละเอียดที่ AiScore เปิดเผยให้ดึงได้โดยไม่เก็บ chat, อัปเดต SQLite เมื่อข้อมูลเปลี่ยนหรือมีคู่ใหม่/จบการแข่งขัน, ส่งเฉพาะข้อมูลที่เปลี่ยนไป Cloudflare D1 ตามช่วงเวลาที่ตั้งค่าได้ และแสดง Live dashboard/รายละเอียดจาก Worker ให้ใกล้เคียงข้อมูลต้นฉบับ

## Current Baseline and Gaps

- `src/main.rs` มี subcommand สองกีฬา, อ่าน Vuex state, upsert SQLite, เปิด detail ผ่าน iframe และมี background D1 sync แล้ว แต่ผูกกับชื่อ module/field บางชุด จึงพลาดข้อมูลเมื่อโครงสร้าง state ต่างกันหรือมีคู่ใหม่
- ตารางปัจจุบันเก็บเฉพาะ field ที่ normalize แล้ว ทำให้ข้อมูลต้นทางที่ไม่อยู่ใน `incidents/stats/lineups/odds/h2h` สูญหาย และยังไม่มี migration path ที่ชัดเจนสำหรับฐานข้อมูลเดิม
- `sync_worker` สอง process สามารถทำงานทับกัน, teams/competitions ที่เปลี่ยนแต่ไม่มี dirty match อาจไม่ถูกส่ง และค่า interval ที่แก้ใน D1 ไม่ได้ควบคุม Rust process อย่างครบวงจร
- `/api/matches/live` ใช้ inner join กับ `match_details` จึงซ่อนคู่ใหม่ที่ detail ยังไม่พร้อม; UI ผูก status/logo/score layout กับสมมติฐานบางอย่างและ render ข้อมูลจากต้นทางโดยตรง

## Target Data Flow

1. Rust CLI หนึ่ง process ต่อกีฬาเลือก Chrome page target ของกีฬาตัวเองและยืนยันว่าอยู่ที่หน้า Live
2. CDP network/runtime events กระตุ้นการ reconcile; periodic reconciliation เป็น safety net สำหรับคู่ใหม่หรือ event ที่ subscriber ไม่จับ
3. ตัว extractor normalize field หลักสำหรับ query/UI พร้อมเก็บ sanitized raw JSON ของ list/detail โดยตัด chat ออกก่อนเขียน SQLite
4. SQLite upsert เปลี่ยน dirty state เฉพาะเมื่อ content เปลี่ยน และเก็บสถานะ final ที่ตรวจพบจาก payload ล่าสุด
5. sync lease ทำให้มี uploader เดียวต่อ SQLite file; payload แบบ bounded batch ถูก upsert แบบ idempotent ที่ Worker/D1 และ mark synced หลัง acknowledgement เท่านั้น
6. Worker API คืน match แม้ detail ยังไม่พร้อม, คืนรายละเอียดแบบ typed + extra sections และคืนค่า sync interval ให้ Rust นำไปใช้รอบถัดไป
7. Dashboard poll API, แยก football/basketball, รักษาการ์ดที่เปิดอยู่ขณะ refresh และแสดงสถานะ/คะแนน/detail โดยไม่แสดง chat

## Task Order

1. `task-01-schema-and-data-contract.md` — สร้าง schema/migration และ data contract ที่รองรับ normalized + sanitized raw payload
2. `task-02-crawler-live-capture.md` — ทำ live-page discovery, event reconciliation, detail extraction และ SQLite change detection สำหรับสอง session
3. `task-03-d1-sync-and-worker-api.md` — ทำ concurrent-safe sync, D1 ingestion/query/settings contract
4. `task-04-live-dashboard.md` — ปรับหน้า Live และ detail UI ให้ใช้ contract ใหม่อย่างปลอดภัย

Task 2 ขึ้นกับ Task 1; Task 3 ขึ้นกับ Task 1–2; Task 4 ขึ้นกับ Task 3. เนื่องจาก `src/main.rs` และ `dashboard/src/index.js` เป็น monolithic entrypoints งานที่ใช้ไฟล์ซ้ำต้องแก้เฉพาะ section ที่ระบุใน task และรักษา contract จาก task ก่อนหน้า

## Global Acceptance Criteria

- รัน `football` และ `basketball` พร้อมกันกับ SQLite file เดียวได้ โดยแต่ละ process ยึดเฉพาะ Chrome tab/URL ของกีฬาตัวเอง
- initial reconciliation เก็บทุก match ที่แสดงใน Live state/response ณ ตอนนั้น; เมื่อมีคู่ใหม่ คะแนน/เวลา/event/status เปลี่ยน หรือคู่จบ ข้อมูลล่าสุดถูก upsert โดยไม่ต้อง restart
- normalized fields ที่ UI/query ต้องใช้ยังอยู่ครบ และ full list/detail payload ที่รองรับในปัจจุบันถูกเก็บเป็น JSON หลังลบ chat-related keys/sections; ไม่มี chat ใน SQLite, sync payload, D1 หรือ UI
- SQLite เดิมและ D1 เดิมอัปเกรดแบบไม่ลบข้อมูล; fresh schema และ migrated schema มีรูปแบบปลายทางเดียวกัน
- D1 sync เป็น idempotent, ไม่ mark row ว่า synced เมื่อ request ล้มเหลว, รองรับสอง CLI process โดยไม่ส่งซ้ำจาก race และใช้ช่วงนาทีที่กำหนดใน D1/settings
- Worker list API ไม่ซ่อนคู่ที่ detail ยังไม่พร้อม และรายงาน scheduled/live/finished ตามข้อมูลล่าสุดของแต่ละกีฬา
- Dashboard แสดง football และ basketball live score, สถานะ final, score breakdown ที่มีอยู่ และรายละเอียดเมื่อคลิก โดย refresh แล้วไม่ทำลาย expanded state

## Explicit Non-goals

- ไม่ scrape, persist, sync, expose หรือ render chat/comment/live-chat
- ไม่เพิ่มกีฬาอื่นนอก football และ basketball
- ไม่พยายาม bypass login, paywall, CAPTCHA หรือ access control ของ AiScore
- ไม่สร้างระบบย้อนหลัง/analytics ใหม่ที่ requirement ไม่ได้ร้องขอ; raw payload มีไว้รักษาข้อมูลต้นทางและรองรับ detail view

