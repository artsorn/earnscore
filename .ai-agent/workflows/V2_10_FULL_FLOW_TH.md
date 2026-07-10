# AIA v2.4.0 Full Flow - Legacy Reference

> สำหรับ v2.5 ให้เริ่มจาก `workflows/V2_5_PRACTICAL_FLOW_TH.md` ซึ่งอัปเดต model profiles, `agy` display names, blank-project bootstrap และ token-saving defaults แล้ว เอกสารนี้คงไว้เป็น reference ของ flow v2.4

เอกสารนี้เป็นไฟล์อ้างอิงหลักของ `code-agent-token-lite` สำหรับโปรเจ็กต์ที่ติดตั้งไว้ใน `.ai-agent/`

แนวคิดของ v2.4.0 คือนอกจาก optimize context และแยก "งานตรวจหนัก" ไปอยู่ใน Final Reviewer แล้ว ยังเพิ่ม Planner Grill Me Loop สำหรับ challenge แผนก่อน freeze เป็น implementation tasks:

- Planner สร้าง/ซ่อม plan
- Coder ลงมือแก้ implementation
- Fast Reviewer ตรวจเฉพาะ task-level correctness (diff, scope, compile, obvious regression)
- Final Reviewer ตรวจ integration, unit test, architecture review, และภาพรวมทั้งโปรเจ็กต์

สิ่งที่เปลี่ยนคือ "context ที่ส่งเข้า Codex" ถูกบีบให้แคบลงอัตโนมัติ เพื่อลด token และลดการอ่านไฟล์ซ้ำ โดยให้ AI เริ่มจาก compact package ก่อนอ่าน source จริง:

- `.ai-agent/generated/runtime/task-size.txt`
- `.ai-agent/generated/runtime/context-escalation.txt`
- `.ai-agent/generated/runtime/context-manifest.txt`
- `.ai-agent/generated/runtime/context-package.md`
- `.ai-agent/generated/runtime/search-allowlist.txt`
- `.ai-agent/generated/runtime/runtime-context.md`
- `.ai-agent/generated/runtime/reviewer-summary.md`
- task file ที่ Planner ใส่ `## Reference Map`

ไฟล์ generated อื่นยังมีอยู่และยังใช้ได้ แต่ไม่ถูก preload ทั้งหมดเหมือนเดิม:

- `.ai-agent/generated/cache/codegraph-lite.md`
- `.ai-agent/generated/cache/codegraph-project.md`
- `.ai-agent/generated/knowledge/*.md`

## 0) สรุปว่าอะไรเหมือนเดิมและอะไรเพิ่ม

เหมือนเดิม:

- คำสั่งหลักยังใช้ `aia refresh`, `aia plan`, `aia run`, `aia docs`, `aia verify`
- Planner/Coder/Fast Reviewer/Final Reviewer ยังทำงาน role เดิม
- ScopeGuard, task checkpoints, repair loop, final-review fail modes, persistent sessions ยังอยู่ครบ
- task format เดิมยังใช้ได้ เช่น `Allowed Files`, `Reference Map`, `Validation Commands`

เพิ่มหรือเปลี่ยน:

- มี `task-size.txt` เพื่อแยกงานเป็น `SMALL`, `MEDIUM`, หรือ `LARGE`
- มี `context-escalation.txt` เพื่อบอกระดับ context ปัจจุบันและเหตุผลที่ escalate
- มี `context-manifest.txt` เพื่อ debug ว่า package ใส่/ตัดอะไรเข้าไปบ้าง
- มี `context-package.md` เป็น compact input หลักก่อนเข้า Codex
- มี `search-allowlist.txt` สำหรับบอกพื้นที่ค้นหาที่อนุญาตอัตโนมัติ
- มี `reviewer-summary.md` สำหรับรอบ repair แทนการพึ่ง raw reviewer logs
- knowledge ถูกโหลดแบบ lazy มากขึ้นตามประเภทงาน
- มี token guard ตัด context เกินจำเป็นก่อนยิง prompt
- มี search budget และกฎห้ามอ่าน runtime artifacts อัตโนมัติระหว่างงานปกติ
- **[v2.3]** Fast Reviewer ตรวจเฉพาะ task-level: diff, scope, compile, syntax, obvious regression
- **[v2.3]** Fast Reviewer ไม่รัน unit test และไม่ทำ architecture review โดย default
- **[v2.3]** Final Reviewer เป็นจุดเดียวที่รัน unit test และ architecture review
- **[v2.3]** config ใหม่ 4 ตัวสำหรับเปิด/ปิดแต่ละ check:
  - `FAST_REVIEW_UNIT_TEST` (default `false`)
  - `FAST_REVIEW_ARCHITECTURE` (default `false`)
  - `FINAL_REVIEW_UNIT_TEST` (default `true`)
  - `FINAL_REVIEW_ARCHITECTURE` (default `true`)

