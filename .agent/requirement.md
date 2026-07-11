# REQUIREMENT: Event-Driven Live Sports Feed, Match Detail Collector และ EarnScore Dashboard

## 1. วัตถุประสงค์

ปรับปรุงระบบ EarnScore ให้ทำงานแบบ Event-driven สำหรับเก็บข้อมูลการแข่งขันฟุตบอลและบาสเก็ตบอล จากหน้า:

https://m.aiscore.com/en/
https://m.aiscore.com/basketball

ระบบต้องเฝ้ารับการเปลี่ยนแปลงของข้อมูลการแข่งขันและราคาบอลจากหน้า Live โดยมีหลักการสำคัญดังนี้:

1. เก็บเฉพาะการแข่งขันที่กำลังแข่งขันอยู่ หรือเคยถูกพบว่ากำลังแข่งขันอยู่ระหว่างที่ระบบเปิดทำงาน
2. ไม่สร้างข้อมูลการแข่งขันที่ยังไม่เริ่มจากหน้า Schedule
3. ไม่สร้างข้อมูลการแข่งขันที่จบไปแล้ว หากระบบไม่เคยพบคู่นั้นในสถานะ Live
4. เมื่อการแข่งขัน Live เปลี่ยนแปลงคะแนน เวลา สถานะ หรือราคาบอล ให้บันทึกข้อมูลใหม่ลงฐานข้อมูล
5. เมื่อพบการแข่งขัน Live เป็นครั้งแรก ให้โหลดรายละเอียดการแข่งขันให้ครบ
6. ไม่โหลดรายละเอียดเดิมซ้ำโดยไม่มีเหตุผล
7. เมื่อ Feed หยุดแล้วเปิดใหม่ ให้ตรวจเฉพาะการแข่งขันที่กำลังแข่งอยู่ก่อน Feed หยุด
8. หากการแข่งขันดังกล่าวจบไปแล้วระหว่าง Feed หยุด ให้โหลดผลการแข่งขันและรายละเอียดฉบับสุดท้ายอีกหนึ่งครั้ง
9. รูปภาพทั้งหมดต้องถูกดาวน์โหลดเข้าระบบ ห้ามใช้ URL รูปภาพจากเว็บไซต์ต้นทางในการแสดงผล
10. Dashboard ต้องแสดงคะแนน Live และหน้ารายละเอียดที่มีโครงสร้างข้อมูลและประสบการณ์ใช้งานใกล้เคียงกับต้นฉบับ

---

# 2. ขอบเขตระบบ

ระบบใหม่แบ่งเป็นส่วนหลักดังนี้:

```text
Source Feed
    │
    ▼
Feed Listener
    │
    ├── Match State Processor
    ├── Odds Event Processor
    ├── Detail Collection Queue
    ├── Asset Download Queue
    └── Recovery Manager
             │
             ▼
       Local SQLite
             │
             ├── Sync Outbox
             ▼
       Cloudflare Worker
             │
             ├── Cloudflare D1
             ├── Cloudflare R2
             ├── Live API
             └── Dashboard
```

อนุญาตให้แก้ไขหรือเขียนระบบเดิมใหม่ทั้งหมดได้ หากจำเป็นต่อความถูกต้อง ความเสถียร และการไม่โหลดข้อมูลซ้ำ

---

# 3. นิยามสถานะการแข่งขัน

ระบบต้องใช้สถานะภายในของตัวเอง ไม่ผูกกับ status code ของแหล่งข้อมูลเพียงอย่างเดียว

สถานะภายในประกอบด้วย:

```text
DISCOVERED_LIVE
LIVE
HALF_TIME
PAUSED
FINISHING
FINISHED
CANCELLED
POSTPONED
ABANDONED
RECOVERY_PENDING
FINALIZED
UNKNOWN
```

## 3.1 กติกาการสร้าง Match

ระบบสร้าง Match ในฐานข้อมูลได้เฉพาะเมื่อ:

```text
source_status เป็น Live
หรือ
พบหลักฐานว่าการแข่งขันเริ่มแล้ว เช่น มีเวลาแข่งขัน คะแนน หรือ period ปัจจุบัน
```

ระบบต้องไม่สร้าง Match ใหม่เมื่อสถานะเป็น:

```text
Scheduled
Not Started
Upcoming
Finished
Cancelled
Postponed
```

