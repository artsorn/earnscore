#!/usr/bin/env bash
set -euo pipefail
AI_DIR="${AI_DIR:-.ai-agent}"
PROJECT_ROOT="$(pwd)"
if [[ -f "$AI_DIR/scripts/agent-load-env.sh" ]]; then
  # shellcheck disable=SC1090
  source "$AI_DIR/scripts/agent-load-env.sh"
  load_agent_env "$AI_DIR"
fi
VERSION="$(cat "$AI_DIR/VERSION" 2>/dev/null || echo unknown)"
RUNTIME="${RUNTIME_DIR:-$AI_DIR/generated/runtime}"
STATUS_JSON="${STATUS_JSON:-$AI_DIR/generated/status.json}"
[[ -f "$STATUS_JSON" ]] || STATUS_JSON="$RUNTIME/status.json"

fmt_elapsed() { local s="${1:-0}"; printf '%02d:%02d:%02d' $((s/3600)) $(((s%3600)/60)) $((s%60)); }
status_of() { awk '/^## Status/{getline; print; exit}' "$1" 2>/dev/null | tr '[:upper:]' '[:lower:]' | xargs || true; }
count_tasks() { find "$AI_DIR/ai-plan/tasks" -maxdepth 1 -type f -name 'task-*.md' 2>/dev/null | wc -l | tr -d ' '; }
count_status() { local pat="$1"; find "$AI_DIR/ai-plan/tasks" -maxdepth 1 -type f -name 'task-*.md' 2>/dev/null | while read -r f; do status_of "$f" | grep -Eqi "$pat" && echo x || true; done | wc -l | tr -d ' '; }
filtered_git_status() { git status --short --untracked-files=all 2>/dev/null | grep -vE '^.. \.ai-agent/|^.. \.agent/(loop-verdict\.txt|requirement\.md)$|^.. AGENTS\.md$|^.. \.gitignore$' || true; }

cat <<HEAD
AI Agent Status v$VERSION
========================================
Project root : $PROJECT_ROOT
Agent dir    : $AI_DIR
HEAD

if [[ -f "$STATUS_JSON" ]]; then
  python3 - "$STATUS_JSON" <<'PY'
import json, sys
p=sys.argv[1]
try: d=json.load(open(p, encoding='utf-8'))
except Exception as e:
    print(f"Live status  : unreadable ({e})"); raise SystemExit
elapsed=int(d.get('elapsed_seconds') or 0)
h=f"{elapsed//3600:02d}:{(elapsed%3600)//60:02d}:{elapsed%60:02d}"
print("\nLive")
print(f"- Stage      : {d.get('stage','unknown')}")
print(f"- Status     : {d.get('status', d.get('verdict',''))}")
print(f"- Task       : {d.get('task','')}")
print(f"- Round      : {d.get('round',0)}/{d.get('max_rounds',0)}")
print(f"- Elapsed    : {h}")
print(f"- Verdict    : {d.get('verdict','')}")
print(f"- Message    : {d.get('message','')}")
if d.get('base_commit'): print(f"- Base       : {d.get('base_commit')}")
if d.get('tokens'): print(f"- Tokens     : {d.get('tokens')}")
if d.get('out_of_scope_files'):
    print("- Out scope  : " + ', '.join(d.get('out_of_scope_files') or []))
if d.get('changed_files'):
    print("- Changed    : " + ', '.join((d.get('changed_files') or [])[:12]))
PY
else
  echo
  echo "Live"
  echo "- No status JSON yet: $STATUS_JSON"
fi

branch="$(git symbolic-ref --quiet --short HEAD 2>/dev/null || true)"
branch="${branch:-unknown}"
commit="$(git rev-parse --short HEAD 2>/dev/null || echo none)"
dirty="$(filtered_git_status | wc -l | tr -d ' ')"
echo
echo "Git"
echo "- Branch     : $branch"
echo "- Commit     : $commit"
echo "- Implementation dirty files: $dirty"
if [[ "$dirty" != "0" ]]; then filtered_git_status | sed -n '1,15p' | sed 's/^/  /'; fi

total="$(count_tasks)"; passed="$(count_status 'passed|done|complete|completed')"; failed="$(count_status '^failed$')"; review="$(count_status 'pending review|review')"
echo
echo "Plan"
echo "- Tasks total    : $total"
echo "- Tasks passed   : $passed"
echo "- Tasks failed   : $failed"
echo "- Pending review : $review"

echo
echo "Config"
echo "- STRICT_SCOPE              : ${STRICT_SCOPE:-true}"
echo "- USE_PERSISTENT_SESSIONS   : ${USE_PERSISTENT_SESSIONS:-true}"
echo "- PERSISTENT_SESSION_SCOPE  : ${PERSISTENT_SESSION_SCOPE:-task}"
echo "- PRE_TASK_SCOPE_GATE       : ${PRE_TASK_SCOPE_GATE:-true}"
echo "- TASK_SCOPE_CARRYOVER      : ${TASK_SCOPE_CARRYOVER:-true}"
echo "- TASK_CHECKPOINT_ON_PASS   : ${TASK_CHECKPOINT_ON_PASS:-true}"
echo "- AUTO_RESTORE_OUT_OF_SCOPE : ${AUTO_RESTORE_OUT_OF_SCOPE:-true}"
echo "- REVIEW_ALLOWED_ONLY       : ${REVIEW_ALLOWED_ONLY:-true}"
echo "- AUTO_MARK_TASK_PASSED     : ${AUTO_MARK_TASK_PASSED:-true}"
echo "- STATUS_JSON               : ${STATUS_JSON:-$AI_DIR/generated/status.json}"
echo "- CODEX_TOOL_OUTPUT_LIMIT   : ${CODEX_TOOL_OUTPUT_TOKEN_LIMIT:-12000}"
echo "- CODEX_AUTO_COMPACT        : ${CODEX_AUTO_COMPACT_TOKEN_LIMIT:-80000}"
echo "- CODEX_ROLLOUT_BUDGET      : ${CODEX_ROLLOUT_BUDGET_TOKENS:-160000}"

if [[ -f "$RUNTIME/validation-latest.json" ]]; then
  echo
  echo "Validation"
  python3 - "$RUNTIME/validation-latest.json" <<'PY'
import json, sys
try:
    row = json.load(open(sys.argv[1], encoding='utf-8'))
except Exception as exc:
    print(f"- Latest     : unreadable ({exc})")
else:
    print(f"- Latest     : {row.get('status', 'unknown')}")
    print(f"- Command    : {row.get('command', '')}")
    print(f"- Reason     : {row.get('reason', '')}")
PY
fi