## 1) เช็กหลังติดตั้งหรืออัปเกรด

จาก root ของโปรเจ็กต์:

```bash
cd /path/to/project
.ai-agent/bin/aia version
.ai-agent/bin/aia doctor
.ai-agent/bin/aia status
```

อัปเกรดจาก package ต้นทาง:

```bash
bash /mnt/d/home/aitools/code-agent-token-lite/install.sh /path/to/project update
```

หลังอัปเกรดให้ตรวจ syntax และ manifest:

```bash
.ai-agent/bin/aia verify
bash -n .ai-agent/bin/aia
bash -n .ai-agent/scripts/agent-codegraph.sh
bash -n .ai-agent/scripts/agent-knowledge-scan.sh
bash -n .ai-agent/scripts/agent-context-build.sh
bash -n .ai-agent/scripts/agent-context-package.sh
```

ถ้าต้องการเช็กว่าโปรเจ็กต์ใช้ flow optimize แล้วหรือยัง ให้ดูไฟล์ต่อไปนี้:

```bash
test -f .ai-agent/scripts/agent-context-package.sh && echo "context-package: ok"
test -f .ai-agent/scripts/agent-task-size.sh && echo "task-size: ok"
test -f .ai-agent/generated/runtime/context-package.md && echo "runtime package: ok"
rg -n "context-package|search-allowlist|reviewer-summary" .ai-agent/AGENTS.md AGENTS.md .ai-agent/workflows/V2_10_FULL_FLOW_TH.md
```

## 2) สร้าง context ให้ AI ก่อนทำงาน

คำสั่งหลัก:

```bash
.ai-agent/bin/aia refresh
```

คำสั่งนี้ทำ 3 อย่าง:

- rebuild CodeGraph/cache indexes
- rebuild Knowledge docs
- rebuild runtime context
- rebuild compact context package

ไฟล์ที่ได้:

```text
.ai-agent/generated/cache/codegraph-project.md
.ai-agent/generated/cache/codegraph-lite.md
.ai-agent/generated/cache/project-summary.json
.ai-agent/generated/cache/symbol-index.md
.ai-agent/generated/cache/api-index.md
.ai-agent/generated/cache/schema-index.md
.ai-agent/generated/cache/frontend-index.md
.ai-agent/generated/cache/dependency-index.md
.ai-agent/generated/cache/docs-index.md
.ai-agent/generated/knowledge/architecture.md
.ai-agent/generated/knowledge/api.md
.ai-agent/generated/knowledge/database.md
.ai-agent/generated/knowledge/frontend.md
.ai-agent/generated/knowledge/documentation.md
.ai-agent/generated/runtime/runtime-context.md
.ai-agent/generated/runtime/task-size.txt
.ai-agent/generated/runtime/task-size.json
.ai-agent/generated/runtime/task-size.env
.ai-agent/generated/runtime/task-type.txt
.ai-agent/generated/runtime/inferred-allowed-area.txt
.ai-agent/generated/runtime/context-package.md
.ai-agent/generated/runtime/context-package.meta.json
.ai-agent/generated/runtime/search-allowlist.txt
```

`aia plan`, `aia run`, `aia docs`, และ `aia context` จะเรียก `graph ensure` และ `scan ensure` ให้เอง ถ้า fingerprint เปลี่ยนจะ refresh อัตโนมัติ

ลำดับการอ่านไฟล์ของ agent หลัง optimize:

1. current task
2. allowed edit files
3. `runtime-context.md`
4. `reviewer-summary.md`
5. relevant codegraph ใน `context-package.md`
6. relevant source snippets ใน `context-package.md`

agent จะไม่ preload docs, logs, history, cache หรือ generated knowledge ทุกไฟล์โดยอัตโนมัติอีกต่อไป เว้นแต่ task ต้องใช้จริง

ดู classification และ adaptive settings:

```bash
.ai-agent/bin/aia task-size
cat .ai-agent/generated/runtime/task-size.txt
cat .ai-agent/generated/runtime/task-size.json
```

ค่า task size:

- `SMALL`: text/content, single frontend section, single file, CSS/layout, copy/link/button/image
- `MEDIUM`: 2-5 related files, small API, small UI + backend integration, one limited feature
- `LARGE`: database/schema, auth/permission, multiple modules, cross-cutting backend/frontend, architecture/workflow

สำหรับ `SMALL` Planner จะพยายามสร้าง task เดียวเท่านั้น เว้นแต่มีเหตุผลทางเทคนิคชัดเจนว่าต้อง split

ตารางสรุป behavior หลัง optimize:

| Task Size | Flow การใช้งาน | Context เริ่มต้น | Escalation ปกติ | Knowledge | Search / Codegraph | เป้าหมาย token โดยทั่วไป |
|---|---|---|---|---|---|---|
| `SMALL` | เหมือนเดิม 100% | requirement, current task, allowed files, target files | ปกติไม่เกิน Level 1; ไป Level 2+ เฉพาะจำเป็นจริง | ไม่โหลดถ้าไม่จำเป็น | search 5, depth 1 | Planner ~15k, Coder ~25k |
| `MEDIUM` | เหมือนเดิม 100% | requirement, task, allowed files, target files, direct refs | ปกติเริ่มที่ Level 1-2; ขยายเมื่อ dependency หรือ validation บังคับ | โหลดเฉพาะ intent ที่เกี่ยวข้อง | search 10, depth 2 | Planner ~30k, Coder ~50k |
| `LARGE` | เหมือนเดิม 100% | requirement, task, target files, direct refs, relevant codegraph | ปกติเริ่มที่ Level 2-3; ขยายต่อเมื่อ cross-module / schema / auth กระทบหลายส่วน | โหลดเฉพาะ knowledge ที่ต้องใช้ | search 20, depth 3 | Planner ~80k, Coder ~120k |

กฎคงเดิมทุกขนาดงาน:

- Workflow ไม่เปลี่ยน: `Planner → Coder → Fast Reviewer → Repair Loop → Final Reviewer → Commit / Push`
- ไม่ต้องเปลี่ยนคำสั่งเดิม เช่น `aia plan`, `aia run`, `aia task-size`
- ระบบจะพยายามลด token ก่อนเสมอ โดยไม่ลด scope guard, reviewer checks, หรือ repair behavior
- runtime logs / token usage / old agent outputs จะไม่ถูกอ่านอัตโนมัติระหว่างงานปกติ

## 3) Config ที่เกี่ยวกับ context

ค่า default อยู่ที่:

```text
.ai-agent/config/default.env
```

ค่าเฉพาะโปรเจ็กต์ให้ใส่ที่:

```text
.ai-agent/config/user.env
```

ค่าที่ใช้บ่อย:

```bash
: ${CODEGRAPH_MODE:=smart}
: ${CODEGRAPH_DEPTH:=2}
: ${SCAN_MODE:=smart}
: ${CODEGRAPH_MAX_FILE_BYTES:=1200000}
: ${CODEGRAPH_PROJECT_MAX_LINES:=1800}
: ${CONTEXT_MODE:=minimal}
: ${TASK_SIZE_AUTO:=true}
: ${TASK_SIZE_OVERRIDE:=}
: ${SMALL_PLANNER_CONTEXT_TOKENS:=15000}
: ${SMALL_CODER_CONTEXT_TOKENS:=25000}
: ${MEDIUM_PLANNER_CONTEXT_TOKENS:=30000}
: ${MEDIUM_CODER_CONTEXT_TOKENS:=50000}
: ${LARGE_PLANNER_CONTEXT_TOKENS:=80000}
: ${LARGE_CODER_CONTEXT_TOKENS:=120000}
: ${SMALL_SEARCH_BUDGET:=5}
: ${MEDIUM_SEARCH_BUDGET:=10}
: ${LARGE_SEARCH_BUDGET:=20}
: ${SMALL_CODEGRAPH_DEPTH:=1}
: ${MEDIUM_CODEGRAPH_DEPTH:=2}
: ${LARGE_CODEGRAPH_DEPTH:=3}
: ${MAX_TASKS_FROM_PLANNER:=}
: ${TOKEN_GUARD:=true}
: ${MAX_CONTEXT_TOKENS:=50000}
: ${ALLOW_RUNTIME_LOG_READ:=false}
: ${ALLOW_HISTORY_SEARCH:=false}
: ${LAZY_KNOWLEDGE:=true}
: ${COMPACT_REVIEW:=true}
: ${SEARCH_BUDGET:=10}
: ${PLANNER_TASK_DETAIL_LEVEL:=high}
```

ความหมาย:

