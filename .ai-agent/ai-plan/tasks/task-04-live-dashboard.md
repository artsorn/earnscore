# Task 04: Football/Basketball Live Dashboard and Detail View

## Outcome

Cloudflare Worker หน้าเดียวแสดง live/today football และ basketball จาก D1 อย่าง responsive, แสดง score/status breakdown ตามกีฬา, อัปเดตคู่ใหม่/finished state โดยไม่เสีย expanded state และคลิกดู known/extra non-chat details ได้อย่างปลอดภัย

## Dependencies

- Task 03 list/detail/settings API contract เสร็จและ local integration validation ผ่าน

## Implementation Scope

### Required Files

- `dashboard/src/index.js` — เฉพาะ GUI HTML/CSS/client JavaScript route `/`, `loadMatches`, card/detail rendering, refresh และ settings interaction

### Allowed Files

- `dashboard/package.json` — UI validation script เฉพาะเมื่อจำเป็น
- `dashboard/wrangler.toml` — ไม่มีการแก้ตามปกติ; ใช้ได้เฉพาะ local preview configuration ที่จำเป็นและไม่ใส่ secret

### Out of Scope

- Rust crawler/sync logic
- D1 schema/migrations และ Worker ingestion/query contracts
- การแสดง chat, prediction หรือ admin system ใหม่

## Implementation Steps

1. จัด client data adapter จาก `/api/matches/live` ให้ validate/default field ก่อน render. แยก football/basketball sections หรือ tabs พร้อม count, empty/error/loading/stale states และระบุ last successful refresh ไม่ใช่เวลาที่ request เริ่ม
2. สร้าง sport-aware match card:
   - football: live minute/HT/FT, total/half score และ cards/corners เมื่อ payload มี
   - basketball: period/status/clock, total และ quarter/overtime breakdown เมื่อ payload มี
   - scheduled/new/detail-pending/finished fallback ต้องไม่ซ่อน card
   - asset URL/logo path ต้องเลือกตามกีฬาและมี fallback เมื่อรูปเสีย
3. ใช้ status mapping ที่ Task 02 ยืนยันแทนเงื่อนไข `status_id > 1 && < 8`; preserve raw status label เมื่อ mapping ไม่รู้จัก และทำ finished state ให้เห็นชัด
4. เปลี่ยน string concatenation ที่รับ upstream values ให้ผ่าน HTML escaping และ URL validation. ใช้ DOM/textContent เมื่อเหมาะสม; ห้าม render raw JSON ด้วย `innerHTML` โดยไม่ escape
5. ทำ keyed refresh/patch หรือ state restore ที่รักษา expanded match, scroll/active sport และ detail content ขณะ poll. คู่ใหม่ต้องแทรกตาม stable order; คู่ที่เปลี่ยน finished ต้อง update card โดยไม่ต้อง reload หน้า
6. ปรับ detail lazy-load ให้แสดง incidents/timeline, stats, lineups, odds, h2h และ extra non-chat sections ที่ API คืน. รองรับ shape ต่างของสองกีฬา, unavailable/partial/malformed field และ refresh active detail เมื่อ summary version เปลี่ยน
7. ผูก settings modal กับ contract Task 03: validate sync minutes/detail seconds ใน client, ส่ง auth ตามกลไกที่กำหนดโดยไม่เก็บ/แสดง token กลับ และแสดงผลสำเร็จ/ผิดพลาดแบบไม่พึ่ง `alert` อย่างเดียว. อธิบายให้ชัดว่าค่านาทีควบคุม SQLite→D1 sync ไม่ใช่ browser polling
8. ปรับ polling ให้ request ไม่ซ้อน (AbortController/in-flight guard), back off เมื่อ error และ resume ที่ cadence เหมาะกับ live display. Browser poll interval แยกจาก D1 sync interval
9. ตรวจ responsive layout, keyboard activation/focus ของ match cards/settings, semantic labels และ reduced-motion/basic accessibility โดยไม่เพิ่ม framework ใหม่

## Acceptance Criteria

- football และ basketball แสดงแยกชัดเจนพร้อม match count; match ที่ยังไม่มี detail แสดงได้และคลิกแล้วเห็น pending/not-available state
- live/HT/period/finished และ score breakdown ใช้ mapping แยกกีฬา; unknown status ไม่ถูกตีความเป็น Live/FT แบบผิดๆ
- เมื่อ API เพิ่มคู่ใหม่หรือเปลี่ยน score/status เป็น finished หน้า update ภายใน polling cycle และ expanded card เดิมยังเปิดอยู่
- detail click แสดงทุก known section ที่มีและ extra non-chat data ที่ API อนุญาต; ไม่มี chat label/data ใน DOM
- upstream names/logos/detail strings ไม่สามารถ inject HTML/script และ broken/malformed optional payload ไม่ทำ list ทั้งหน้าพัง
- settings แก้ D1 sync interval ได้ด้วย range/error feedback และข้อความ UI ไม่ทำให้สับสนกับ dashboard refresh interval
- หน้าใช้งานได้บน mobile/desktop และ card/settings ใช้ keyboard ได้

## Validation

```bash
node --check dashboard/src/index.js
cd dashboard && npx wrangler deploy --dry-run
cd dashboard && npm run d1:init
cd dashboard && npm run dev
```

Browser validation กับ local Worker ต้อง seed fixture แยกกรณี แล้วตรวจทีละ acceptance criterion:

1. football live + finished และ basketball live + finished
2. match ที่ไม่มี `match_details`
3. new match/score/final update ระหว่างเปิด expanded detail
4. partial/malformed optional detail
5. team/detail text ที่มี `<script>`, quote และ URL scheme ที่ไม่อนุญาต
6. settings interval ต่ำ/สูงเกิน range และค่าที่ valid
7. mobile viewport, keyboard-only, failed API/recovery

Reviewer ใช้ DevTools Network/Elements ยืนยันว่าไม่มี overlapping poll, expanded state คงอยู่, malicious fixture แสดงเป็น text และไม่มี request/render ของ chat; screenshot อย่างเดียวไม่พอสำหรับ state/update/security criteria

## Reference Map

### Generated Knowledge/Cache

- `.ai-agent/generated/knowledge/frontend.md` — frontend index baseline
- `.ai-agent/generated/knowledge/api.md` — list/detail/settings endpoints และ client call sites
- `.ai-agent/generated/cache/frontend-index.md` — embedded UI entrypoints
- `.ai-agent/generated/cache/api-index.md` — exact Worker/client route markers

### Exact Source Files

- `dashboard/src/index.js` — embedded HTML/CSS, match/detail renderer, settings modal และ polling
- `dashboard/schema.sql` — field meanings/default settings (read-only reference in this task)
- `dashboard/wrangler.toml` — local Worker/D1 binding (normally read-only)
- `dashboard/package.json` — preview/validation commands
- `tests/fixtures/sync-batch.json` — sanitized two-sport seed payload จาก Task 03 (read-only ใน task นี้)