ข้อยกเว้นคือ Match ที่มีอยู่ในฐานข้อมูลอยู่แล้ว เพราะเคยถูกพบในสถานะ Live สามารถอัปเดตไปเป็น Finished, Cancelled, Postponed หรือ Abandoned ได้

---

# 4. Live Feed Listener

## 4.1 วิธีรับข้อมูล

ลำดับความสำคัญของวิธีรับข้อมูลคือ:

1. ดัก Network Response จาก XHR, Fetch หรือ WebSocket
2. Subscribe การเปลี่ยนแปลงจาก State Store ของหน้าเว็บ
3. ใช้ DOM Mutation Observer เป็น fallback
4. ใช้ polling เฉพาะเป็น watchdog เมื่อไม่มี event เป็นเวลานาน

ห้ามใช้การ reload หน้าเว็บทั้งหมดทุกครั้งที่ต้องการตรวจข้อมูล

## 4.2 Feed Session

Football Feed ต้องเปิดอยู่ใน Chrome tab เฉพาะของระบบ

Feed tab ต้อง:

* เปิดหน้า Football
* เลือกตัวกรอง Live
* ตรวจสอบว่ากีฬาเป็น Football
* ตรวจสอบว่าตัวกรองปัจจุบันเป็น Live
* ไม่เปลี่ยน tab ของผู้ใช้
* ไม่ปิด tab ที่ระบบไม่ได้สร้าง
* เชื่อมต่อใหม่อัตโนมัติเมื่อหน้าเว็บ reload หรือ WebSocket หลุด

## 4.3 Event ที่ต้องรองรับ

ระบบต้องแปลง event จากต้นทางเป็น event ภายในดังนี้:

```text
FEED_CONNECTED
FEED_DISCONNECTED
FEED_HEARTBEAT
MATCH_DISCOVERED_LIVE
MATCH_SCORE_CHANGED
MATCH_CLOCK_CHANGED
MATCH_PERIOD_CHANGED
MATCH_STATUS_CHANGED
MATCH_ODDS_CHANGED
MATCH_REMOVED_FROM_LIVE
MATCH_FINISHED
```

ทุก event ต้องมีอย่างน้อย:

```text
event_id
source_event_id
match_id
sport_id
event_type
source_timestamp
received_at
payload_hash
payload
feed_session_id
```

## 4.4 การป้องกัน Event ซ้ำ

Event ต้องมี idempotency key ซึ่งประกอบด้วย:

```text
match_id
event_type
source_timestamp
payload_hash
```

หากได้รับ event ที่มีข้อมูลเหมือนเดิมทุกประการ ห้ามสร้างประวัติใหม่ซ้ำ

หาก source ไม่มี timestamp ให้ใช้:

```text
match_id
event_type
normalized_payload_hash
```

---

# 5. การเก็บราคาบอล

## 5.1 ขอบเขตราคาบอล

เก็บราคาบอลทุก market ที่มีสำหรับ Match ที่กำลัง Live เช่น:

```text
1X2
Asian Handicap
Over/Under
Moneyline
Draw No Bet
Both Teams to Score
Correct Score
ครึ่งแรก
ครึ่งหลัง
ตลาดย่อยอื่นที่ต้นทางมี
```

ระบบต้องรองรับ bookmaker หลายรายโดยไม่ hard-code ชื่อ bookmaker

## 5.2 เงื่อนไขการเก็บ

เก็บราคาบอลเฉพาะเมื่อ:

* Match อยู่ในสถานะ Live
* Match เคยอยู่ในสถานะ Live และกำลังอยู่ในขั้นตอน final reconciliation
* ราคาใหม่แตกต่างจากราคาปัจจุบัน

ไม่เก็บราคาเมื่อ:

* Match ยังไม่เริ่ม
* Match จบแล้วและ Finalization เสร็จสมบูรณ์
* เป็นข้อมูลราคาซ้ำกับค่าล่าสุด
* ไม่สามารถระบุ Match ID ได้อย่างถูกต้อง

## 5.3 Current Odds และ Odds History

ต้องแยกข้อมูลเป็นสองส่วน:

### `odds_current`

เก็บราคาล่าสุดของแต่ละ market และ bookmaker

Primary key ตัวอย่าง:

```text
match_id
bookmaker_id
market_type
period
selection_key
line_value
```

### `odds_history`

เก็บประวัติทุกครั้งที่ราคาเปลี่ยน

ข้อมูลอย่างน้อย:

```text
id
match_id
bookmaker_id
market_type
period
selection_key
line_value
odds_value
previous_odds_value
is_live
source_timestamp
received_at
payload_hash
```

