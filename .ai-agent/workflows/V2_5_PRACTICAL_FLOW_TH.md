# code-agent-token-lite v2.5: Practical Flow

คู่มือใช้งานจริงสำหรับโปรเจกต์ว่างและโปรเจกต์เดิม โดยเน้นผลลัพธ์ที่ตรวจสอบได้และลด token ที่ไม่จำเป็น

## 1. Flow ทำงานอย่างไร

1. **Planner** อ่าน requirement/project map และสร้าง task ที่มี scope, acceptance criteria และ validation
2. **Coder** แก้เฉพาะ Allowed Files ของ task ปัจจุบัน
3. **Fast Reviewer** ตรวจ task diff; เมื่อ fail Coder ใช้ reviewer summary ซ่อมใน session เดิม
4. **Final Reviewer** ตรวจ test, architecture และ integration หลังทุก task ผ่าน

หลักการคือใช้ model แข็งแรงกับการตัดสินใจที่เกิดน้อยครั้ง และ model เร็วกับงานแคบที่เกิดหลายครั้ง

## 2. Blank Project

```bash
mkdir my-cli
bash /path/to/code-agent-token-lite/install.sh "$PWD/my-cli"
cd my-cli
.ai-agent/bin/aia doctor
```

installer จะ `git init` แต่ไม่ commit แทนผู้ใช้ และสร้าง:

```text
.agent/requirement.md             requirement รอบปัจจุบัน
.ai-agent/config/user.env         project config
.ai-agent/ai-plan/tasks/          Planner outputs
.ai-agent/generated/runtime/      context, diff, verdict, logs, token usage
```

## 3. Existing Project

```bash
bash /path/to/code-agent-token-lite/install.sh /path/to/existing-project
cd /path/to/existing-project
.ai-agent/bin/aia doctor
.ai-agent/bin/aia refresh
```

ตรวจ `git status --short` ก่อนเริ่ม หากมี implementation diff เก่า ScopeGuard อาจ block เพราะแยกไม่ได้ว่าเป็นงานของ task ใด ควร commit/stash งานเดิม หรือกำหนด task scope ให้ครอบคลุมโดยตั้งใจ

## 4. Requirement ที่ช่วยลด Repair

เขียน `.agent/requirement.md` ด้วยโครงนี้:

```md
# Goal
ผลลัพธ์ปลายทางที่ผู้ใช้ต้องได้

## Current Problem
อาการปัจจุบันและตัวอย่างที่ผิด

## Constraints
- technology/version
- สิ่งที่ห้ามเปลี่ยน
- compatibility/security/performance

## Acceptance Criteria
- พฤติกรรมที่พิสูจน์ได้ข้อ 1
- พฤติกรรมที่พิสูจน์ได้ข้อ 2

## Validation
- command หรือ manual scenario สำหรับแต่ละข้อ
```

หลีกเลี่ยงคำกว้างอย่าง “ทำให้ดี” หรือ “แก้ที่เกี่ยวข้องทั้งหมด” เพราะทำให้ Planner เดา scope และ Coder ค้นหาเพิ่ม

## 5. เลือก Model Profile

### เสถียรและ setup น้อย

```bash
# .ai-agent/config/user.env
AIA_PROFILE=balanced
```

ทุก role ใช้ Codex `gpt-5.6-sol`; ความประหยัดมาจาก adaptive effort, compact context และ session reuse

### แนะนำสำหรับคุณภาพต่อ Token

```bash
AIA_PROFILE=hybrid-efficient
```

```text
Planner        Codex gpt-5.6-sol
Coder          AGY Gemini 3.5 Flash (Low)
Fast Reviewer  AGY Gemini 3.5 Flash (Medium)
Final Reviewer Codex gpt-5.6-sol
```

- Planner ที่แข็งแรงลด task กำกวมและ repair round
- Flash Low ทำงานแคบซ้ำหลาย task
- Flash Medium ตรวจ Coder ด้วย reasoning สูงกว่าเล็กน้อย
- Final Reviewer ที่แข็งแรงใช้ครั้งเดียวตรวจผลรวม

### Codex Sol/Luna

```bash
AIA_PROFILE=codex-split
```

ใช้เมื่อ account รัน `gpt-5.6-luna` ได้จริง หากยังไม่ expose Luna ให้ใช้ `balanced` หรือ `hybrid-efficient` อย่าใช้ production loop ทดสอบ entitlement

### AGY ทั้งหมด

```bash
AIA_PROFILE=agy-efficient
agy models
.ai-agent/bin/aia config
```

## 6. Preflight ก่อนเสีย Token

```bash
.ai-agent/bin/aia doctor
.ai-agent/bin/aia config
git status --short
```

hybrid profile ต้องพบทั้ง `codex` และ `agy` และ effective AGY model ต้องเป็น display name ไม่ใช่ `gpt-*`

ทดสอบ `agy` โดยตรงได้ด้วย:

```bash
agy \
  --model 'Gemini 3.5 Flash (Low)' \
  --new-project \
  --print-timeout 1m \
  --dangerously-skip-permissions \
  --print 'Reply with exactly: OK'
```

## 7. Plan

flow ตรง:

```bash
.ai-agent/bin/aia refresh
.ai-agent/bin/aia plan
find .ai-agent/ai-plan/tasks -maxdepth 1 -name 'task-*.md' -print | sort
```

แต่ละ task ควรมี `Status`, `Allowed Files`, `Acceptance Criteria`, validation ที่เจาะจง และ reference map ที่ไม่สั่งอ่านทั้ง repo

