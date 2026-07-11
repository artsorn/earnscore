#!/usr/bin/env bash
set -euo pipefail

AI_DIR="${AI_DIR:-$(cd "$(dirname "$0")/.." && pwd)}"
PACKAGE_ROOT="$(cd "$AI_DIR/.." && pwd)"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

if [[ -f "$PACKAGE_ROOT/install.sh" ]]; then
  bash "$PACKAGE_ROOT/install.sh" "$tmp" >/dev/null
else
  mkdir -p "$tmp/.ai-agent"
  cp -a "$AI_DIR/." "$tmp/.ai-agent/"
  AI_DIR="$tmp/.ai-agent" PROJECT_ROOT="$tmp" "$tmp/.ai-agent/bin/aia" init >/dev/null
fi
cd "$tmp"

printf 'Build a fresh feature.\n' > .agent/requirement.md
printf 'user source must remain\n' > source.txt
mkdir -p .ai-agent/ai-plan/tasks .ai-agent/ai-plan/grill .ai-agent/generated/runtime/sessions .ai-agent/generated/runtime/task-checkpoints
printf '# Old overview\n' > .ai-agent/ai-plan/overview.md
printf '# Old context\n' > .ai-agent/ai-plan/context.md
printf '# Old task\n' > .ai-agent/ai-plan/tasks/task-001-old.md
printf '# Old questions\n' > .ai-agent/ai-plan/grill/questions.md
printf '{"stage":"old"}\n' > .ai-agent/generated/status.json
printf 'old-session\n' > .ai-agent/generated/runtime/sessions/coder_session_id.txt
printf 'old-checkpoint\n' > .ai-agent/generated/runtime/task-checkpoints/accepted-files.tsv
printf '{"tokens":123}\n' > .ai-agent/generated/runtime/token-usage.jsonl
printf 'old log\n' > .ai-agent/generated/runtime/coder-round-1.log

restart_output="$(.ai-agent/bin/aia restart)"
printf '%s\n' "$restart_output" | grep -q 'Restarted for a new plan'
test -f source.txt
grep -q 'Build a fresh feature' .agent/requirement.md
test ! -e .ai-agent/ai-plan/overview.md
test ! -e .ai-agent/ai-plan/context.md
test ! -e .ai-agent/ai-plan/tasks/task-001-old.md
test ! -e .ai-agent/ai-plan/grill
test -f .ai-agent/ai-plan/tasks/.gitkeep
test ! -e .ai-agent/generated/status.json
test ! -e .ai-agent/generated/runtime/sessions
test ! -e .ai-agent/generated/runtime/task-checkpoints
test ! -e .ai-agent/generated/runtime/coder-round-1.log
find .ai-agent/generated/runtime -maxdepth 1 -type f -name 'token-usage.*.jsonl' | grep -q .
backup_plan="$(find .ai-agent/backups -path '*/restart-*/ai-plan/tasks/task-001-old.md' -print -quit)"
test -n "$backup_plan"

printf '# Kept task\n' > .ai-agent/ai-plan/tasks/task-002-keep.md
printf '{"stage":"old-again"}\n' > .ai-agent/generated/status.json
.ai-agent/bin/aia restart --keep-plan >/dev/null
test -f .ai-agent/ai-plan/tasks/task-002-keep.md
test ! -e .ai-agent/generated/status.json

set +e
.ai-agent/bin/aia restart --unknown > "$tmp/restart-invalid.out" 2>&1
code=$?
set -e
test "$code" -eq 2
grep -q 'restart \[--keep-plan\]' "$tmp/restart-invalid.out"

echo "Restart tests passed."
