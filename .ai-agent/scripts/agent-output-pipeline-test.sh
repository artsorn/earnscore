#!/usr/bin/env bash
set -euo pipefail

AI_DIR="${AI_DIR:-$(cd "$(dirname "$0")/.." && pwd)}"
# shellcheck disable=SC1090
source "$AI_DIR/scripts/agent-codex-retry.sh"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

large_log="$tmp/large.log"
: > "$large_log"
RAW_LOG_COMPRESSION=gzip \
VISIBLE_LOG_MAX_LINES=420 \
VISIBLE_LOG_MAX_BYTES=50000 \
VISIBLE_LOG_DIFF_MAX_LINES=40 \
agent_capture_command "$large_log" "$tmp/large.raw.log.gz" python3 -c '
for i in range(50000):
    print("diff --git a/large.txt b/large.txt" if i % 500 == 0 else f"+repeated diff payload {i}")
print("FATAL: synthetic important failure marker")
' >/dev/null

test "$(gzip -cd "$tmp/large.raw.log.gz" | wc -l)" -eq 50001
test "$(wc -l < "$large_log")" -le 425
grep -q 'FATAL: synthetic important failure marker' "$large_log"
grep -q 'diff_lines_seen=' "$large_log"
raw_bytes="$(gzip -cd "$tmp/large.raw.log.gz" | wc -c)"
visible_bytes="$(wc -c < "$large_log")"
test "$visible_bytes" -lt $((raw_bytes / 20))

json_log="$tmp/codex.log"
: > "$json_log"
agent_capture_command "$json_log" "$tmp/codex.raw.log.gz" python3 -c '
import json
print(json.dumps({"type":"thread.started","thread_id":"11111111-1111-4111-8111-111111111111"}))
for i in range(2000):
    print(json.dumps({"type":"item.completed","item":{"type":"command_execution","output":"X" * 4000}}))
print(json.dumps({"type":"turn.completed","usage":{"input_tokens":42000,"cached_input_tokens":12000,"output_tokens":3000}}))
' >/dev/null
agent_record_token_usage "$tmp/token-usage.jsonl" "$json_log" coder test-model 1 task.md
python3 - "$tmp/token-usage.jsonl" <<'PY'
import json, sys
row = json.loads(open(sys.argv[1], encoding='utf-8').readline())
assert row['tokens'] == 45000, row
assert row['input_tokens'] == 42000, row
assert row['cached_input_tokens'] == 12000, row
assert row['accounting_source'] == 'structured_cli_usage', row
PY

# Completed command events may contain an enormous aggregated_output with words
# such as "error" or "failed". Keep that payload only in the exact raw log;
# otherwise a single JSON line can consume the entire rollout budget.
json_command_log="$tmp/codex-command.log"
: > "$json_command_log"
VISIBLE_LOG_MAX_BYTES=40000 \
agent_capture_command "$json_command_log" "$tmp/codex-command.raw.log.gz" python3 -c '
import json
payload = "error: synthetic diff content " + ("X" * 250000) + " failed"
print(json.dumps({"type":"item.completed","item":{"type":"command_execution","status":"completed","aggregated_output":payload}}))
print(json.dumps({"type":"item.completed","item":{"type":"command_execution","status":"failed","aggregated_output":payload}}))
print(json.dumps({"type":"turn.failed","message":"shared rollout token budget exhausted"}))
' >/dev/null
test "$(gzip -cd "$tmp/codex-command.raw.log.gz" | wc -c)" -gt 500000
test "$(wc -c < "$json_command_log")" -lt 5000
grep -q '^command_execution: failed$' "$json_command_log"
grep -q '^turn.failed: shared rollout token budget exhausted$' "$json_command_log"
! grep -q 'synthetic diff content' "$json_command_log"

validation_runtime="$tmp/runtime"
AI_DIR="$AI_DIR" RUNTIME_DIR="$validation_runtime" bash "$AI_DIR/scripts/agent-validation.sh" -- bash -c 'echo ok' >/dev/null
test "$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["status"])' "$validation_runtime/validation-latest.json")" = PASS
set +e
AI_DIR="$AI_DIR" RUNTIME_DIR="$validation_runtime" bash "$AI_DIR/scripts/agent-validation.sh" -- bash -c 'echo assertion mismatch; exit 7' >/dev/null
fail_code=$?
AI_DIR="$AI_DIR" RUNTIME_DIR="$validation_runtime" bash "$AI_DIR/scripts/agent-validation.sh" -- bash -c 'echo could not resolve host: example.invalid; exit 1' >/dev/null
blocked_code=$?
set -e
test "$fail_code" -eq 7
test "$blocked_code" -eq 3
AI_DIR="$AI_DIR" RUNTIME_DIR="$validation_runtime" bash "$AI_DIR/scripts/agent-validation.sh" --not-run 'not requested in this phase' >/dev/null
python3 - "$validation_runtime/validation-status.jsonl" <<'PY'
import json, sys
statuses = [json.loads(line)['status'] for line in open(sys.argv[1], encoding='utf-8')]
assert statuses == ['PASS', 'FAIL', 'BLOCKED_BY_ENVIRONMENT', 'NOT_RUN'], statuses
PY

repo="$tmp/repo"
mkdir -p "$repo/.ai-agent/ai-plan/tasks" "$repo/.ai-agent/generated/runtime"
git -C "$repo" init -q
git -C "$repo" config user.email test@example.com
git -C "$repo" config user.name Test
printf 'old\n' > "$repo/app.txt"
git -C "$repo" add app.txt
git -C "$repo" commit -q -m initial
printf 'new\n' > "$repo/app.txt"
printf '# Task\n\n## Allowed Files\n- app.txt\n' > "$repo/.ai-agent/ai-plan/tasks/task.md"
(
  cd "$repo"
  AI_DIR=.ai-agent RUNTIME_DIR=.ai-agent/generated/runtime \
    bash "$AI_DIR/scripts/agent-scope-guard.sh" prepare-reviewer-diff .ai-agent/ai-plan/tasks/task.md 1 >/dev/null
  first_hash="$(cat .ai-agent/generated/runtime/reviewer-diff.sha256)"
  AI_DIR=.ai-agent RUNTIME_DIR=.ai-agent/generated/runtime \
    bash "$AI_DIR/scripts/agent-scope-guard.sh" prepare-reviewer-diff .ai-agent/ai-plan/tasks/task.md 1 >/dev/null
  test "$first_hash" = "$(cat .ai-agent/generated/runtime/reviewer-diff.sha256)"
  grep -q '"state": "unchanged"' .ai-agent/generated/runtime/reviewer-diff.meta.json
  test "$(grep -c '^diff --git ' .ai-agent/generated/runtime/reviewer-diff.patch)" -eq 1
)

echo "Output pipeline, token accounting, validation status, and repeated-diff tests passed."
