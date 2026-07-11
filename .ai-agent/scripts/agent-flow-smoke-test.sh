#!/usr/bin/env bash
set -euo pipefail

SOURCE_AI_DIR="${AI_DIR:-$(cd "$(dirname "$0")/.." && pwd)}"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
project="$tmp/project"
mkdir -p "$project" "$tmp/bin"
cp -a "$SOURCE_AI_DIR" "$project/.ai-agent"

git -C "$project" init -q
git -C "$project" config user.email test@example.com
git -C "$project" config user.name Test
printf 'old\n' > "$project/app.txt"
git -C "$project" add app.txt
git -C "$project" commit -q -m initial
mkdir -p "$project/.ai-agent/ai-plan/tasks"
printf '# Smoke Task\n\n## Status\nPending\n\n## Allowed Files\n- app.txt\n\n## Acceptance Criteria\n- app.txt contains new\n\n## Validation Commands\n- `.ai-agent/bin/aia validate -- bash -c true`\n' \
  > "$project/.ai-agent/ai-plan/tasks/task-001-smoke.md"

printf '%s\n' '#!/usr/bin/env bash' \
  'set -euo pipefail' \
  'if [[ "${1:-}" == "features" && "${2:-}" == "list" ]]; then echo "rollout_budget under-development false"; exit 0; fi' \
  'prompt="$(cat)"' \
  'if [[ "$prompt" == *"Role: coder"* ]]; then' \
  '  grep -qx new app.txt || printf "new\n" >> app.txt' \
  '  uuid=11111111-1111-4111-8111-111111111111' \
  'elif [[ "$prompt" == *"Role: reviewer"* ]]; then' \
  '  grep -q "^+new$" .ai-agent/generated/runtime/reviewer-diff.patch' \
  '  printf "PASS\n" > .ai-agent/generated/runtime/loop-verdict.txt' \
  '  uuid=22222222-2222-4222-8222-222222222222' \
  'else uuid=33333333-3333-4333-8333-333333333333; fi' \
  'printf "{\"type\":\"thread.started\",\"thread_id\":\"%s\"}\n" "$uuid"' \
  'printf "{\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":\"completed smoke role\"}}\n"' \
  'printf "{\"type\":\"turn.completed\",\"usage\":{\"input_tokens\":1200,\"cached_input_tokens\":200,\"output_tokens\":300}}\n"' \
  > "$tmp/bin/codex"
chmod +x "$tmp/bin/codex"

(
  cd "$project"
  PATH="$tmp/bin:$PATH" AI_DIR=.ai-agent RUNTIME_DIR=.ai-agent/generated/runtime \
    .ai-agent/bin/aia validate -- bash -c true >/dev/null
  PATH="$tmp/bin:$PATH" AI_DIR=.ai-agent RUNTIME_DIR=.ai-agent/generated/runtime \
    CURRENT_TASK=.ai-agent/ai-plan/tasks/task-001-smoke.md MAX_ROUNDS=1 \
    CODER_MODEL=smoke-model REVIEWER_MODEL=smoke-model CODEX_CAPACITY_RETRY=false \
    bash .ai-agent/scripts/agent-loop-current-task.sh >/dev/null

  grep -qx new app.txt
  grep -A1 '^## Status' .ai-agent/ai-plan/tasks/task-001-smoke.md | grep -q Passed
  test -s .ai-agent/generated/runtime/coder-round-1.raw.log.gz
  test -s .ai-agent/generated/runtime/reviewer-round-1.raw.log.gz
  test "$(wc -l < .ai-agent/generated/runtime/coder-round-1.log)" -lt 30
  test "$(wc -l < .ai-agent/generated/runtime/reviewer-round-1.log)" -lt 30
  python3 - .ai-agent/generated/runtime/token-usage.jsonl <<'PY'
import json, sys
rows = [json.loads(line) for line in open(sys.argv[1], encoding='utf-8')]
assert [row['tokens'] for row in rows] == [1500, 1500], rows
assert all(row['accounting_source'] == 'structured_cli_usage' for row in rows), rows
PY
)

echo "Planner-compatible Coder -> Reviewer flow smoke test passed."
