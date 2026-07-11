#!/usr/bin/env bash
set -euo pipefail

AI_DIR="${AI_DIR:-$(cd "$(dirname "$0")/.." && pwd)}"
RUNTIME="${RUNTIME_DIR:-$AI_DIR/generated/runtime}"
mkdir -p "$RUNTIME/validation"
if [[ -f "$AI_DIR/scripts/agent-load-env.sh" ]]; then
  # shellcheck disable=SC1090
  source "$AI_DIR/scripts/agent-load-env.sh"
  load_agent_env "$AI_DIR"
fi
if [[ -f "$AI_DIR/scripts/agent-codex-retry.sh" ]]; then
  # shellcheck disable=SC1090
  source "$AI_DIR/scripts/agent-codex-retry.sh"
fi

record_validation() {
  local status="$1" exit_code="$2" command_text="$3" visible_log="$4" raw_log="$5" reason="${6:-}"
  python3 - "$RUNTIME/validation-status.jsonl" "$RUNTIME/validation-latest.json" "$status" "$exit_code" "$command_text" "$visible_log" "$raw_log" "$reason" <<'PY'
import datetime, json, os, sys
out, latest, status, exit_code, command, visible, raw, reason = sys.argv[1:]
row = {
    "timestamp": datetime.datetime.now(datetime.timezone.utc).isoformat(),
    "status": status,
    "exit_code": int(exit_code),
    "command": command,
    "reason": reason,
    "visible_log": visible,
    "raw_log": raw,
    "visible_bytes": os.path.getsize(visible) if os.path.exists(visible) else 0,
    "raw_bytes_compressed": os.path.getsize(raw) if os.path.exists(raw) else 0,
}
with open(out, "a", encoding="utf-8") as stream:
    stream.write(json.dumps(row, ensure_ascii=False) + "\n")
with open(latest, "w", encoding="utf-8") as stream:
    json.dump(row, stream, ensure_ascii=False, indent=2)
    stream.write("\n")
PY
}

if [[ "${1:-}" == "--not-run" ]]; then
  shift
  reason="${*:-Validation was not run.}"
  record_validation "NOT_RUN" 0 "" "" "" "$reason"
  echo "Validation status: NOT_RUN — $reason"
  exit 0
fi

failure_kind="auto"
if [[ "${1:-}" == "--failure-kind" ]]; then
  failure_kind="${2:-}"
  shift 2
  case "$failure_kind" in
    auto|fail|environment) ;;
    *) echo "Invalid --failure-kind: $failure_kind (expected auto, fail, or environment)" >&2; exit 2 ;;
  esac
fi
[[ "${1:-}" == "--" ]] && shift
if [[ "$#" -eq 0 ]]; then
  echo "Usage: .ai-agent/bin/aia validate -- <command> [args...]" >&2
  echo "       .ai-agent/bin/aia validate --not-run <reason>" >&2
  exit 2
fi

stamp="$(date +%Y%m%d-%H%M%S)-$$"
visible_log="$RUNTIME/validation/${stamp}.log"
: > "$visible_log"
raw_log="$(agent_raw_log_file "$visible_log")"
printf -v command_text '%q ' "$@"
command_text="${command_text% }"

set +e
agent_capture_command "$visible_log" "$raw_log" "$@"
code=$?
set -e

status="FAIL"
reason="command exited with status $code"
if [[ "$code" -eq 0 ]]; then
  status="PASS"
  reason="command completed successfully"
elif [[ "$failure_kind" == "environment" ]]; then
  status="BLOCKED_BY_ENVIRONMENT"
  reason="caller identified a current-environment restriction"
elif [[ "$failure_kind" == "auto" ]] && python3 - "$raw_log" <<'PY'
import gzip, re, sys
path = sys.argv[1]
opener = gzip.open if path.endswith('.gz') else open
with opener(path, 'rt', encoding='utf-8', errors='replace') as stream:
    text = stream.read()[-4_000_000:]
patterns = [
    r'permission denied|operation not permitted|read-only file system|sandbox',
    r'could not resolve host|name or service not known|temporary failure in name resolution|network is unreachable',
    r'authentication required|not authenticated|missing (?:api )?(?:token|credential)|unauthorized|forbidden',
    r'command not found|no such file or directory.*(?:node|npm|cargo|wrangler|python)',
    r'wrangler.*(?:login|required|network|permission)|cloudflare.*(?:api|authentication|network)',
    r'cannot (?:access|connect|bind)|address already in use|resource temporarily unavailable',
]
raise SystemExit(0 if any(re.search(p, text, re.I) for p in patterns) else 1)
PY
then
  status="BLOCKED_BY_ENVIRONMENT"
  reason="command could not execute or complete because of the current environment"
fi

record_validation "$status" "$code" "$command_text" "$visible_log" "$raw_log" "$reason"
echo "Validation status: $status (exit=$code)"
case "$status" in
  PASS) exit 0 ;;
  BLOCKED_BY_ENVIRONMENT) exit 3 ;;
  *) exit "$code" ;;
esac
