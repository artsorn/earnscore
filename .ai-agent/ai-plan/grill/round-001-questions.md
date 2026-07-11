# Planner Grill — Round 001

## Q1: การส่งมอบรอบนี้ต้องรองรับ Football และ Basketball เต็มรูปแบบพร้อมกันหรือไม่
**Why this matters:** Requirement ระบุสองกีฬาแต่รายละเอียด Feed และ Acceptance Criteria เน้น Football จึงมีผลต่อ schema, adapters, fixtures, UI และขนาดงานโดยตรง

1. **รองรับทั้งสองกีฬาแบบ end-to-end ด้วย shared core และแยก sport adapters** [AI RECOMMENDED]
   - Explanation: รักษาขอบเขตตามวัตถุประสงค์และใช้กฎ admission, recovery และ idempotency ร่วมกัน โดยแยกเฉพาะการแปลง payload ของแต่ละกีฬา
   - Example: Football และ Basketball Live สร้าง event รูปแบบเดียวกัน แต่ period/score mapping ใช้ adapter คนละตัว
2. **ส่ง Football ให้ production-ready ก่อน แล้วทำ Basketball เป็น phase ถัดไปในแผนเดียวกัน**
   - Explanation: ลดความเสี่ยงรอบแรกและได้พิสูจน์ pipeline ก่อน แต่ Definition of Done ทั้งโครงการยังไม่ครบจนกว่า phase Basketball เสร็จ
   - Example: Release 1 ผ่าน Football E2E ส่วน Release 2 เพิ่ม Basketball fixtures และ dashboard states
3. **ทำ Football เต็มรูปแบบ แต่ Basketball เก็บเฉพาะ feed และ raw normalized events ในรอบนี้**
   - Explanation: ตรวจความเข้ากันได้ของแหล่ง Basketball ได้เร็ว แต่ detail, recovery และ UI ของ Basketball ยังไม่พร้อมใช้งาน
   - Example: Basketball score event เข้า `feed_events` แต่ยังไม่เปิดหน้า Match Detail
4. **จำกัดขอบเขตเป็น Football เท่านั้นและตัด Basketball ออกจาก requirement รอบนี้**
   - Explanation: ขนาดงานเล็กที่สุดแต่เป็นการเปลี่ยนวัตถุประสงค์ที่ระบุไว้และต้องแก้ acceptance/Definition of Done ให้ชัดเจน
   - Example: ระบบเปิดเฉพาะ `/en/` และไม่สร้าง Basketball adapter

**Custom:** Describe a different choice and the result you expect.

## Q2: ระบบควรเป็นเจ้าของ Chrome และ Feed tabs ด้วยรูปแบบใด
**Why this matters:** รูปแบบ ownership กำหนดความปลอดภัยของ tab ผู้ใช้ ความสามารถในการ reconnect การเก็บ target ID และความเสถียรของ browser tests

1. **ใช้ Chrome process/profile เฉพาะที่ระบบเป็นเจ้าของและควบคุมผ่าน CDP** [AI RECOMMENDED]
   - Explanation: แยก session ของ collector จาก browsing ของผู้ใช้และทำ target ownership/restart ได้ชัดเจน โดยยังใช้ visible Chrome เมื่อ source ต้องการ
   - Example: ระบบสร้าง profile `earnscore-feed`, เปิด feed tabs เอง และปิดได้เฉพาะ target ที่ลงทะเบียนไว้
2. **ใช้ Chrome process เดิมของผู้ใช้แต่สร้างและลงทะเบียนเฉพาะ tabs ของระบบ**
   - Explanation: ใช้ทรัพยากรน้อยลงแต่เสี่ยงกับ profile state, extension และการปิด browser ของผู้ใช้ จึงต้องตรวจ ownership เข้มงวด
   - Example: ปิด collector แล้ว tab ส่วนตัวคงอยู่ แต่ feed tab ที่มี target ID ของระบบถูกปิด
