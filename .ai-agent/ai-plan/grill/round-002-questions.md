# Planner Grill — Round 002

## Q1: Full rewrite ต้องรักษา legacy data, Match IDs และ API routes ถึงระดับใด
**Why this matters:** คำตอบ Q3 อนุญาต breaking changes แต่ requirement บังคับไม่ลบข้อมูลและรักษา Match ID ขณะที่ Q5 เลือก additive versioned API จึงต้องกำหนด cutover boundary ให้ไม่ขัดกัน

1. **Rewrite internals ได้ทั้งหมด แต่ migrate ข้อมูลและ Match IDs แบบไม่ทำลาย พร้อมคง legacy routes ชั่วคราวจน versioned API ผ่าน cutover** [AI RECOMMENDED]
   - Explanation: รักษาข้อบังคับด้านข้อมูลและ rollback โดยไม่ผูก architecture ใหม่กับ code เดิม หลัง verification จึงถอด legacy routes ใน release แยกได้
   - Example: DB ใหม่รักษา `match_id=123`, `/api/v1/...` เปิดใช้งาน และ route เดิมเป็น compatibility alias จนจบ cutover
2. **Rewrite internals แต่ต้องรักษาข้อมูล, Match IDs และ legacy API contracts ผ่าน compatibility adapters ต่อเนื่อง**
   - Explanation: ลด regression ต่อ consumers เดิมมากที่สุด แต่เพิ่มภาระดูแล adapters และอาจจำกัด response model ใหม่ระยะยาว
   - Example: Dashboard เก่ายังอ่าน response shape เดิมได้ ขณะที่ Dashboard ใหม่ใช้ `/api/v1/...`
3. **เก็บ legacy DB เป็น read-only archive แต่ import แบบ best-effort และอนุญาตสร้าง Match IDs ใหม่**
   - Explanation: ทำ schema ใหม่ได้ง่ายขึ้นแต่ทำลาย external references และขัดข้อกำหนดรักษา Match ID จึงต้องแก้ requirement พร้อม migration mapping ชัดเจน
   - Example: Match เดิม `123` กลายเป็น `9001` และมีตาราง `legacy_id_map` สำหรับค้นย้อนหลัง
4. **เริ่มระบบใหม่แบบ clean slate หลัง backup โดยไม่ import ข้อมูลและไม่รักษา routes เดิม**
   - Explanation: ลดงาน migration มากที่สุดแต่เป็น destructive cutover ที่ขัด requirement หลายข้อและ rollback มีเพียงการกลับไปใช้ระบบเก่า
   - Example: Production เปิด DB ว่างและมีเฉพาะ Match ที่พบ Live หลังวัน cutover

**Custom:** Describe a different choice and the result you expect.