เมื่อราคาเปลี่ยน ต้องทำภายใน transaction:

1. Insert `odds_history`
2. Upsert `odds_current`
3. Update `matches.last_odds_event_at`

---

# 6. การเก็บ Score และสถานะ Live

ตาราง `matches` ต้องเก็บ snapshot ล่าสุด ได้แก่:

```text
match_id
sport_id
competition_id
home_team_id
away_team_id
home_score
away_score
home_period_scores
away_period_scores
match_clock
period
source_status
internal_status
is_live
started_at
finished_at
last_feed_event_at
last_score_event_at
last_odds_event_at
detail_state
recovery_state
finalized_at
```

ทุกครั้งที่ Score เปลี่ยน ให้:

1. บันทึกค่าล่าสุดลง `matches`
2. เพิ่มประวัติใน `match_state_history`
3. ส่ง event ไป Dashboard
4. ห้าม trigger การโหลด Match Detail ใหม่ หาก Detail ครบแล้ว

---

# 7. Match Detail Collector

## 7.1 ข้อมูลที่ต้องดึง

เมื่อพบ Match Live เป็นครั้งแรก ให้ดึงข้อมูลต่อไปนี้:

```text
Overview
Odds
H2H
Lineups
Stats
Incidents หรือ Timeline
ข้อมูลลีก
ข้อมูลทีม
ข้อมูลสนาม
ข้อมูลผู้ตัดสิน
ข้อมูลผู้เล่น
ข้อมูลอันดับตาราง หากมี
ข้อมูลการแข่งขันย้อนหลังที่อยู่ใน H2H
```

ไม่ต้องเก็บ:

```text
Chat
Comment
Message
User-generated discussion
Advertisement
Tracking data
```

## 7.2 การโหลดครั้งแรก

เมื่อได้รับ `MATCH_DISCOVERED_LIVE`:

1. สร้าง Match ในฐานข้อมูล
2. ตรวจ `match_detail_sections`
3. สร้าง Detail Job เฉพาะ section ที่ยังไม่มี
4. เปิด detail tab ตาม concurrency limit
5. ดึงข้อมูลแต่ละ section
6. ดาวน์โหลดรูปภาพ
7. บันทึกข้อมูล
8. ปิด detail tab
9. Mark section ว่า completed

## 7.3 การไม่โหลดรายละเอียดซ้ำ

แต่ละ Match ต้องมีสถานะราย section:

```text
PENDING
LOADING
COMPLETED
EMPTY_CONFIRMED
FAILED_RETRYABLE
FAILED_PERMANENT
FINAL_REFRESH_PENDING
FINAL_COMPLETED
```

ตัวอย่างตาราง:

```text
match_detail_sections
- match_id
- section_name
- status
- content_hash
- first_loaded_at
- last_loaded_at
- final_loaded_at
- attempt_count
- last_error
```

ห้ามโหลด section ซ้ำเมื่อ:

```text
status = COMPLETED
และ
Match ยังไม่เข้าเงื่อนไข Final Recovery
```

หากบาง section โหลดสำเร็จและบาง sectionล้มเหลว ให้ retry เฉพาะ section ที่ล้มเหลว ห้ามโหลด section ที่สำเร็จแล้วใหม่ทั้งหมด

## 7.4 Retry Policy

ใช้ exponential backoff เช่น:

```text
ครั้งที่ 1: 10 วินาที
ครั้งที่ 2: 30 วินาที
ครั้งที่ 3: 2 นาที
ครั้งที่ 4: 5 นาที
ครั้งที่ 5: 15 นาที
```

ต้องมี jitter เพื่อไม่ให้หลาย Match retry พร้อมกัน

เมื่อเกินจำนวน retry ที่กำหนด ให้ mark `FAILED_PERMANENT` แต่ต้องเปิดให้ Recovery Manager สั่ง retry ได้ภายหลัง

---

# 8. การจัดการรูปภาพ

## 8.1 รูปที่ต้องดาวน์โหลด

ดาวน์โหลดรูปที่เกี่ยวข้องกับข้อมูลที่เก็บ เช่น:

```text
โลโก้ลีก
ธงประเทศ
โลโก้ทีม
รูปผู้เล่น
รูปโค้ช
รูปสนาม
รูปผู้ตัดสิน
รูปประกอบการแข่งขันที่จำเป็น
```

