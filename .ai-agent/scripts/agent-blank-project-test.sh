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

git rev-parse --is-inside-work-tree | grep -qx true
test -f .agent/requirement.md

set +e
.ai-agent/bin/aia plan > "$tmp/empty-plan.out" 2>&1
code=$?
set -e
test "$code" -eq 2
grep -q 'Requirement is missing or empty' "$tmp/empty-plan.out"

printf 'Create src/hello.txt containing hello blank project.\n' > .agent/requirement.md
set +e
RUN_FINAL_REVIEW=false .ai-agent/bin/aia run > "$tmp/no-tasks.out" 2>&1
code=$?
set -e
test "$code" -eq 2
grep -q 'No task files found' "$tmp/no-tasks.out"

mkdir -p src .ai-agent/ai-plan/tasks
cat > .ai-agent/ai-plan/tasks/task-001-blank.md <<'EOF'
# Task 001: Blank project

## Status
Pending

## Allowed Files
- src/hello.txt

## Acceptance Criteria
- File contains the required greeting.
EOF
printf 'hello blank project\n' > src/hello.txt

.ai-agent/scripts/agent-scope-guard.sh diff .ai-agent/ai-plan/tasks/task-001-blank.md 1 >/dev/null
grep -q '^new file mode ' .ai-agent/generated/runtime/reviewer-diff.patch
grep -q '^+hello blank project$' .ai-agent/generated/runtime/reviewer-diff.patch
! .ai-agent/bin/aia task-files | grep -q '^\.agent/requirement.md$'

echo "Blank project tests passed."