สำหรับ schema/auth/destructive behavior หรือ requirement ขัดกัน ใช้ Grill flow:

```bash
.ai-agent/bin/aia plan grill-start
$EDITOR .ai-agent/ai-plan/grill/answers.md
.ai-agent/bin/aia plan grill-next
```

เมื่อครบให้ใส่ `PLAN APPROVED` แล้วรัน:

```bash
.ai-agent/bin/aia plan freeze
```

งานเล็กชัดเจนไม่ควรใช้ grill เพราะเพิ่ม Planner rounds

## 8. Run และ Monitor

```bash
MAX_ROUNDS=3 .ai-agent/bin/aia run
```

อีก terminal:

```bash
.ai-agent/bin/aia monitor watch
```

หรือ:

```bash
.ai-agent/bin/aia progress
.ai-agent/bin/aia status
```

เมื่อ Fast Reviewer fail ระบบสร้าง `reviewer-summary.md`, เพิ่ม context เฉพาะเมื่อจำเป็น และ reuse session ภายใน task เดิม

เมื่อ Final Reviewer fail ค่า default จะหยุด หากต้องการให้ Planner แก้ final finding โดยตรง:

```bash
.ai-agent/bin/aia run \
  --final-reviewer-fail-mode plan-code \
  --final-reviewer-fix-rounds 1
```

## 9. ตัวอย่างครบ: Blank Node CLI

```bash
mkdir greet-cli
bash /path/to/code-agent-token-lite/install.sh "$PWD/greet-cli"
cd greet-cli
```

`.agent/requirement.md`:

```md
# Goal
สร้าง Node.js CLI ที่ทักทายชื่อจาก argument แรก

## Constraints
- ไม่เพิ่ม npm dependency
- ใช้ CommonJS และ Node.js 20+

## Acceptance Criteria
- `node src/cli.js Nana` แสดง `Hello, Nana!`
- ไม่มีชื่อให้พิมพ์ usage ทาง stderr และ exit 2
- `node --test` ผ่าน

## Validation
- `node src/cli.js Nana`
- `node --test`
```

เลือก profile และรัน:

```bash
printf '%s\n' 'AIA_PROFILE=hybrid-efficient' >> .ai-agent/config/user.env
.ai-agent/bin/aia doctor
.ai-agent/bin/aia config
.ai-agent/bin/aia plan
.ai-agent/bin/aia run
```

ตรวจผล:

```bash
node src/cli.js Nana
node --test
.ai-agent/bin/aia task-diff
.ai-agent/bin/aia tokens
```

ไฟล์ source/test ที่ยัง untracked จะปรากฏใน reviewer patch เป็น `new file mode` พร้อมเนื้อหา จึง review ได้โดยไม่มี initial commit

## 10. Requirement รอบใหม่

แก้ `.agent/requirement.md` แล้ว:

```bash
.ai-agent/bin/aia restart
.ai-agent/bin/aia refresh
.ai-agent/bin/aia plan
.ai-agent/bin/aia run
```

`restart` ไม่ลบ source, config หรือ token archive

## 11. Token Controls ที่แนะนำ

```bash
USE_PERSISTENT_SESSIONS=true
PERSISTENT_SESSION_SCOPE=task
INLINE_CONTEXT_PACKAGE=false
FAST_REVIEW_UNIT_TEST=false
FAST_REVIEW_ARCHITECTURE=false
FINAL_REVIEW_UNIT_TEST=true
FINAL_REVIEW_ARCHITECTURE=true
MAX_ROUNDS=3
```

ใช้ `INLINE_CONTEXT_PACKAGE=true` เฉพาะ backend ที่อ่าน workspace file ไม่ได้ สำหรับ Codex/AGY local flow ให้เป็น `false`

อย่าปิด Final Reviewer ก่อนแก้ task quality เพราะ repair หลายรอบมักแพงกว่าการวางแผนดีครั้งเดียว

## 12. Troubleshooting

### AGY model ไม่ถูกต้อง

```bash
agy models
.ai-agent/bin/aia config
```

ใช้ชื่อจาก `agy models` หรือ alias ที่รองรับ อย่าใส่ `gpt-*` ใน `*_AGY_MODEL`

### AGY ไม่มี project/conversation

`AGY_NEW_PROJECT=auto` จะเติม `--new-project` เมื่อไม่มี project, conversation หรือ continue mode

### Requirement ว่าง

เติม `.agent/requirement.md`; heading หรือ HTML comment อย่างเดียวไม่นับเป็น requirement

### No task files found

รัน `.ai-agent/bin/aia plan` และตรวจ `.ai-agent/ai-plan/tasks/task-*.md`

### ScopeGuard block

```bash
git status --short
.ai-agent/bin/aia scope allowed
.ai-agent/bin/aia task-files
```

commit/stash diff เก่า หรือแก้ Allowed Files ให้ครบ ห้ามปิด ScopeGuard เพียงเพื่อให้ loop เดินต่อ

### Reviewer มองไม่เห็นไฟล์ใหม่

```bash
.ai-agent/bin/aia task-diff
rg -n 'new file mode|^\+' .ai-agent/generated/runtime/reviewer-diff.patch
```

ตรวจว่าไฟล์อยู่ใน Allowed Files และไม่ใช่ agent state path

### Debug invocation

```bash
.ai-agent/bin/aia config
.ai-agent/bin/aia status
tail -n 100 .ai-agent/generated/runtime/events.log
```

อ่าน raw role logs เฉพาะตอน repair/debug เพราะ log ยาวและเพิ่ม context โดยไม่จำเป็น