3. **ใช้ browser extension ใน Chrome ของผู้ใช้เพื่อดัก network/state**
   - Explanation: ผูกกับ browser ที่ใช้งานจริงได้ดีแต่เพิ่มงาน permission, install/update, security review และ compatibility ของ extension
   - Example: Extension ส่ง normalized events ไป local service โดยไม่เปิด CDP port ภายนอก
4. **ใช้ standalone headless browser ที่ระบบจัดการทั้งหมด**
   - Explanation: เหมาะกับ server automation แต่ behavior อาจต่างจาก requirement ที่กล่าวถึง Chrome tab และอาจเจอ source/browser detection ต่างจาก headed mode
   - Example: Worker เปิด headless Chromium สอง pages และไม่มีหน้าต่าง browser บน desktop

**Custom:** Describe a different choice and the result you expect.

## Q3: ขอบเขตการแทนที่ระบบเดิมและ compatibility boundary ที่ห้ามทำลายควรเป็นแบบใด
**Why this matters:** คำตอบนี้กำหนด forbidden modules, migration/cutover, rollback และความเสี่ยงที่ Match IDs, routes หรือข้อมูลเดิมจะเสียหาย

1. **ใช้ additive migration และวางโมดูลใหม่หลัง IDs/routes เดิม โดยห้ามแตะโมดูล unrelated** [AI RECOMMENDED]
   - Explanation: รักษาข้อมูลและ consumers เดิมก่อน แล้วค่อย cut over ด้วย compatibility adapters หรือ versioned routes หลัง shadow verification
   - Example: เพิ่ม section tables และ outbox โดย `match_id` กับ endpoint เดิมยังตอบได้ระหว่างเปลี่ยนระบบ
2. **สร้างระบบ v2 คู่ขนานด้วย DB/API/Dashboard ใหม่แล้ว cut over ครั้งเดียว**
   - Explanation: แยก architecture ใหม่ได้สะอาดแต่ต้องทำ dual-sync, reconciliation และ rollback ข้ามสองระบบอย่างรัดกุม
   - Example: `/api/v2/live/matches` รันคู่กับ API เดิมจนผลเทียบกันผ่านก่อนสลับ traffic
3. **แทนที่ collector และ backend ได้ แต่ต้องรักษา Dashboard/API contracts เดิมทุกจุด**
   - Explanation: เปิดทาง rewrite ฝั่งข้อมูลโดยไม่กระทบผู้ใช้ แต่เพิ่ม adapter burden และอาจจำกัด schema/real-time behavior ใหม่
   - Example: Dashboard เดิมไม่เปลี่ยน response shape แม้ backend เปลี่ยนเป็น event/outbox architecture
4. **อนุญาต full rewrite และ import ข้อมูลเดิมแบบ best-effort โดยยอมรับ breaking changes**
   - Explanation: ให้อิสระสูงสุดแต่เสี่ยง data loss, route regression และ rollback ยาก จึงต้องอนุมัติการหยุดระบบและข้อยกเว้นการรักษาข้อมูลอย่างชัดเจน
   - Example: deploy แอปใหม่พร้อม schema ใหม่และประกาศว่า legacy API ใช้งานไม่ได้หลัง cutover

**Custom:** Describe a different choice and the result you expect.

## Q4: ข้อมูลการแข่งขันย้อนหลังใน H2H ควรถูกเก็บอย่างไรโดยไม่ละเมิดกฎห้ามสร้าง Finished Match ที่ไม่เคย Live
**Why this matters:** H2H ต้องมีผลย้อนหลัง แต่การใส่ผลเหล่านั้นใน canonical `matches` จะขัดกับ admission invariant และทำให้ recovery/history ปะปน

1. **เก็บเป็น non-canonical external match references ใต้ `match_h2h` และไม่สร้างแถวใน `matches`** [AI RECOMMENDED]
   - Explanation: เก็บรายละเอียด H2H ที่ต้องแสดงได้ครบพร้อม source identity แยก โดย canonical workflow เห็นเฉพาะ Match ที่เคย Live
   - Example: H2H row มีคู่ทีม คะแนน วันที่ และ external reference แต่ไม่มี canonical `match_id` ใหม่
