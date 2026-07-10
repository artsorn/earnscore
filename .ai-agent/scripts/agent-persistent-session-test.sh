#!/usr/bin/env bash
set -euo pipefail

AI_DIR="${AI_DIR:-$(cd "$(dirname "$0")/.." && pwd)}"
source "$AI_DIR/scripts/agent-codex-retry.sh"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

json_log="$tmp/json.log"
text_log="$tmp/text.log"
header_log="$tmp/header.log"

cat > "$json_log" <<'EOF'
{"type":"session","session_id":"019f18e6-20ec-73b3-a0bd-4c3506026101"}
EOF
cat > "$text_log" <<'EOF'
To continue this session, run codex resume 019f18e6-20ec-73b3-a0bd-4c3506026101
EOF
cat > "$header_log" <<'EOF'
OpenAI Codex v0.142.4
--------
session id: 11111111-1111-4111-8111-111111111111
--------
user
source fixture says: To continue this session, run codex resume 22222222-2222-4222-8222-222222222222
EOF

test "$(codex_extract_session_id_from_log "$json_log")" = "019f18e6-20ec-73b3-a0bd-4c3506026101"
test "$(codex_extract_session_id_from_log "$text_log")" = "019f18e6-20ec-73b3-a0bd-4c3506026101"
test "$(codex_extract_session_id_from_log "$header_log")" = "11111111-1111-4111-8111-111111111111"

test "$(codex_session_key_for_role planner)" = "planner"
test "$(codex_session_key_for_role planner-final-fix)" = "planner"
test "$(codex_session_key_for_role coder)" = "coder"
test "$(codex_session_key_for_role reviewer)" = "reviewer"
test "$(codex_session_key_for_role reviewer-final-fix)" = "reviewer"
test "$(codex_session_key_for_role reviewer-final)" = "final_reviewer"

test "$(codex_session_env_for_key planner)" = "PLANNER_SESSION"
test "$(codex_session_env_for_key coder)" = "CODER_SESSION"
test "$(codex_session_env_for_key reviewer)" = "REVIEWER_SESSION"
test "$(codex_session_env_for_key final_reviewer)" = "FINAL_REVIEWER_SESSION"

CODEX_SESSION_DIR="$tmp/sessions"
file="$(codex_session_file_for_key coder "$tmp/coder.log")"
test "$file" = "$tmp/sessions/coder_session_id.txt"
PERSISTENT_SESSION_SCOPE=task CODEX_SESSION_SCOPE_ID=".ai-agent/ai-plan/tasks/task-001-demo.md"
file="$(codex_session_file_for_key coder "$tmp/coder.log")"
test "$file" = "$tmp/sessions/tasks/task-001-demo/coder_session_id.txt"
PERSISTENT_SESSION_SCOPE=role CODEX_SESSION_SCOPE_ID=".ai-agent/ai-plan/tasks/task-001-demo.md"
file="$(codex_session_file_for_key coder "$tmp/coder.log")"
test "$file" = "$tmp/sessions/coder_session_id.txt"
unset PERSISTENT_SESSION_SCOPE CODEX_SESSION_SCOPE_ID

"$AI_DIR/bin/aia" help | grep -q 'reset-sessions'
"$AI_DIR/bin/aia" run --help | grep -q 'USE_PERSISTENT_SESSIONS=false'
grep -q 'USE_PERSISTENT_SESSIONS:=true' "$AI_DIR/config/default.env"
grep -q 'PERSISTENT_SESSION_SCOPE:=task' "$AI_DIR/config/default.env"
grep -q 'Reset saved sessions' "$AI_DIR/bin/aia"
grep -q 'codex exec resume' "$AI_DIR/scripts/agent-codex-retry.sh"
grep -q 'USE_PERSISTENT_SESSIONS=false; starting fresh' "$AI_DIR/scripts/agent-codex-retry.sh"

echo "Persistent session tests passed."