- `CODEGRAPH_MODE=smart` สร้าง index ทั้งโปรเจ็กต์แบบย่อ ใช้เป็นค่าแนะนำ
- `CODEGRAPH_MODE=lite` ยังสร้าง cache ครบ แต่เน้น `codegraph-lite.md` สำหรับ task/ไฟล์ที่เปลี่ยน
- `CODEGRAPH_MODE=strict` หรือ `full` เหมาะกับ final review หรือ project-wide review เพราะให้ codegraph excerpt ยาวขึ้น
- `CODEGRAPH_DEPTH=2` จำกัดการไล่ dependency/reverse reference ของ compact context
- `SCAN_MODE=smart` ใช้สร้าง generated knowledge จาก cache indexes
- `CODEGRAPH_MAX_FILE_BYTES` จำกัดขนาดไฟล์ที่ scanner อ่าน
- `CODEGRAPH_PROJECT_MAX_LINES` จำกัดความยาว `codegraph-project.md` ในโหมด smart/lite
- `CONTEXT_MODE=minimal` ใส่ context เท่าที่จำเป็นใน runtime
- `CONTEXT_MODE=balanced` ใส่ knowledge excerpts เพิ่ม เหมาะกับ Planner
- `CONTEXT_MODE=strict` หรือ `full` ใส่ project codegraph excerpt เพิ่ม เหมาะกับ final review
- `TASK_SIZE_AUTO=true` เปิด automatic task-size classification
- `TASK_SIZE_OVERRIDE=` ตั้งเป็น `SMALL`, `MEDIUM`, หรือ `LARGE` เพื่อบังคับขนาดงาน
- `SMALL_*`, `MEDIUM_*`, `LARGE_*` ใช้คุม context budget, search budget, และ codegraph depth ตามขนาดงาน
- `MAX_TASKS_FROM_PLANNER` ใช้ override จำนวน task สูงสุดที่ Planner ควรสร้างจาก adaptive default
- `TOKEN_GUARD=true` ให้ trim context อัตโนมัติก่อนส่งเข้า Codex
- `MAX_CONTEXT_TOKENS=50000` คือเพดาน input context โดยประมาณ
- `LAZY_KNOWLEDGE=true` โหลด knowledge เฉพาะที่เกี่ยวกับงาน
- `COMPACT_REVIEW=true` ใช้ `reviewer-summary.md` สำหรับ repair
- `SEARCH_BUDGET=10` จำกัดจำนวน search operation ที่ agent ควรใช้ต่อรอบ
- `ALLOW_RUNTIME_LOG_READ=false` และ `ALLOW_HISTORY_SEARCH=false` ป้องกันการอ่าน runtime artifacts อัตโนมัติ

## 4) Flow เริ่มงานใหม่

เช็ก worktree ก่อน:

```bash
git status --short
.ai-agent/bin/aia status
.ai-agent/bin/aia task-files
```

ถ้างานเก่าจบแล้วให้ commit/deploy ก่อน จากนั้นล้าง runtime:

```bash
.ai-agent/bin/aia restart
```

เขียน requirement:

```bash
mkdir -p .agent
nano .agent/requirement.md
```

ตัวอย่าง requirement:

```md
# Requirement

ต้องการเพิ่ม/แก้ ...

## เป้าหมาย
- ...

## ขอบเขตที่ให้แก้
- ...

## ห้ามแตะ
- ...

## Validation ที่ต้องผ่าน
- ...
```

สร้าง context และ plan:

```bash
.ai-agent/bin/aia refresh
.ai-agent/bin/aia plan
```

Planner จะสร้าง:

```text
.ai-agent/ai-plan/overview.md
.ai-agent/ai-plan/context.md
.ai-agent/ai-plan/tasks/task-*.md
```

ถ้าต้องการให้ Planner ถาม challenge ก่อนสร้าง task implementation ให้ใช้ Planner Grill Me Loop:

```bash
.ai-agent/bin/aia refresh
.ai-agent/bin/aia plan grill-start
```

Planner จะสร้างเฉพาะ draft และคำถาม:

```text
.ai-agent/ai-plan/draft-plan.md
.ai-agent/ai-plan/grill/questions.md
.ai-agent/ai-plan/grill/answers.md
.ai-agent/ai-plan/grill/round-001-questions.md
.ai-agent/ai-plan/grill/round-001-answers.md
```

ผู้ใช้ตอบใน `.ai-agent/ai-plan/grill/answers.md` แล้ววนต่อ:

```bash
.ai-agent/bin/aia plan grill-next
```

เมื่อพร้อมให้ใส่บรรทัด `APPROVED` หรือ `PLAN APPROVED` ใน answers แล้ว freeze:

```bash
.ai-agent/bin/aia plan freeze
```

หลัง freeze เท่านั้น Planner จึงสร้าง:

```text
.ai-agent/ai-plan/overview.md
.ai-agent/ai-plan/context.md
.ai-agent/ai-plan/tasks/task-*.md
```

กฎสำคัญ: `draft-plan.md` และ `revised-plan.md` ไม่ใช่ plan สำหรับ Coder และห้ามใช้เริ่ม implementation ก่อน approve/freeze ถ้าต้องการบังคับ flow นี้ทุกครั้ง ให้ตั้ง `PLANNER_GRILL_REQUIRED=true`

task ที่ดีควรมี:

```md
## Status
Pending

## Goal
...

## Reference Map
- Generated: `.ai-agent/generated/runtime/context-package.md`
- Generated: `.ai-agent/generated/runtime/runtime-context.md`
- Generated: `.ai-agent/generated/knowledge/api.md`
- Source: `src/api/example.rs`
- Source: `public/admin/index.html`

## Allowed Files
- `src/api/example.rs`
- `public/admin/index.html`

## Validation Commands
- `cargo check`
```