## 8.2 ห้ามเก็บ URL ต้นทาง

ห้ามบันทึก URL รูปจาก AiScore ลงตารางหลัก

URL ต้นทางอนุญาตให้มีได้เฉพาะใน memory ของ Download Job และต้องถูกทิ้งหลังดาวน์โหลดสำเร็จหรือจบการ retry

ฐานข้อมูลเก็บเพียง:

```text
asset_id
asset_type
owner_type
owner_id
local_path
storage_key
content_hash
mime_type
file_size
width
height
downloaded_at
uploaded_at
```

## 8.3 ขั้นตอนดาวน์โหลด

1. รับ URL ภายใน Asset Job
2. ดาวน์โหลดลงไฟล์ชั่วคราว
3. ตรวจ HTTP status
4. ตรวจ Content-Type
5. ตรวจว่าเป็นไฟล์ภาพจริง
6. คำนวณ SHA-256
7. ตรวจ duplicate จาก hash
8. เปลี่ยนชื่อไฟล์แบบ atomic
9. บันทึก local path
10. ส่งเข้า Internal Asset Storage หรือ R2
11. ลบ URL ต้นทางออกจาก job payload

รูปที่มี hash เหมือนกันต้องใช้ไฟล์เดียวกันได้

## 8.4 โครงสร้างไฟล์

ตัวอย่าง:

```text
data/assets/
├── competitions/
├── countries/
├── teams/
├── players/
├── coaches/
├── venues/
└── referees/
```

ชื่อไฟล์ต้องไม่ขึ้นกับชื่อที่ต้นทางส่งมา เช่น:

```text
{asset_type}/{owner_id}/{content_hash}.{extension}
```

## 8.5 การใช้งานบน Dashboard

Dashboard ห้ามเรียกรูปจากโดเมนต้นทาง

ต้องเรียกจาก:

```text
/assets/{asset_id}
```

หรือจาก internal R2 domain ของระบบเท่านั้น

---

# 9. Feed Heartbeat และการตรวจจับ Feed หยุด

Feed Listener ต้องบันทึก heartbeat อย่างน้อยทุก 5 วินาที

ตาราง `feed_sessions` ต้องมี:

```text
session_id
sport_id
started_at
last_heartbeat_at
disconnected_at
stopped_at
stop_reason
browser_target_id
status
```

ถือว่า Feed ขาดการเชื่อมต่อเมื่อ:

* ไม่มี heartbeat เกิน 20 วินาที
* Chrome tab ถูกปิด
* DevTools WebSocket หลุด
* หน้าเว็บเปลี่ยนออกจาก Football Live
* source store หาย
* Network listener หยุดรับข้อมูล
* browser process หยุดทำงาน

ก่อน reconnect ต้องบันทึก Match ทุกคู่ที่ยังมี:

```text
internal_status = LIVE
```

ให้เป็น:

```text
recovery_state = RECOVERY_PENDING
```

---

# 10. Recovery หลัง Feed กลับมาทำงาน

เมื่อระบบเริ่มใหม่หรือ Feed reconnect ห้ามโหลดรายละเอียดทุก Match ใหม่

ระบบต้องเลือกตรวจเฉพาะ Match ที่เข้าเงื่อนไข:

```text
เคยเป็น Live
ยังไม่ Finalized
และ
Feed หยุดก่อนทราบผลสุดท้าย
```

## 10.1 Match ยังแข่งขันอยู่

หาก Match ยัง Live:

1. อัปเดต Score, Clock, Period และ Odds ล่าสุด
2. เปลี่ยน `recovery_state` เป็น `RESUMED_LIVE`
3. ตรวจ section status
4. โหลดเฉพาะ section ที่ยังไม่ completed
5. ห้ามโหลด section ที่ completed แล้ว
6. กลับไปรับ event ตามปกติ

## 10.2 Match จบระหว่าง Feed หยุด

หาก Match จบแล้ว:

1. อัปเดต Score สุดท้าย
2. อัปเดตสถานะ Finished
3. บันทึกเวลาจบ
4. โหลด Match Detail ฉบับสุดท้ายอีกหนึ่งครั้ง
5. โหลดเฉพาะ section ที่เปลี่ยนหลังจบหรือจำเป็นต้อง finalize
6. บันทึก final content hash
7. เปลี่ยนสถานะเป็น `FINALIZED`
8. ห้ามโหลด Match นี้อีกหลัง Finalization สำเร็จ

Final refresh ต้องดึงอย่างน้อย:

```text
Overview
Odds ปิดตลาดหรือราคาสุดท้าย
Stats ฉบับสุดท้าย
Incidents หรือ Timeline ฉบับสุดท้าย
Lineups หากก่อนหน้านี้ยังไม่มี
ผลการแข่งขันและคะแนนแยกช่วง
```

H2H ไม่จำเป็นต้องโหลดซ้ำ หากเคย completed แล้วและไม่มีเหตุผลว่าข้อมูลเปลี่ยน

## 10.3 Match หาไม่พบ

หาก Match ไม่พบหลัง Feed กลับมา:

1. ตรวจจาก Match Detail URL โดยตรง
2. Retry ตาม recovery schedule
3. ตรวจสูงสุดภายใน recovery grace period เช่น 6 ชั่วโมง
4. หากพบว่า Finished ให้ Finalize
5. หากพบว่า Cancelled, Postponed หรือ Abandoned ให้บันทึกสถานะนั้น
6. หากยังไม่ทราบสถานะ ให้ mark `UNKNOWN_TERMINAL`
7. ห้ามสร้าง Match ใหม่จากหน้า Finished เพื่อทดแทน

---

# 11. กฎการไม่โหลดข้อมูลซ้ำ

ระบบต้องรับประกันกฎต่อไปนี้:

1. Feed reconnect ไม่ทำให้โหลดรายละเอียดทุกคู่ใหม่
2. Score event ไม่ทำให้โหลด Detail
3. Odds event ไม่ทำให้โหลด Detail
4. การ reload หน้า Live ไม่ทำให้โหลด Detail
5. การ restart โปรแกรมไม่ทำให้โหลด Detail ที่ completed แล้ว
6. การเปลี่ยน dataset หรือ sync รอบใหม่ไม่ทำให้โหลด Detail
7. โหลดซ้ำได้เฉพาะ:

   * section เดิมล้มเหลว
   * section เดิมยังไม่ครบ
   * Match จบระหว่าง Feed หยุด
   * ผู้ดูแลสั่ง Force Refresh
8. Final Refresh ทำได้สูงสุดหนึ่งครั้งต่อ Match ต่อ finalization version
9. ทุก Detail Job ต้องมี unique key:

```text
match_id
section_name
load_phase
```

โดย `load_phase` มีค่า:

```text
INITIAL
RETRY
FINAL
MANUAL
```

---

# 12. Database Schema ที่แนะนำ

ตารางหลัก:

```text
sports
competitions
teams
players
coaches
venues
referees
matches
match_state_history
odds_current
odds_history
match_details
match_detail_sections
match_incidents
match_statistics
match_lineups
match_h2h
assets
asset_links
feed_sessions
feed_events
detail_jobs
asset_jobs
recovery_jobs
sync_outbox
settings
```

## 12.1 `feed_events`

ใช้เป็น append-only event log:

```text
id
event_key
session_id
match_id
sport_id
event_type
source_timestamp
received_at
payload_hash
payload_json
processed_at
processing_error
```

ต้องมี unique index ที่ `event_key`

## 12.2 `recovery_jobs`

```text
id
match_id
reason
status
previous_feed_session_id
scheduled_at
started_at
completed_at
attempt_count
last_error
```

## 12.3 `sync_outbox`

ทุก transaction ที่เปลี่ยนข้อมูลต้องสร้าง outbox event ใน transaction เดียวกัน เพื่อป้องกันข้อมูลเปลี่ยนแล้วแต่ไม่ถูก sync

---

# 13. Dashboard Live Page

## 13.1 รายการ Live

หน้า Dashboard ต้องแสดงเฉพาะ Match ที่:

```text
is_live = true
```

ข้อมูลที่แสดง:

```text
ลีก
ประเทศ
โลโก้ลีก
ทีมเหย้า
ทีมเยือน
โลโก้ทีม
คะแนนปัจจุบัน
คะแนนแยกครึ่งหรือแยกช่วง
เวลาการแข่งขัน
Period
สถานะ Live
ราคาบอลหลักล่าสุด
เวลาที่ข้อมูลอัปเดตล่าสุด
สถานะ Feed
```

เมื่อ Match จบ:

* อัปเดตคะแนนสุดท้ายทันที
* แสดงสถานะ Finished ชั่วคราวตามเวลาที่กำหนด เช่น 1–5 นาที
* จากนั้นนำออกจากหน้า Live
* Match ยังคงอยู่ในฐานข้อมูลและเปิดดูผ่านหน้า History ได้

