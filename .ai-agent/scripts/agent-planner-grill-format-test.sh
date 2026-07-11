#!/usr/bin/env bash
set -euo pipefail

AI_DIR="${AI_DIR:-$(cd "$(dirname "$0")/.." && pwd)}"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

cat > "$tmp/valid.md" <<'EOF'
# Planner Grill Questions

## Q1: ควรเก็บข้อมูลย้อนหลังนานเท่าไร?
**Why this matters:** ระยะเวลาเก็บข้อมูลมีผลต่อค่าใช้จ่ายและหน้ารายงาน

1. **เก็บ 90 วัน** [AI RECOMMENDED]
   - Explanation: สมดุลระหว่างการวิเคราะห์แนวโน้มกับพื้นที่จัดเก็บ
   - Example: ผู้ใช้เปิดกราฟย้อนหลังได้ประมาณสามเดือน
2. **เก็บ 30 วัน**
   - Explanation: ประหยัดพื้นที่ที่สุดแต่เปรียบเทียบแนวโน้มระยะยาวไม่ได้
   - Example: รายงานเดือนก่อนยังอยู่ แต่ไตรมาสก่อนหายไป
3. **เก็บ 1 ปี**
   - Explanation: เหมาะกับรายงานตามฤดูกาลแต่ใช้พื้นที่มากขึ้น
   - Example: เปรียบเทียบข้อมูลเดือนเดียวกันของปีก่อนได้
4. **ไม่ลบอัตโนมัติ**
   - Explanation: เก็บประวัติครบแต่ต้องวางแผนพื้นที่และ archive เอง
   - Example: ข้อมูลทุกปีอยู่จนกว่าผู้ดูแลจะลบ

**Custom:** ระบุจำนวนวันและผลลัพธ์ที่ต้องการ
EOF

bash "$AI_DIR/scripts/agent-planner-grill.sh" validate-questions "$tmp/valid.md" >/dev/null

sed '/Example: รายงานเดือนก่อน/d' "$tmp/valid.md" > "$tmp/missing-example.md"
set +e
bash "$AI_DIR/scripts/agent-planner-grill.sh" validate-questions "$tmp/missing-example.md" > "$tmp/missing-example.out" 2>&1
code=$?
set -e
test "$code" -ne 0
grep -q 'Q1 choice 2: missing Example' "$tmp/missing-example.out"

sed 's/2\. \*\*เก็บ 30 วัน\*\*/2. **เก็บ 30 วัน** [AI RECOMMENDED]/' "$tmp/valid.md" > "$tmp/two-recommended.md"
set +e
bash "$AI_DIR/scripts/agent-planner-grill.sh" validate-questions "$tmp/two-recommended.md" > "$tmp/two-recommended.out" 2>&1
code=$?
set -e
test "$code" -ne 0
grep -q 'choice 1 must be the only' "$tmp/two-recommended.out"

printf '# Questions\n\nNo blocker questions were written by Planner.\n' > "$tmp/no-blockers.md"
bash "$AI_DIR/scripts/agent-planner-grill.sh" validate-questions "$tmp/no-blockers.md" >/dev/null

echo "Planner grill format tests passed."