## 5) Run Coder และ Reviewer

รันทุก task ที่ยังไม่ผ่าน:

```bash
.ai-agent/bin/aia run
```

Flow การใช้งานยังเหมือนเดิม:

```bash
.ai-agent/bin/aia refresh
.ai-agent/bin/aia plan
.ai-agent/bin/aia run
```

แต่ก่อนแต่ละ role ยิงเข้า Codex ระบบจะ build prompt แบบนี้:

1. ใส่ invocation context ของ role
2. แทรก `context-package.md`
3. ค่อยตามด้วย role prompt เฉพาะของ Planner/Coder/Reviewer/Final Reviewer

ผลคือ role เดิมยังทำงานเหมือนเดิม แต่ใช้ input context แคบลงมาก

Adaptive defaults ตาม task size:

```text
SMALL  planner context 15k, coder context 25k, search 5,  depth 1, max planner tasks 1
MEDIUM planner context 30k, coder context 50k, search 10, depth 2, max planner tasks 3
LARGE  planner context 80k, coder context 120k, search 20, depth 3, max planner tasks unlimited
```

Reasoning defaults ตาม task size:

```text
SMALL  planner medium, coder medium, reviewer medium, final reviewer high
MEDIUM planner high,   coder high,   reviewer medium, final reviewer high
LARGE  planner high,   coder high,   reviewer medium, final reviewer xhigh
```

ถ้าตั้ง `PLANNER_LEVEL`, `CODER_LEVEL`, `REVIEWER_LEVEL`, `FINAL_REVIEWER_LEVEL`, `SEARCH_BUDGET`, `CODEGRAPH_DEPTH`, หรือ `MAX_CONTEXT_TOKENS` เอง ระบบจะเคารพค่า override นั้น

โดย default `aia run` ใช้ persistent Codex sessions แบบแยกตาม task เพื่อลดการสะสม context ข้าม task:

```text
Coder task 001    -> .ai-agent/generated/runtime/sessions/tasks/task-001-.../coder_session_id.txt
Reviewer task 001 -> .ai-agent/generated/runtime/sessions/tasks/task-001-.../reviewer_session_id.txt
FinalReviewer     -> .ai-agent/generated/runtime/sessions/final_reviewer_session_id.txt
```

รอบถัดไปของ task เดิมจะ resume session เดิม เช่น Coder round 2 ของ task เดียวกันใช้ Coder session เดิม แต่ task ถัดไปจะเริ่ม session ใหม่เพื่อลด token และลด stale context ไม่แชร์ session ข้าม role

ล้างเฉพาะ session ids:

```bash
.ai-agent/bin/aia reset-sessions
```

หรือล้างก่อน run:

```bash
RESET_SESSIONS=true .ai-agent/bin/aia run
```

กลับไปใช้ behavior เดิมที่แต่ละ `codex exec` เป็น session ใหม่:

```bash
USE_PERSISTENT_SESSIONS=false .ai-agent/bin/aia run
```

ถ้าต้องการ behavior เก่าแบบ session เดียวต่อ role ข้ามทุก task:

```bash
PERSISTENT_SESSION_SCOPE=role .ai-agent/bin/aia run
```

จำกัดรอบ:

```bash
MAX_ROUNDS=2 .ai-agent/bin/aia run
```

กำหนดโมเดล:

```bash
CODER_SESSION_MODEL=gpt-5.4-mini REVIEWER_SESSION_MODEL=gpt-5.4-mini .ai-agent/bin/aia run
```

เลือก CLI แยกตาม role ได้ โดย default ทุก role ยังเป็น `codex`:

```bash
PLANNER_CLI=codex
CODER_CLI=codex
REVIEWER_CLI=codex
FINAL_REVIEWER_CLI=codex

PLANNER_MODEL=gpt-5.5
CODER_MODEL=gpt-5.4-mini
REVIEWER_MODEL=gpt-5.4-mini
FINAL_REVIEWER_MODEL=gpt-5.5
```

ใช้ `agy` เฉพาะ Coder:

```bash
MAX_ROUNDS=3 \
PLANNER_CLI=codex \
CODER_CLI=agy \
REVIEWER_CLI=codex \
CODER_MODEL=gemini-3.5-flash \
bash .ai-agent/scripts/agent-loop-all-tasks.sh
```

ใช้ `agy` ทั้ง Planner/Coder/Reviewer/Final Reviewer:

```bash
MAX_ROUNDS=3 \
PLANNER_CLI=agy \
CODER_CLI=agy \
REVIEWER_CLI=agy \
FINAL_REVIEWER_CLI=agy \
PLANNER_MODEL=gemini-3.1-pro \
CODER_MODEL=gemini-3.5-flash \
REVIEWER_MODEL=gemini-3.1-pro \
FINAL_REVIEWER_MODEL=gemini-3.1-pro \
AGY_SKIP_PERMISSIONS=true \
AGY_TIMEOUT=60m \
bash .ai-agent/scripts/agent-loop-all-tasks.sh
```

ใช้ Codex Coder แต่ `agy` Reviewer:

```bash
MAX_ROUNDS=3 \
CODER_CLI=codex \
REVIEWER_CLI=agy \
CODER_MODEL=gpt-5.4-mini \
REVIEWER_MODEL=gemini-3.1-pro \
bash .ai-agent/scripts/agent-loop-current-task.sh
```

AGY config เพิ่มเติม:

```bash
AGY_TIMEOUT=30m
AGY_SANDBOX=false
AGY_SKIP_PERMISSIONS=true
AGY_PROJECT=
AGY_CONVERSATION=
AGY_CONTINUE=false
AGY_ADD_DIRS=
```

รองรับ role-specific override เช่น `PLANNER_AGY_MODEL`, `CODER_AGY_PROJECT`, `REVIEWER_AGY_CONVERSATION`, `FINAL_REVIEWER_AGY_MODEL` โดย fallback เป็น shared AGY config แล้วค่อย fallback เป็น model/config ของ role นั้น

ถ้า Final Reviewer ไม่ผ่าน:

```bash
.ai-agent/bin/aia run --final-reviewer-fail-mode stop
.ai-agent/bin/aia run --final-reviewer-fail-mode plan-code --final-reviewer-fix-rounds 2
.ai-agent/bin/aia run --final-reviewer-fail-mode review-code --final-reviewer-fix-rounds 2
```

- `plan-code`: หลัง Final Reviewer fail ให้ Planner แก้ source code โดยตรงจน final goal ผ่าน ไม่ใช่แค่เขียนแผนหรือแก้ task
- `review-code`: หลัง Final Reviewer fail ให้ Reviewer แก้ source code โดยตรง ไม่ใช่แค่รายงานปัญหา
- ทั้งสอง mode ห้าม rewrite/split/create task files, ห้ามแก้ `overview.md` หรือ `context.md`, และยังต้องเคารพ allowed/forbidden files
- หลังแต่ละ direct fix round จะรัน Final Reviewer ใหม่ จนครบ `FINAL_REVIEWER_FIX_ROUNDS`

### หลัง ScopeGuard fail หรือหลังอัปเดต framework

ถ้ายังทำ requirement/plan เดิมอยู่ ให้ run ต่อได้เลย ไม่ต้อง `restart`:

```bash
.ai-agent/bin/aia scope check
.ai-agent/bin/aia run
```

ใช้ `restart` เฉพาะตอนเริ่ม requirement ใหม่ หรืออยากล้าง runtime/checkpoint ของรอบเดิม:

```bash
.ai-agent/bin/aia restart
.ai-agent/bin/aia refresh
.ai-agent/bin/aia plan
.ai-agent/bin/aia run
```

ถ้า `scope check` ผ่าน แปลว่า current task พร้อมเข้า Coder/Reviewer แล้ว ถ้าไม่ผ่าน ให้อ่าน `.ai-agent/generated/runtime/loop-verdict.txt` แล้วจัดการไฟล์ที่อยู่นอก scope ก่อน ไม่ควรขยาย scope เพื่อให้ผ่านถ้าไฟล์นั้นเป็นงานของ task ก่อนหน้า

อย่า `restart` กลาง plan ที่มี accepted implementation diff แล้วยังไม่ได้ commit เพราะ restart จะล้าง checkpoint ledger; task ถัดไปอาจมอง diff จาก task ที่ผ่านแล้วเป็น out-of-scope อีกครั้ง

## 6) ตรวจสถานะระหว่างทำงาน

```bash
.ai-agent/bin/aia status
.ai-agent/bin/aia progress
.ai-agent/bin/aia monitor
.ai-agent/bin/aia monitor watch
.ai-agent/bin/aia tail
.ai-agent/bin/aia tokens
```

ดูไฟล์ implementation ที่เปลี่ยนจริง:

```bash
.ai-agent/bin/aia task-files
.ai-agent/bin/aia task-diff
```

เช็ก scope:

```bash
.ai-agent/bin/aia scope allowed
.ai-agent/bin/aia scope changed
.ai-agent/bin/aia scope diff
.ai-agent/bin/aia scope preflight
.ai-agent/bin/aia scope check
```

ให้ manual scope check restore tracked files ที่หลุด scope:

```bash
SCOPE_CHECK_AUTO_RESTORE=true .ai-agent/bin/aia scope check .ai-agent/ai-plan/tasks/task-xxx.md
```