## 13.2 Real-time Update

แนะนำให้ Backend ส่งข้อมูลไป Dashboard ด้วย:

```text
Server-Sent Events
หรือ
WebSocket
```

Polling ใช้เป็น fallback เท่านั้น

Dashboard ต้องอัปเดตเฉพาะ Match card ที่เปลี่ยน ห้าม reload หน้าใหม่ทั้งหน้า

---

# 14. Match Detail Dashboard

เมื่อคลิก Match ให้เปิดหน้า:

```text
/matches/{match_id}
```

หน้า Detail ต้องประกอบด้วย:

```text
Match Header
Overview
Odds
H2H
Lineups
Stats
Timeline หรือ Incidents
```

## 14.1 Match Header

แสดง:

```text
ลีก
เวลาแข่งขัน
สนาม
สถานะ
ทีมเหย้า
ทีมเยือน
โลโก้ทีม
คะแนน
คะแนนครึ่งแรก
คะแนนเต็มเวลา
Period ปัจจุบัน
```

## 14.2 Overview

แสดงข้อมูลภาพรวมและ Timeline ของการแข่งขัน เช่น:

```text
ประตู
ใบเหลือง
ใบแดง
เปลี่ยนตัว
VAR
Penalty
เริ่มและจบครึ่ง
เหตุการณ์สำคัญ
```

## 14.3 Odds

แสดง:

```text
ราคาก่อนแข่ง หากมีจาก Initial Detail
ราคา Live ปัจจุบัน
ประวัติการเปลี่ยนราคา
Asian Handicap
Over/Under
1X2
Bookmaker
เวลาเปลี่ยนราคา
```

ต้องแยก Current Odds และ Odds Movement อย่างชัดเจน

## 14.4 H2H

แสดง:

```text
ผลงานการพบกัน
จำนวนชนะ
จำนวนเสมอ
จำนวนแพ้
คะแนนรวม
ผลการแข่งขันย้อนหลัง
Asian Handicap result
Over/Under result
```

## 14.5 Lineups

แสดง:

```text
แผนการเล่น
ตัวจริง
ตัวสำรอง
ตำแหน่งผู้เล่น
หมายเลขเสื้อ
รูปผู้เล่น
โค้ช
ผู้เล่นบาดเจ็บหรือขาดหาย หากมีข้อมูล
```

## 14.6 Stats

แสดงสถิติที่ต้นทางมี เช่น:

```text
Possession
Shots
Shots on Target
Corners
Fouls
Yellow Cards
Red Cards
Offsides
Passes
Dangerous Attacks
Attacks
Saves
```

ต้องรองรับ stat ชนิดใหม่โดยไม่ต้องแก้ schema ทุกครั้ง สามารถมีทั้ง normalized fields และ raw JSON สำรองได้

---

# 15. ข้อกำหนดด้าน Layout

เป้าหมายคือให้หน้าตา ลำดับข้อมูล responsive behavior และ interaction ใกล้เคียงหน้าต้นฉบับมากที่สุด แต่ต้องเขียน component, HTML, CSS และ JavaScript ขึ้นใหม่

ห้าม:

* Copy source code จากเว็บไซต์ต้นทาง
* Hotlink รูปจากเว็บไซต์ต้นทาง
* ฝังหน้าเว็บไซต์ต้นทางผ่าน iframe
* Proxy HTML ต้นทางมาแสดงตรง ๆ
* ใช้โลโก้หรือเครื่องหมายการค้าโดยไม่ได้รับอนุญาต

อนุญาตให้ใช้หน้าเว็บไซต์ต้นทางเป็น visual reference สำหรับ:

```text
Information hierarchy
Tab arrangement
Spacing
Card structure
Responsive layout
Score presentation
Odds table presentation
Timeline layout
Stats comparison layout
```

ต้องสร้าง Visual Regression Test ด้วย Playwright โดยมี reference screenshot สำหรับ:

```text
Desktop
Tablet
Mobile
Live match
Finished match
Match with incomplete detail
Match without lineup
```

---

# 16. API ที่ต้องมี

```text
GET /api/live/matches
GET /api/live/events
GET /api/matches/{match_id}
GET /api/matches/{match_id}/overview
GET /api/matches/{match_id}/odds
GET /api/matches/{match_id}/odds/history
GET /api/matches/{match_id}/h2h
GET /api/matches/{match_id}/lineups
GET /api/matches/{match_id}/stats
GET /api/matches/{match_id}/incidents
GET /api/assets/{asset_id}
GET /api/feed/status
```

