# Project Agent Entry

อ่านคำสั่งหลักจาก `.ai-agent/AGENTS.md`

กฎสำคัญ:
- อย่าอ่านทั้ง repo ถ้าไม่จำเป็น
- ดู `.ai-agent/generated/runtime/task-size.txt` เพื่อรู้ว่าเป็น SMALL/MEDIUM/LARGE
- ดู `.ai-agent/generated/runtime/context-escalation.txt` เพื่อรู้ว่า script เลือก context level ไหน
- ดู `.ai-agent/generated/runtime/context-manifest.txt` เพื่อ debug ว่ารวมไฟล์อะไรบ้าง
- เริ่มจาก `.ai-agent/generated/runtime/context-package.md`
- ทำตาม task ปัจจุบันใน `.ai-agent/ai-plan/tasks/`
- ห้ามแก้ไฟล์นอก Allowed/Required Files เว้นแต่ task ระบุชัดเจน
- จำกัดการค้นหาตาม `.ai-agent/generated/runtime/search-allowlist.txt` และ `SEARCH_BUDGET`
- ห้ามอ่าน/ค้นหา runtime logs, token files, jsonl, cache/tmp/history อัตโนมัติ ยกเว้น `REPAIR_MODE=true` หรือผู้ใช้ขอให้ตรวจ previous runs โดยตรง
- ใช้ `.ai-agent/generated/runtime/reviewer-summary.md` ก่อน raw reviewer logs เสมอ
- ไฟล์ใน `.ai-agent/`, root `AGENTS.md`, `.gitignore`, และ `.agent/loop-verdict.txt` เป็น agent framework/generated/local/task state ไม่ใช่ implementation scope
- ถ้าต้องเพิ่มคำสั่งเฉพาะโปรเจ็กต์ ให้ใส่ใน `.ai-agent/custom/prompts/*.append.md`
