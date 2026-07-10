# Implementation Plan: Re-baseline AiScore Capture and Eliminate Stale Feeds

## Goal

ซ่อม crawler หลัง implementation รอบแรกให้ยึดข้อมูลจริงจาก AiScore ใหม่ตั้งแต่ source relationships, ไม่อ่าน state ก่อนหน้า Live พร้อม, เปิด Chrome targets ใหม่สำหรับ list/detail feeds ที่ต้องเก็บพร้อมกัน, รอ readiness ด้วยเงื่อนไขที่ตรวจสอบได้ และทำให้การลบ/สร้าง `local.db` ใหม่ไม่แสดงข้อมูล D1 generation เก่าบน dashboard

## Why the Current Behavior Is Wrong

- `main` ส่ง initial reconciliation ทันทีหลัง inject script และเรียก `activate_live_js()` เฉพาะเมื่อ Vuex state เป็น `null`; ถ้า state ของ All/previous page มีข้อมูลอยู่แล้ว crawler จะบันทึกข้อมูลนั้นโดยไม่เคยยืนยันว่า Live filter active
- adapters เดาชื่อ Vuex modules/collections หลายชื่อ แต่ fixtures ปัจจุบันเป็น simplified contract ไม่ใช่หลักฐานจาก runtime network/store จริง จึงยังพิสูจน์ relation match → competition/team/status/detail ไม่ได้
- detail helper ใช้ hidden iframe ใน tab หลัก, รอคงที่ 3 วินาทีหลัง `onload` และอ่าน detail module ทันที; feed ที่ hydrate ช้าหรือ state ค้างจาก match ก่อนหน้าจึงให้ข้อมูลว่าง/ผิด match ได้
- `get_websocket_url()` เลือก AiScore tab เดิมหรือ hijack generic tab แทนการสร้าง/ถือ ownership ของ targets ใหม่ ทำให้สอง sessions หรือหลาย detail feeds แย่ง page state กันได้
- การลบ `local.db` ไม่ลบ Cloudflare D1; dashboard `/api/matches/live` อ่าน D1 โดยตรง จึงยังเห็น rows เก่า นอกจากนี้ process ที่ยังรันอยู่ต้องรับมือกรณี DB file ถูก unlink/recreated อย่างชัดเจน

## Completed Baseline

Task 01–04 เดิมมีสถานะ `Passed` และยังเป็นฐานที่ต้องรักษา: full-payload schema, sanitized/chat-free capture, SQLite dirty lifecycle, concurrent-safe D1 sync และ responsive Worker dashboard. งานใหม่ไม่เปิด concern เหล่านี้ซ้ำ เว้นแต่ต้องต่อ schema/protocol เพื่อแก้ stale generation และ source correctness

## Active Task Order

5. `task-05-source-contract-rebaseline.md` — ตรวจ AiScore ด้วย Chrome DevTools ใหม่และแก้ sport adapters/fixtures ให้ตรง source relationships จริง
6. `task-06-database-generation-and-reset.md` — เพิ่ม dataset generation, canonical DB identity และ recovery เมื่อ local DB ถูกลบ/สร้างใหม่
7. `task-07-dedicated-tabs-and-readiness.md` — สร้าง/ถือ ownership/ปิด Chrome targets, รอ Live/detail readiness และเก็บหลาย feeds แบบ bounded concurrency
8. `task-08-active-generation-sync-and-dashboard.md` — ส่ง generation ไป D1, activate/filter current generation และทำ dashboard ไม่แสดง cache/rows เก่า

Task 05 ต้องเสร็จก่อน Task 07. Task 06 ต้องเสร็จก่อน Task 08. Task 07 ต้องเสร็จก่อน end-to-end validation ของ Task 08. Task 05 และ Task 06 ทำคู่ขนานได้ในเชิง dependency แต่ทั้งคู่แตะ `src/main.rs` จึงควร merge ตามลำดับเพื่อเลี่ยง conflict

## Target Runtime Flow

