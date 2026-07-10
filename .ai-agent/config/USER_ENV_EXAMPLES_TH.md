# ตัวอย่าง `.ai-agent/config/user.env`

ไฟล์นี้รวม preset สำหรับงานขนาดเล็ก กลาง และใหญ่ แยก Codex, AGY และ Hybrid

## กติกาสำคัญ

ใช้ assignment ตรงใน `user.env`:

```bash
AIA_PROFILE=hybrid-efficient
MAX_ROUNDS=3
```

อย่าใช้ `: ${NAME:=value}` เมื่อต้องการ override ค่า default ของโปรเจกต์ เพราะรูปแบบนั้นหมายถึง “ตั้งค่าเมื่อยังไม่มีค่าเท่านั้น”

ตรวจค่าหลังแก้ทุกครั้ง:

```bash
.ai-agent/bin/aia config
.ai-agent/bin/aia doctor
```

สำหรับ AGY ให้ตรวจ model ที่ account ใช้ได้:

```bash
agy models
```

## Project Size กับ Task Size

ตัว classifier วัดขนาด requirement/task ไม่ใช่จำนวนไฟล์ทั้งหมดใน repository ดังนั้น:

- repo ใหญ่แต่งานแก้ข้อความหนึ่งไฟล์ควรเป็น `SMALL`
- repo เล็กแต่งานเปลี่ยน schema, auth และ frontend พร้อมกันอาจเป็น `LARGE`
- ใช้ `TASK_SIZE_OVERRIDE=SMALL|MEDIUM|LARGE` เมื่อทราบขนาดแน่นอน
- ลบค่า `TASK_SIZE_OVERRIDE` หรือกำหนดเป็นค่าว่างเมื่ออยากให้ระบบแยกแต่ละ requirement อัตโนมัติ

ไฟล์ preset พร้อมใช้ติดตั้งอยู่ใน `.ai-agent/config/examples/`

## 1. SMALL: Codex-only

ไฟล์: `examples/small-codex.env`

เหมาะกับงาน 1-2 ไฟล์ เช่น copy, CSS, handler เล็ก หรือ test fix ใช้ Codex CLI ตัวเดียวและจำกัด Planner ให้สร้าง task เดียว

```bash
cp .ai-agent/config/examples/small-codex.env .ai-agent/config/user.env
.ai-agent/bin/aia config
```

จุดสำคัญ:

- `MAX_ROUNDS=2` ป้องกัน repair loop ยาว
- Final Reviewer ยังทำ unit test แต่ข้าม architecture review เต็มระบบ
- context cap ต่ำและค้นหาไม่เกิน 4 คำค้นหลัก

## 2. SMALL: AGY-only

ไฟล์: `examples/small-agy.env`

Planner/Final ใช้ Gemini Pro Low, Coder ใช้ Flash Low และ Reviewer ใช้ Flash Medium เหมาะกับเครื่องที่ไม่มี Codex หรืออยากลดการใช้ Codex quota

```bash
cp .ai-agent/config/examples/small-agy.env .ai-agent/config/user.env
agy models
.ai-agent/bin/aia config
```

`AGY_NEW_PROJECT=auto` ทำให้แต่ละ invocation สร้าง project เมื่อไม่มี project/conversation ที่ระบุไว้ ไม่ใช้ `AGY_CONTINUE=true` โดย default เพราะอาจ resume conversation ผิดโปรเจกต์

## 3. MEDIUM: Hybrid แนะนำ

ไฟล์: `examples/medium-hybrid.env`

เหมาะกับ feature 2-5 modules หรือ frontend/backend ที่ contract ชัดเจน:

```text
Planner        Codex gpt-5.6-sol
Coder          AGY Gemini 3.5 Flash (Low)
Reviewer       AGY Gemini 3.5 Flash (Medium)
Final Reviewer Codex gpt-5.6-sol
```

นี่เป็น preset แนะนำเมื่อทั้ง `codex` และ `agy` login แล้ว เพราะใช้ model แข็งแรงตอนวางแผนและตรวจรวม แต่ใช้ Flash ในรอบที่เกิดบ่อย

## 4. MEDIUM: Codex Sol/Luna

ไฟล์: `examples/medium-codex-split.env`

Planner/Final ใช้ Sol และ Coder/Reviewer ใช้ Luna ต้องยืนยันก่อนว่า account มี model นี้:

```bash
cp .ai-agent/config/examples/medium-codex-split.env .ai-agent/config/user.env
.ai-agent/bin/aia config
```

หาก Luna ไม่พร้อมให้ใช้ `medium-hybrid.env` หรือเปลี่ยน `CODER_MODEL`/`REVIEWER_MODEL` เป็น `gpt-5.6-sol`

## 5. LARGE: Codex เน้นคุณภาพ

ไฟล์: `examples/large-codex.env`

เหมาะกับ migration, auth, cross-module contract, architecture change หรือ feature หลาย layer ค่า context/search สูงขึ้นและเปิด Final Reviewer เต็มรูปแบบ

จุดสำคัญ:

- Planner/Final ใช้ `xhigh`
- Coder `high`, Reviewer `medium`
- ไม่ใช้ role session ข้าม task (`PERSISTENT_SESSION_SCOPE=task`)
- จำกัด Planner ไม่เกิน 8 tasks เพื่อไม่แตกงานละเอียดเกินประโยชน์

## 6. LARGE: Hybrid คุม Token

ไฟล์: `examples/large-hybrid.env`

ใช้ Codex Sol วางแผน/ตรวจท้าย, AGY Flash High เขียน code และ Gemini Pro Low review ต่อ task เหมาะกับงานใหญ่ที่ต้องการลด Codex usage แต่ยังรักษา review quality

งานใหญ่ไม่ควรใช้ Flash Low เป็น Coder โดยอัตโนมัติ เพราะจำนวน repair rounds อาจทำให้ token รวมสูงกว่า Flash High

## 7. LARGE: AGY-only

ไฟล์: `examples/large-agy.env`

ใช้ Gemini Pro High สำหรับ Planner/Final, Flash High สำหรับ Coder และ Pro Low สำหรับ Reviewer ต้องตรวจ `agy models` เพราะชื่อ/availability ขึ้นกับ account

## 8. Auto-size สำหรับหลาย Requirement

ถ้าโปรเจกต์มีทั้งงานเล็กและใหญ่ ให้เริ่มจาก preset medium แล้วเปิด auto:

```bash
AIA_PROFILE=hybrid-efficient
TASK_SIZE_AUTO=true
TASK_SIZE_OVERRIDE=
MAX_ROUNDS=3
INLINE_CONTEXT_PACKAGE=false
USE_PERSISTENT_SESSIONS=true
PERSISTENT_SESSION_SCOPE=task
```

ระบบจะเลือก context/search/task budgets จาก `SMALL`, `MEDIUM`, `LARGE` ตาม requirement ปัจจุบัน ดูผลได้ด้วย:

```bash
.ai-agent/bin/aia task-size
cat .ai-agent/generated/runtime/task-size.json
```

## 9. Override เฉพาะ Role

ไม่จำเป็นต้องเปลี่ยนทั้ง profile:

```bash
AIA_PROFILE=hybrid-efficient

# ใช้ model แรงขึ้นเฉพาะ Coder
CODER_AGY_MODEL=Gemini 3.5 Flash (High)

# ใช้ Codex review task ที่เสี่ยง
REVIEWER_CLI=codex
REVIEWER_MODEL=gpt-5.6-sol
REVIEWER_LEVEL=high
```

environment ที่ส่งหน้าคำสั่งมี precedence สูงสุด เหมาะกับ override ครั้งเดียว:

```bash
TASK_SIZE_OVERRIDE=LARGE CODER_LEVEL=high .ai-agent/bin/aia run
```

## 10. เลือก Preset อย่างรวดเร็ว

| ลักษณะงาน | Preset |
|---|---|
| copy/CSS/test fix 1-2 ไฟล์ | `small-codex.env` |
| งานเล็กและใช้ AGY เท่านั้น | `small-agy.env` |
| feature ทั่วไป 2-5 modules | `medium-hybrid.env` |
| Codex account มี Luna | `medium-codex-split.env` |
| migration/auth/architecture เน้นคุณภาพ | `large-codex.env` |
| งานใหญ่แต่ต้องลด Codex quota | `large-hybrid.env` |
| งานใหญ่และใช้ AGY เท่านั้น | `large-agy.env` |

หลังเลือก preset ให้รันตามลำดับ:

```bash
.ai-agent/bin/aia doctor
.ai-agent/bin/aia config
.ai-agent/bin/aia refresh
.ai-agent/bin/aia plan
.ai-agent/bin/aia run
```
