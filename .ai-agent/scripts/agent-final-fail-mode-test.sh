#!/usr/bin/env bash
set -euo pipefail

AI_DIR="${AI_DIR:-$(cd "$(dirname "$0")/.." && pwd)}"

script="$AI_DIR/scripts/agent-loop-all-tasks.sh"

bash -n "$script"

grep -q 'stop|plan-code|planner-code|review-code' "$script"
grep -q 'planner-code) printf '\''plan-code'\''' "$script"
grep -q 'FINAL_REVIEWER_FAIL_MODE=plan-code' "$script"
grep -q 'Planner will directly edit code until final goal is satisfied.' "$script"
grep -q 'Planner will not rewrite task files.' "$script"
grep -q 'FINAL_REVIEWER_FAIL_MODE=review-code' "$script"
grep -q 'Reviewer will directly edit code until final goal is satisfied.' "$script"
grep -q 'Reviewer will not only report issues.' "$script"
grep -q 'Do not modify .ai-agent/ai-plan/overview.md.' "$script"
grep -q 'Do not modify .ai-agent/ai-plan/context.md.' "$script"
grep -q 'Do not modify .ai-agent/ai-plan/tasks/\*.md.' "$script"
grep -q 'Run required validation commands with real command output evidence' "$script"

"$AI_DIR/bin/aia" run --help | grep -q -- '--final-reviewer-fail-mode <stop|plan-code|review-code>'
"$AI_DIR/bin/aia" run --help | grep -q 'planner-code is accepted as a backward-compatible alias for plan-code.'

echo "Final reviewer fail mode tests passed."