1. CLI resolve และ log absolute SQLite path พร้อม persistent `dataset_id`; DB file ใหม่สร้าง generation ใหม่ ส่วนสอง sport sessions ที่ชี้ file เดียวกันอ่าน generation เดียวกัน
2. แต่ละ CLI สร้าง dedicated list target ของกีฬาตัวเองผ่าน Chrome DevTools และติด ownership marker; ไม่ยึด tab ผู้ใช้หรือ tab ของอีก process
3. หลัง navigate ให้ activate Live เสมอ แล้วรอ DOM/store/network readiness และ stable snapshot ก่อน persist; initial fixed delay เป็น minimum settle time ไม่ใช่หลักฐานเดียว
4. extractor ที่ยืนยันจาก Chrome runtime map match IDs ไป competition/team/status/scores อย่างถูกต้อง และ reject incomplete/cross-sport snapshots
5. detail coordinator สร้าง dedicated targets แบบจำกัดจำนวน, รอ match-specific readiness, ยืนยัน returned match ID แล้วจึง save และปิด/reuse target อย่างปลอดภัย
6. SQLite upserts ทั้งหมดติด `dataset_id`; ถ้า DB path หายหรือ inode/generation เปลี่ยน process reinitialize และหยุดส่ง generation เก่า
7. sync payload ระบุ generation. Worker activate generation ใหม่แบบ atomic และ list/detail APIs filter เฉพาะ active generation
8. dashboard ใช้ `no-store`, แสดง dataset/freshness state และแสดง empty/loading ระหว่างรอ generation ใหม่แทน rows เก่า

## Global Acceptance Criteria

- ก่อน write ครั้งแรก crawler ยืนยันว่า dedicated page อยู่ URL/กีฬา/Live filter ที่ถูกต้องและ snapshot stable; All/Finished/previous Vuex state ไม่ถูกบันทึกเป็น Live
- football และ basketball sessions สร้าง list targets คนละ target โดยไม่ hijack tab ผู้ใช้ และเปิด detail/feed targets หลายตัวตาม configured concurrency โดยข้อมูลไม่สลับ match
- readiness delay ปรับค่าได้ มี timeout/backoff และต้องใช้ predicate ที่ตรวจ DOM/store/match ID; ห้ามแก้ด้วย `sleep` อย่างเดียว
- fixtures ของสองกีฬามาจาก sanitized runtime shape ที่ Coder ตรวจด้วย Chrome DevTools และครอบคลุม list relation, live status, finished status และ detail readiness
- `local.db` ใหม่ได้ `dataset_id` ใหม่; dashboard ไม่คืน rows จาก generation ก่อน แม้ D1 ยังเก็บ rows เหล่านั้น
- สอง sessions ที่ใช้ DB path เดียวกันใช้ `dataset_id` เดียวกัน; path คนละไฟล์ถูกระบุชัดใน log และไม่ถูกรวมโดยเงียบ
- หาก DB ถูกลบขณะ process รัน ระบบ detect/reinitialize หรือหยุดพร้อม actionable error โดยไม่ sync handle/generation เก่าต่อ
- `/api/matches/live` และ `/api/matches/detail` คืนเฉพาะ active generation พร้อม `Cache-Control: no-store`; browser refresh ไม่แสดง response cache เก่า
- chat exclusion, dirty-sync correctness, settings, security และ dashboard behavior จาก Task 01–04 ยังผ่าน regression tests

## Explicit Non-goals

- ไม่ล้าง D1 history แบบ destructive เพียงเพื่อซ่อนข้อมูลเก่า; ใช้ active generation filter และแยก cleanup policy ออกจาก correctness
- ไม่ hijack/ปิด tabs ที่ crawler ไม่ได้สร้างหรือไม่ได้ถือ ownership
- ไม่ใช้ arbitrary sleep เพียงอย่างเดียวเป็น readiness strategy
- ไม่เพิ่มกีฬาอื่น, chat, login bypass, CAPTCHA bypass หรือ paid/private feeds

