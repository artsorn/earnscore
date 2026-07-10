#!/usr/bin/env bash
set -euo pipefail
AI_DIR="${AI_DIR:-.ai-agent}"
TASK_DIR="$AI_DIR/ai-plan/tasks"
RUNTIME="${RUNTIME_DIR:-$AI_DIR/generated/runtime}"
STATUS_JSON="${STATUS_JSON:-$AI_DIR/generated/status.json}"
[[ -f "$STATUS_JSON" ]] || STATUS_JSON="$RUNTIME/status.json"
passed=0; total=0; pending=0; failed=0; review=0
shopt -s nullglob
for f in "$TASK_DIR"/task-*.md; do
  total=$((total+1))
  st="$(awk '/^## Status/{getline; print; exit}' "$f" 2>/dev/null | tr '[:upper:]' '[:lower:]' | xargs || true)"
  case "$st" in
    passed|done|complete|completed) passed=$((passed+1)) ;;
    failed) failed=$((failed+1)) ;;
    *review*) review=$((review+1)); pending=$((pending+1)) ;;
    *) pending=$((pending+1)) ;;
  esac
done
percent=0; [[ "$total" -gt 0 ]] && percent=$((passed*100/total))
bar_done=$((percent/5)); bar_left=$((20-bar_done))
printf 'Tasks: %s/%s passed (%s%%) [' "$passed" "$total" "$percent"
printf '%*s' "$bar_done" '' | tr ' ' '#'
printf '%*s' "$bar_left" '' | tr ' ' '-'
printf ']\n'
echo "Pending: $pending | Failed: $failed | Review: $review"
if [[ -f "$STATUS_JSON" ]]; then
  python3 - "$STATUS_JSON" <<'PY'
import json,sys
try: d=json.load(open(sys.argv[1], encoding='utf-8'))
except Exception: raise SystemExit
elapsed=int(d.get('elapsed_seconds') or 0); h=f"{elapsed//3600:02d}:{(elapsed%3600)//60:02d}:{elapsed%60:02d}"
print(f"Stage: {d.get('stage','unknown')} | Round: {d.get('round',0)}/{d.get('max_rounds',0)} | Elapsed: {h}")
print(f"Task : {d.get('task','')}")
print(f"Msg  : {d.get('message','')}")
if d.get('out_of_scope_files'):
    print('Out of scope: ' + ', '.join(d.get('out_of_scope_files') or []))
PY
fi
