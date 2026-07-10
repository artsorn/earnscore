#!/usr/bin/env bash
set -euo pipefail
AI_DIR="${AI_DIR:-.ai-agent}"
VERSION="$(cat "$AI_DIR/VERSION" 2>/dev/null || echo unknown)"
RUNTIME="${RUNTIME_DIR:-$AI_DIR/generated/runtime}"
STATUS_JSON="${STATUS_JSON:-$AI_DIR/generated/status.json}"
[[ -f "$STATUS_JSON" ]] || STATUS_JSON="$RUNTIME/status.json"
EVENTS="$RUNTIME/events.log"
action="${1:-once}"; interval="${MONITOR_INTERVAL:-2}"
filtered_git_status() { git status --short 2>/dev/null | grep -vE '^.. \.ai-agent/|^.. \.agent/loop-verdict\.txt$|^.. AGENTS\.md$|^.. \.gitignore$' || true; }
render() {
  clear 2>/dev/null || true
  echo "AI Agent Monitor v$VERSION"
  echo "========================================"
  if [[ -f "$STATUS_JSON" ]]; then
    python3 - "$STATUS_JSON" <<'PY'
import json,sys
try: d=json.load(open(sys.argv[1], encoding='utf-8'))
except Exception as e: print(f"Status JSON unreadable: {e}"); raise SystemExit
elapsed=int(d.get('elapsed_seconds') or 0); h=f"{elapsed//3600:02d}:{(elapsed%3600)//60:02d}:{elapsed%60:02d}"
print(f"Stage      : {d.get('stage','unknown')}")
print(f"Task       : {d.get('task','')}")
print(f"Round      : {d.get('round',0)}/{d.get('max_rounds',0)}")
print(f"Elapsed    : {h}")
print(f"Verdict    : {d.get('verdict','')}")
print(f"Message    : {d.get('message','')}")
if d.get('out_of_scope_files'):
    print("Out scope  : " + ', '.join(d.get('out_of_scope_files') or []))
PY
  else echo "No live status yet: $STATUS_JSON"; fi
  echo; echo "Implementation diff"; filtered_git_status | sed -n '1,20p' || true
  echo; echo "Last events"; [[ -f "$EVENTS" ]] && tail -n 12 "$EVENTS" || echo "No events yet: $EVENTS"
  echo; echo "Tip: Ctrl+C to exit monitor. Use '.ai-agent/bin/aia tail' for raw logs."
}
case "$action" in once) render ;; watch|live) while true; do render; sleep "$interval"; done ;; *) echo "Usage: .ai-agent/bin/aia monitor [once|watch]" >&2; exit 2 ;; esac