Admin API:

```text
POST /api/admin/feed/restart
POST /api/admin/matches/{match_id}/retry-missing
POST /api/admin/matches/{match_id}/force-finalize
POST /api/admin/assets/{asset_id}/retry
GET  /api/admin/recovery-jobs
GET  /api/admin/detail-jobs
```

---

# 17. Feed Status และ Monitoring

ระบบต้องแสดงสถานะ:

```text
Connected
Disconnected
Reconnecting
Stale
Wrong Page
Wrong Sport
Source Changed
Browser Unavailable
```

Metrics ที่ต้องมี:

```text
จำนวน Live matches
จำนวน Feed events ต่อนาที
จำนวน Odds changes ต่อนาที
เวลา event delay
จำนวน Detail jobs
จำนวน Detail failures
จำนวน Recovery jobs
จำนวน Asset download failures
เวลาตั้งแต่ heartbeat ล่าสุด
เวลาตั้งแต่ odds event ล่าสุด
```

หาก source structure เปลี่ยนจน extract ไม่ได้ ต้อง:

1. หยุดบันทึกข้อมูลผิด
2. เปลี่ยน Feed เป็น `SOURCE_CHANGED`
3. เก็บ diagnostic snapshot
4. เก็บ sanitized HTML/state/network sample
5. แจ้ง error ที่อ่านเข้าใจได้
6. ห้าม mark Match เป็น Finished เพียงเพราะ extract ไม่สำเร็จ

---

# 18. Concurrency และ Rate Control

ต้องตั้งค่าได้:

```text
detail_concurrency
asset_download_concurrency
recovery_concurrency
request_delay_ms
max_retries
recovery_grace_period
feed_stale_timeout
```

ค่าเริ่มต้นแนะนำ:

```text
detail_concurrency = 3
asset_download_concurrency = 5
recovery_concurrency = 2
feed_stale_timeout = 20 วินาที
recovery_grace_period = 6 ชั่วโมง
```

Match เดียวกันห้ามมี Initial Detail Job และ Final Detail Job ทำงานพร้อมกัน

---

# 19. ความถูกต้องและความปลอดภัยของข้อมูล

1. ทุกการเขียนข้อมูลสำคัญต้องอยู่ใน transaction
2. ใช้ WAL mode สำหรับ SQLite
3. รองรับโปรแกรมถูก kill กลางงาน
4. Job ที่ค้างสถานะ LOADING ต้องถูก reclaim หลัง lease หมดอายุ
5. ห้ามลบข้อมูล Match เมื่อ Match หายจากหน้า Live
6. ห้ามเปลี่ยน Finished กลับเป็น Live เว้นแต่มีหลักฐานชัดเจนจาก source
7. เก็บ source timestamp แยกจาก received timestamp
8. Event เก่ากว่าค่าปัจจุบันห้าม overwrite ข้อมูลใหม่
9. Finalized Match ต้อง immutable ยกเว้น Admin Force Refresh
10. Log ห้ามมี token, cookie หรือข้อมูล authentication

---

# 20. Migration จากระบบเดิม

AI Agent ต้อง:

1. วิเคราะห์ schema และ code เดิมก่อนแก้ไข
2. สร้าง migration โดยไม่ลบข้อมูลที่มีอยู่
3. แปลง `match_details` เดิมเป็น section-based detail
4. แยก Odds ออกจาก raw payload
5. เพิ่ม feed event log
6. เพิ่ม recovery state
7. เพิ่ม asset storage
8. รักษา Match ID เดิม
9. ทำ migration ให้รันซ้ำได้อย่างปลอดภัย
10. สำรองฐานข้อมูลก่อน migration

ไม่จำเป็นต้องรักษา architecture เดิม หาก architecture ใหม่ตอบโจทย์ได้ดีกว่า แต่ต้องรักษาข้อมูลเดิมเท่าที่สามารถทำได้

---

# 21. Automated Tests

ต้องมี Unit Test และ Integration Test อย่างน้อยสำหรับ:

1. ไม่สร้าง Scheduled Match
2. ไม่สร้าง Finished Match ที่ไม่เคย Live
3. สร้าง Match เมื่อพบ Live
4. Odds ซ้ำไม่ถูกบันทึกซ้ำ
5. Odds เปลี่ยนถูกเพิ่มใน History
6. Score เปลี่ยนโดยไม่โหลด Detail ซ้ำ
7. Feed reconnect โดยไม่โหลด Detail ซ้ำ
8. Match ยัง Live หลัง reconnect
9. Match จบระหว่าง Feed หยุด
10. Final Detail ถูกโหลดเพียงครั้งเดียว
11. โหลดเฉพาะ Detail section ที่ขาด
12. Asset ถูกดาวน์โหลดลงระบบ
13. ไม่มี URL ต้นทางในฐานข้อมูล
14. รูปซ้ำใช้ asset เดียวกัน
15. Event เก่าไม่ overwrite Event ใหม่
16. Job lease recovery
17. Browser tab ownership
18. Source structure mismatch
19. Dashboard real-time update
20. API authorization
21. Database migration จาก schema เดิม
22. Program restart ระหว่าง Detail Job
23. Program restart ระหว่าง Asset Download
24. Program restart ระหว่าง Finalization

---

# 22. Acceptance Criteria

งานถือว่าเสร็จเมื่อผ่านเงื่อนไขทั้งหมดดังนี้:

## Feed

* เปิด Football Feed และรับเฉพาะการแข่งขัน Live
* ไม่มี Scheduled Match ใหม่ในฐานข้อมูล
* ไม่มี Finished Match ใหม่ที่ไม่เคยถูกพบ Live
* Score และ Odds เปลี่ยนตาม event โดยไม่ต้อง reload หน้า
* Feed reconnect อัตโนมัติได้

## Detail

* Match Live ใหม่ถูกโหลด Overview, Odds, H2H, Lineups และ Stats
* Match ที่ Detail ครบแล้วไม่ถูกโหลดซ้ำเมื่อ Score หรือ Odds เปลี่ยน
* Retry เฉพาะ section ที่ขาด
* Match ที่จบระหว่าง Feed หยุดถูก Final Refresh หนึ่งครั้ง
* Match ที่ Finalized แล้วไม่ถูกโหลดซ้ำ

## Asset

* รูปถูกดาวน์โหลดลงระบบก่อนใช้งาน
* Dashboard ไม่เรียกรูปจาก URL ต้นทาง
* ฐานข้อมูลไม่มี URL รูปต้นทาง
* รูปซ้ำถูก deduplicate

## Dashboard

* แสดงคะแนน Live ปัจจุบัน
* อัปเดต Score และ Odds โดยไม่ reload ทั้งหน้า
* คลิก Match แล้วเปิดรายละเอียดได้
* มี Overview, Odds, H2H, Lineups และ Stats
* รองรับ Mobile และ Desktop
* Layout และ interaction ใกล้เคียง reference
* ใช้เฉพาะข้อมูลและ asset จากระบบ EarnScore

## Recovery

* ปิด Feed ขณะแข่งขันแล้วเปิดใหม่ได้
* Match ที่ยังแข่งอยู่ทำงานต่อโดยไม่โหลด Detail ซ้ำ
* Match ที่จบไปแล้วถูกโหลดผลสุดท้าย
* ไม่มีการไล่โหลดรายละเอียด Match เก่าทั้งหมด
* ไม่มีข้อมูลซ้ำจาก restart หรือ reconnect

---

# 23. Definition of Done

ก่อนส่งมอบ AI Agent ต้องแนบ:

```text
1. Architecture diagram
2. Database ER diagram
3. Migration files
4. รายการไฟล์ที่เปลี่ยน
5. Unit test results
6. Integration test results
7. ตัวอย่าง Feed event
8. ตัวอย่าง Odds history
9. ตัวอย่าง Recovery หลัง Feed หยุด
10. Screenshot Dashboard บน Desktop และ Mobile
11. หลักฐานว่าไม่มี remote image URL ในฐานข้อมูล
12. หลักฐานว่า Detail ไม่ถูกโหลดซ้ำ
13. คู่มือการติดตั้งและใช้งาน
14. คู่มือแก้ปัญหา Chrome และ Feed
15. Rollback procedure
```

ห้ามถือว่างานเสร็จเพียงเพราะ compile ผ่าน ต้องทดสอบด้วย Feed จริงหรือ fixture ที่จำลองลำดับเหตุการณ์ครบถ้วน

** คุณสามารถใช้  mcp ของ chrome-devtools สำหรับดึงข้อมูล ดูโครงสร้างต่างๆ และใช้ทำงานต่างๆได้