Task ที่ reviewer ผ่านแล้วจะถูก checkpoint อัตโนมัติไว้ใน `.ai-agent/generated/runtime/task-checkpoints/` เพื่อให้ task ถัดไปไม่ล้มเพราะ diff ที่ยอมรับแล้วจาก task ก่อนหน้า แต่ถ้าไฟล์ checkpointed ถูกแก้ต่อโดย task ที่ไม่ได้ระบุไฟล์นั้นใน scope จะยังถูก ScopeGuard block ก่อน Coder เริ่มทำงาน

`aia restart` จะล้าง checkpoint ledger นี้ เพื่อไม่ให้ carryover จาก requirement เก่าถูกยอมรับในงานรอบใหม่

## 7) คำสั่ง CodeGraph และ Knowledge

อัปเดตทั้งหมด:

```bash
.ai-agent/bin/aia refresh
```

CodeGraph:

```bash
.ai-agent/bin/aia graph status
.ai-agent/bin/aia graph check
.ai-agent/bin/aia graph update
.ai-agent/bin/aia graph ensure
```

Knowledge:

```bash
.ai-agent/bin/aia scan status
.ai-agent/bin/aia scan check
.ai-agent/bin/aia scan update
.ai-agent/bin/aia scan ensure
```

Runtime context:

```bash
.ai-agent/bin/aia context
```

กฎการใช้:

- ใช้ `refresh` เมื่อต้องการ rebuild แน่นอนก่อนเริ่มงานใหญ่
- ใช้ `graph check` หรือ `scan check` เพื่อตรวจว่า cache stale หรือไม่
- `ensure` จะ rebuild เฉพาะเมื่อ fingerprint หรือ mode เปลี่ยน
- `plan` จะ ensure graph/scan และ rebuild `runtime-context.md` ก่อนเรียก Planner ทุกครั้ง

## 8) README และ docs

สร้าง context สำหรับอัปเดต README:

```bash
.ai-agent/bin/aia docs build
```

ผลลัพธ์:

```text
.ai-agent/generated/knowledge/README-build-context.md
```

ใช้ไฟล์นี้เป็น input เวลาสั่ง AI ให้ปรับ `README.md`

## 9) คำสั่งจัดการ framework

ดู help:

```bash
.ai-agent/bin/aia help
```

ตรวจ version:

```bash
.ai-agent/bin/aia version
```

ตรวจไฟล์ที่ต้องมี:

```bash
.ai-agent/bin/aia doctor
```

รัน migrations ของ framework:

```bash
.ai-agent/bin/aia migrate
```

ดู diff ของ framework เทียบ manifest:

```bash
.ai-agent/bin/aia diff
```

ตรวจ manifest, syntax, required files:

```bash
.ai-agent/bin/aia verify
```

สร้าง zip package:

```bash
.ai-agent/bin/aia pack
```

ออก release ใหม่จาก package ต้นทาง:

```bash
.ai-agent/bin/aia release 2.2.0
```

ย้อน backup ล่าสุด:

```bash
.ai-agent/bin/aia rollback
```

ล้าง runtime prompt/context ชั่วคราว:

```bash
.ai-agent/bin/aia clean
```

## 10) Prompt roles

ดู prompt ที่จะส่งให้ agent:

```bash
.ai-agent/bin/aia prompt planner
.ai-agent/bin/aia prompt coder
.ai-agent/bin/aia prompt reviewer
.ai-agent/bin/aia prompt reviewer-final
```

เพิ่ม prompt เฉพาะโปรเจ็กต์โดยสร้างไฟล์:

```text
.ai-agent/custom/prompts/planner.append.md
.ai-agent/custom/prompts/coder.append.md
.ai-agent/custom/prompts/reviewer.append.md
.ai-agent/custom/prompts/reviewer-final.append.md
```

## 11) จบงาน

หลัง `aia run` ผ่าน:

```bash
.ai-agent/bin/aia status
.ai-agent/bin/aia task-files
.ai-agent/bin/aia task-diff
.ai-agent/bin/aia tokens
```

รัน validation ของโปรเจ็กต์ตาม task เช่น:

```bash
cargo check
npm run check
```

จากนั้น commit/deploy ตาม workflow ของโปรเจ็กต์

## 12) Troubleshooting

ถ้า `graph check` หรือ `scan check` stale:

```bash
.ai-agent/bin/aia refresh
```

ถ้า Planner สร้าง task ที่ไม่มี `Reference Map`:

```bash
.ai-agent/bin/aia refresh
.ai-agent/bin/aia plan
```

ถ้า `aia run` ย้อนกลับไป task เก่า ให้ดู status:

```bash
find .ai-agent/ai-plan/tasks -maxdepth 1 -type f -name 'task-*.md' -print \
  -exec awk '/^## Status/{getline; print FILENAME ":" $0; exit}' {} \;
```

task ที่จบแล้วควรเป็น:

```md
## Status
Passed
```

ถ้า generated context ดูเก่า:

```bash
.ai-agent/bin/aia graph status
.ai-agent/bin/aia scan status
stat .ai-agent/generated/cache/codegraph-project.md
stat .ai-agent/generated/knowledge/architecture.md
stat .ai-agent/generated/runtime/context-package.md
```

ถ้า Coder หรือ Reviewer ดูเหมือนอ่าน context กว้างเกินไป ให้เช็ก:

```bash
cat .ai-agent/generated/runtime/search-allowlist.txt
cat .ai-agent/generated/runtime/context-package.meta.json
cat .ai-agent/generated/runtime/task-size.json
sed -n '1,220p' .ai-agent/generated/runtime/context-package.md
```

ถ้ารอบ repair ต้องดู feedback แบบย่อก่อน:

```bash
sed -n '1,220p' .ai-agent/generated/runtime/reviewer-summary.md
```

raw runtime logs เช่น `coder-round-*.log`, `reviewer-round-*.log`, `token-usage*.jsonl` ควรเปิดเฉพาะตอน `REPAIR_MODE=true` หรือเวลาต้องสืบปัญหา previous run จริงๆ

## 13) [v2.3] Review-Scope Config — ควบคุม unit test และ architecture review

config ใหม่ 4 ตัวควบคุมว่า unit test และ architecture review จะทำในขั้นไหน:

| Config | Default | ความหมาย |
|---|---|---|
| `FAST_REVIEW_UNIT_TEST` | `false` | Fast Reviewer (รายTask) รัน unit test หรือไม่ |
| `FAST_REVIEW_ARCHITECTURE` | `false` | Fast Reviewer ทำ architecture review หรือไม่ |
| `FINAL_REVIEW_UNIT_TEST` | `true` | Final Reviewer รัน unit test หรือไม่ |
| `FINAL_REVIEW_ARCHITECTURE` | `true` | Final Reviewer ทำ architecture review หรือไม่ |

ทุก task จะมี log บอกชัดเจน:

```text
== Fast Reviewer: gpt-5.4-mini ==
  [review-scope] FAST_REVIEW_UNIT_TEST=false — unit test skipped in Fast Reviewer (deferred to Final Reviewer)
  [review-scope] FAST_REVIEW_ARCHITECTURE=false — architecture review skipped in Fast Reviewer (deferred to Final Reviewer)
```

และตอน Final Reviewer:

```text
== Final review: gpt-5.5 (initial) ==
  [review-scope] FINAL_REVIEW_UNIT_TEST=true — unit test ENABLED in Final Reviewer
  [review-scope] FINAL_REVIEW_ARCHITECTURE=true — architecture review ENABLED in Final Reviewer
```

### แบบประหยัด token (default)

unit test และ architecture review จะรันแค่ครั้งเดียวตอนท้าย:

```bash
# ไม่ต้องตั้งค่าใดเลย — ค่า default คือแบบประหยัด
.ai-agent/bin/aia run

# หรือตั้งชัดเจน
FAST_REVIEW_UNIT_TEST=false \
FAST_REVIEW_ARCHITECTURE=false \
FINAL_REVIEW_UNIT_TEST=true \
FINAL_REVIEW_ARCHITECTURE=true \
.ai-agent/bin/aia run
```

### แบบเร็วสุด ไม่รัน test และไม่ review architecture เลย

สำหรับ prototyping หรือรอบ draft ที่ต้องการ iterate เร็ว:

```bash
FINAL_REVIEW_UNIT_TEST=false \
FINAL_REVIEW_ARCHITECTURE=false \
.ai-agent/bin/aia run
```

### แบบเข้มงวดมาก รัน test ทุก task

ใช้เมื่อต้องการ catch regression ทันทีในแต่ละ task (ใช้ token มากกว่าปกติ):

```bash
FAST_REVIEW_UNIT_TEST=true \
FAST_REVIEW_ARCHITECTURE=true \
FINAL_REVIEW_UNIT_TEST=true \
FINAL_REVIEW_ARCHITECTURE=true \
.ai-agent/bin/aia run
```

### Override ผ่าน environment variable ได้เสมอ

```bash
# ปิด unit test เฉพาะรอบนี้
FINAL_REVIEW_UNIT_TEST=false .ai-agent/bin/aia run

# เปิด unit test ทุก task เฉพาะรอบนี้
FAST_REVIEW_UNIT_TEST=true .ai-agent/bin/aia run
```

ดู log สถานะ review-scope:

```bash
grep "review-scope" .ai-agent/generated/runtime/events.log
```
