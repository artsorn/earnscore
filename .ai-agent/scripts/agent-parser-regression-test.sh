#!/usr/bin/env bash
set -euo pipefail

AI_DIR="${AI_DIR:-$(cd "$(dirname "$0")/.." && pwd)}"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

mkdir -p \
  "$tmp/.ai-agent/ai-plan/tasks" \
  "$tmp/src/api" \
  "$tmp/public/admin" \
  "$tmp/scripts"

cd "$tmp"
git init -q
git config user.email test@example.com
git config user.name Test

cat > src/api/auth.rs <<'EOF'
pub fn auth_marker() {}
EOF
cat > src/api/other.rs <<'EOF'
pub fn other_marker() {}
EOF
cat > public/admin/index.html <<'EOF'
<button id="reset-line">Reset</button>
EOF
cat > schema.sql <<'EOF'
CREATE TABLE staff (id INTEGER PRIMARY KEY);
EOF
cat > README.md <<'EOF'
# Test Project
EOF
cat > scripts/update-codegraph.sh <<'EOF'
#!/usr/bin/env bash
echo update
EOF

git add .
git commit -q -m initial

long_value="$(printf 'a%.0s' {1..700})"
cat > .ai-agent/ai-plan/tasks/task-001-parser-regression.md <<EOF
# Task 001: Parser Regression

## Status
Pending

## Reference Files
- public/admin/index.html
- src/api/auth.rs
- schema.sql
- README.md
- scripts/update-codegraph.sh
- invalid path/with space.rs
- ${long_value}.rs
- text
- \`\`\`text

## Allowed Files
- public/admin/index.html
- src/api/auth.rs

## Scope
- src/api/

## Requirements

\`\`\`text
รีเซ็ตการเชื่อมต่อ LINE ของพนักงานนี้หรือไม่

ระบบจะลบการผูกบัญชี LINE ทั้งหมด
พนักงานจะต้องเชื่อมต่อ LINE ใหม่
ก่อนจึงจะสามารถใช้งาน Backend ได้
\`\`\`

> public/quoted/should_not_parse.html

## Acceptance Criteria
- UI text says "รีเซ็ตการเชื่อมต่อ LINE"
- Do not parse src/api/from-acceptance.rs from free-form text.

## Examples

\`\`\`html
<div class="modal">public/admin/not-a-path.html</div>
\`\`\`

\`\`\`css
.toast { z-index: 120; }
\`\`\`

\`\`\`sql
SELECT * FROM staff WHERE note = 'schema.sql';
\`\`\`

\`\`\`bash
cat public/admin/index.html
\`\`\`

\`\`\`json
{"file":"src/api/auth.rs"}
\`\`\`
EOF

allowed="$("$AI_DIR/scripts/agent-scope-guard.sh" allowed .ai-agent/ai-plan/tasks/task-001-parser-regression.md)"
printf '%s\n' "$allowed" | grep -qx 'public/admin/index.html'
printf '%s\n' "$allowed" | grep -qx 'src/api/auth.rs'
printf '%s\n' "$allowed" | grep -qx 'src/api/'
! printf '%s\n' "$allowed" | grep -q 'รีเซ็ต'
! printf '%s\n' "$allowed" | grep -q '^text'
! printf '%s\n' "$allowed" | grep -q 'invalid path'
! printf '%s\n' "$allowed" | grep -q "$long_value"

AI_DIR="$tmp/.ai-agent" \
PROJECT_ROOT="$tmp" \
CURRENT_TASK=".ai-agent/ai-plan/tasks/task-001-parser-regression.md" \
bash "$AI_DIR/scripts/agent-codegraph.sh" update >/tmp/parser-codegraph.out

grep -q '## public/admin/index.html' .ai-agent/generated/cache/codegraph-lite.md
grep -q '## src/api/auth.rs' .ai-agent/generated/cache/codegraph-lite.md
grep -q '## schema.sql' .ai-agent/generated/cache/codegraph-lite.md
grep -q '## README.md' .ai-agent/generated/cache/codegraph-lite.md
grep -q '## scripts/update-codegraph.sh' .ai-agent/generated/cache/codegraph-lite.md
! grep -q 'รีเซ็ตการเชื่อมต่อ' .ai-agent/generated/cache/codegraph-lite.md
! grep -q 'not-a-path' .ai-agent/generated/cache/codegraph-lite.md
! grep -q "$long_value" .ai-agent/generated/cache/codegraph-lite.md

cat > .ai-agent/ai-plan/tasks/task-002-scope-regression.md <<'EOF'
# Task 002: Scope Regression

## Status
Pending

## Allowed Files
- public/admin/index.html
EOF

printf '\npub fn changed_out_of_scope() {}\n' >> src/api/other.rs
set +e
AI_DIR="$tmp/.ai-agent" \
RUNTIME_DIR="$tmp/.ai-agent/generated/runtime" \
STATUS_JSON="$tmp/.ai-agent/generated/status.json" \
LEGACY_STATUS_JSON="$tmp/.ai-agent/generated/runtime/status.json" \
AUTO_RESTORE_OUT_OF_SCOPE=false \
bash "$AI_DIR/scripts/agent-scope-guard.sh" check .ai-agent/ai-plan/tasks/task-002-scope-regression.md 0 >/tmp/parser-scope.out 2>&1
scope_code=$?
set -e
test "$scope_code" -ne 0
grep -q 'src/api/other.rs' .ai-agent/generated/runtime/loop-verdict.txt

echo "Parser regression tests passed."