2. **เก็บเฉพาะ aggregate H2H โดยไม่เก็บรายการผลย้อนหลังแต่ละนัด**
   - Explanation: schema ง่ายและไม่เสี่ยงสร้าง Match ผิดกฎ แต่ UI จะไม่มีรายการย้อนหลังและข้อมูล odds result รายเกมตาม requirement
   - Example: หน้า H2H แสดงชนะ 3 เสมอ 1 แพ้ 1 แต่ไม่มี match cards ย้อนหลัง
3. **อนุญาตแถว `matches` แบบ `reference_only` สำหรับ H2H และกันออกจาก feed/recovery**
   - Explanation: query ร่วมกับ Match ง่ายขึ้นแต่ทำให้ความหมายของ canonical table ไม่บริสุทธิ์และต้อง guard ทุก processor/API อย่างถาวร
   - Example: Finished H2H row อยู่ใน `matches` แต่ `reference_only=true` และห้าม enqueue detail job
4. **ไม่ persist รายการ H2H และแสดงเฉพาะข้อมูลที่อยู่ใน initial detail snapshot**
   - Explanation: ลด migration/schema แต่ค้นหา เปรียบเทียบ และ sync H2H รายการยาก รวมถึงอาจเก็บข้อมูลซ้ำใน raw JSON
   - Example: API ส่ง H2H จาก JSON blob ของ Match หลักโดยไม่มีตารางอ้างอิง

**Custom:** Describe a different choice and the result you expect.

## Q5: API, real-time, authorization และ Dashboard ควรมี release contract และ task boundary แบบใด
**Why this matters:** การตัดสินใจนี้มีผลต่อ existing flows, security tests, route regression, visual acceptance และว่าควรแบ่งงานเป็น vertical slices หรือ cutover ก้อนใหญ่

1. **ใช้ versioned additive REST กับ SSE, ป้องกัน Admin API, และส่ง backend-to-UI เป็น vertical slices** [AI RECOMMENDED]
   - Explanation: SSE เหมาะกับ server-to-dashboard updates และ REST ใช้ resync ได้ง่าย ขณะที่ vertical slices ทำให้ตรวจ Live, Detail, Recovery และ UI contract ทีละเส้นทางได้
   - Example: Live slice ส่ง snapshot ผ่าน REST แล้ว patch card ผ่าน SSE; admin restart ต้องยืนยัน session/token และมี audit
2. **บังคับ authentication ทุก API และใช้ WebSocket สำหรับ real-time แบบสองทาง**
   - Explanation: เหมาะกับ Dashboard ส่วนตัวและ future commands แต่เพิ่ม connection/auth state, reconnect complexity และ test surface
   - Example: ผู้ใช้ต้อง login ก่อนดู Live และ WebSocket เดียวรับ score พร้อมส่ง admin command
3. **แทนที่ routes เดิมด้วย unversioned REST กับ SSE และ cut over Dashboard พร้อมกัน**
   - Explanation: ลด compatibility layer แต่เสี่ยง breaking change และยากต่อ rollback หาก API หรือ UI ส่วนใดส่วนหนึ่งล้มเหลว
   - Example: `/api/live/matches` เปลี่ยน response shape ใน release เดียวกับ Dashboard ใหม่
4. **ส่ง Feed/DB/API ก่อนและเลื่อน Dashboard/visual regression ไป phase หลัง**
   - Explanation: ลด scope ของ backend release แรกแต่ยังไม่ผ่าน Acceptance Criteria และทำให้ contract defects อาจถูกพบช้ากว่า
   - Example: รอบแรกตรวจ API ด้วย fixtures เท่านั้นและหน้า Live เดิมยังไม่อัปเดตแบบ real-time

**Custom:** Describe a different choice and the result you expect.
