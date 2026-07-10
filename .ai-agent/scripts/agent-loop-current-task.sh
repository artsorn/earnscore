#!/usr/bin/env bash
set -euo pipefail

AI_DIR="${AI_DIR:-.ai-agent}"
PROJECT_ROOT="${PROJECT_ROOT:-$(cd "$AI_DIR/.." && pwd)}"
if [[ -f "$AI_DIR/scripts/agent-load-env.sh" ]]; then
  # shellcheck disable=SC1090
  source "$AI_DIR/scripts/agent-load-env.sh"
  load_agent_env "$AI_DIR"
fi
if [[ -f "$AI_DIR/scripts/agent-codex-retry.sh" ]]; then source "$AI_DIR/scripts/agent-codex-retry.sh"; fi
if [[ -f "$AI_DIR/scripts/agent-task-size.sh" ]]; then source "$AI_DIR/scripts/agent-task-size.sh"; fi
MAX_ROUNDS="${MAX_ROUNDS:-3}"
PLANNER_CLI="${PLANNER_CLI:-codex}"
CODER_CLI="${CODER_CLI:-codex}"
REVIEWER_CLI="${REVIEWER_CLI:-codex}"
FINAL_REVIEWER_CLI="${FINAL_REVIEWER_CLI:-codex}"
PLANNER_MODEL="${PLANNER_MODEL:-gpt-5.6-sol}"
CODER_MODEL="${CODER_MODEL:-${CODER_SESSION_MODEL:-gpt-5.6-sol}}"
REVIEWER_MODEL="${REVIEWER_MODEL:-${REVIEWER_SESSION_MODEL:-gpt-5.6-sol}}"
FINAL_REVIEWER_MODEL="${FINAL_REVIEWER_MODEL:-gpt-5.6-sol}"
SANDBOX="${SANDBOX:-workspace-write}"
CODEX_TIMEOUT="${CODEX_TIMEOUT:-120m}"
AGY_TIMEOUT="${AGY_TIMEOUT:-30m}"
AGY_SANDBOX="${AGY_SANDBOX:-false}"
AGY_SKIP_PERMISSIONS="${AGY_SKIP_PERMISSIONS:-true}"
AGY_PROJECT="${AGY_PROJECT:-}"
AGY_CONVERSATION="${AGY_CONVERSATION:-}"
AGY_CONTINUE="${AGY_CONTINUE:-false}"
AGY_ADD_DIRS="${AGY_ADD_DIRS:-}"
AUTO_COMMIT="${AUTO_COMMIT:-false}"
CONTEXT_MODE="${CONTEXT_MODE:-minimal}"
CODEGRAPH_MODE="${CODEGRAPH_MODE:-lite}"
STRICT_SCOPE="${STRICT_SCOPE:-true}"
REVIEW_ALLOWED_ONLY="${REVIEW_ALLOWED_ONLY:-true}"
TASK_BASE_COMMIT="${TASK_BASE_COMMIT:-true}"
AUTO_MARK_TASK_PASSED="${AUTO_MARK_TASK_PASSED:-true}"
PRE_TASK_SCOPE_GATE="${PRE_TASK_SCOPE_GATE:-true}"
TASK_CHECKPOINT_ON_PASS="${TASK_CHECKPOINT_ON_PASS:-true}"
# v2.3 review-scope config
FAST_REVIEW_UNIT_TEST="${FAST_REVIEW_UNIT_TEST:-false}"
FAST_REVIEW_ARCHITECTURE="${FAST_REVIEW_ARCHITECTURE:-false}"
GENERATED="$AI_DIR/generated"
RUNTIME="${RUNTIME_DIR:-$GENERATED/runtime}"
mkdir -p "$RUNTIME" "$GENERATED"
EVENTS="$RUNTIME/events.log"
STATUS_JSON="${STATUS_JSON:-$GENERATED/status.json}"
LEGACY_STATUS_JSON="${LEGACY_STATUS_JSON:-$RUNTIME/status.json}"
VERDICT_FILE="$RUNTIME/loop-verdict.txt"
REVIEWER_SUMMARY_FILE="${REVIEWER_SUMMARY_FILE:-$RUNTIME/reviewer-summary.md}"
RUN_STARTED_AT="$(date +%s)"
TASK_BASE_SHA=""

if [[ -f "$AI_DIR/scripts/agent-scope-guard.sh" ]]; then
  source "$AI_DIR/scripts/agent-scope-guard.sh"
fi

event() { local stage="$1" msg="$2"; printf '%s [%s] %s\n' "$(date -Iseconds)" "$stage" "$msg" >> "$EVENTS"; }

json_escape() { python3 -c 'import json,sys; print(json.dumps(sys.stdin.read()))'; }

record_token_usage() {
  local role="$1" model="$2" round_no="$3" log_file="$4"
  [[ -s "$log_file" ]] || return 0
  python3 - "$RUNTIME/token-usage.jsonl" "$log_file" "$role" "$model" "$round_no" "${TASK:-}" <<'PY'
import datetime, json, re, sys
out, log_file, role, model, round_no, task = sys.argv[1:7]
text = open(log_file, encoding="utf-8", errors="replace").read()
patterns = [
    r"tokens\s+used\s*[:\n ]\s*([0-9][0-9,]*)",
    r"total[_ ]tokens\s*[=:]\s*([0-9][0-9,]*)",
    r"total\s+tokens\s*[:=]\s*([0-9][0-9,]*)",
]
tokens = None
for pattern in patterns:
    matches = re.findall(pattern, text, flags=re.I)
    if matches:
        tokens = int(matches[-1].replace(",", ""))
        break
if tokens is None:
    raise SystemExit(0)
row = {
    "timestamp": datetime.datetime.now(datetime.timezone.utc).isoformat(),
    "role": role,
    "model": model,
    "round": int(round_no or 0),
    "task": task,
    "tokens": tokens,
    "log_file": log_file,
}
with open(out, "a", encoding="utf-8") as f:
    f.write(json.dumps(row, ensure_ascii=False) + "\n")
PY
}

write_status() {
  local stage="$1" task="$2" round="$3" message="$4" verdict="${5:-}"; shift 5 || true
  local now elapsed changed
  now="$(date +%s)"; elapsed=$((now - RUN_STARTED_AT))
  if declare -F changed_paths_for_scope >/dev/null 2>&1; then
    changed="$(changed_paths_for_scope | paste -sd, - || true)"
  else
    changed="$(git diff --name-only 2>/dev/null | paste -sd, - || true)"
  fi
  python3 - "$STATUS_JSON" "$LEGACY_STATUS_JSON" "$stage" "$task" "$round" "$MAX_ROUNDS" "$elapsed" "$message" "$verdict" "$TASK_BASE_SHA" "$changed" <<'PYJSON'
import json, sys, datetime, os
out, legacy, stage, task, round_s, max_rounds_s, elapsed_s, message, verdict, base, changed = sys.argv[1:12]
data = {
    "updated_at": datetime.datetime.now(datetime.timezone.utc).isoformat(),
    "stage": stage,
    "task": task,
    "round": int(round_s or 0),
    "max_rounds": int(max_rounds_s or 0),
    "elapsed_seconds": int(elapsed_s or 0),
    "message": message,
    "verdict": verdict,
    "base_commit": base,
    "changed_files": [x for x in changed.split(',') if x],
}
for path in {out, legacy}:
    if not path: continue
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8") as f:
        json.dump(data, f, ensure_ascii=False, indent=2)
        f.write("\n")
PYJSON
}

find_current_task() {
  if [[ -n "${CURRENT_TASK:-}" && -f "${CURRENT_TASK:-}" ]]; then echo "$CURRENT_TASK"; return 0; fi
  find "$AI_DIR/ai-plan/tasks" -maxdepth 1 -type f -name 'task-*.md' 2>/dev/null | sort | while read -r f; do
    status="$(awk '/^## Status/{getline; print; exit}' "$f" 2>/dev/null | tr '[:upper:]' '[:lower:]' | xargs || true)"
    case "$status" in passed|done|complete|completed) continue ;; pending|failed|"in progress"|"pending review"|"") echo "$f"; break ;; *) echo "$f"; break ;; esac
  done
}

mark_task_status() {
  local task="$1" status="$2"
  [[ -f "$task" ]] || return 0
  if grep -q '^## Status' "$task"; then
    awk -v st="$status" '
      /^## Status[[:space:]]*$/ {print; if (getline > 0) {print st} else {print st}; next}
      {print}
    ' "$task" > "$task.tmp"
    mv "$task.tmp" "$task"
  else
    tmp="$(mktemp)"; { echo "## Status"; echo "$status"; echo; cat "$task"; } > "$tmp"; mv "$tmp" "$task"
  fi
}

write_verdict_file() {
  local verdict="${1:-PENDING}"
  mkdir -p "$(dirname "$VERDICT_FILE")"
  printf '%s\n' "$verdict" > "$VERDICT_FILE"
}

read_latest_verdict() {
  python3 - "$VERDICT_FILE" <<'PY'
import re, sys
path = sys.argv[1]
try:
    lines = open(path, encoding='utf-8', errors='replace').read().splitlines()
except OSError:
    sys.exit(0)
latest = ''
for line in lines:
    m = re.match(r'^\s*(PASS|FAIL|BLOCKED)\b', line.strip(), re.I)
    if m:
        latest = m.group(1).upper()
if latest:
    print(latest)
PY
}

normalize_verdict_file() {
  local verdict="${1:-}"
  case "$verdict" in
    PASS|FAIL|BLOCKED) write_verdict_file "$verdict" ;;
    *) write_verdict_file "FAIL" ;;
  esac
}

write_reviewer_summary() {
  local round_no="${1:-0}" log_file="${2:-}" verdict_hint="${3:-}"
  python3 - "$REVIEWER_SUMMARY_FILE" "$VERDICT_FILE" "$RUNTIME/reviewer-files.txt" "$RUNTIME/reviewer-scope.txt" "$log_file" "$TASK" "$round_no" "$verdict_hint" <<'PY'
import datetime
import re
import sys
from pathlib import Path

out_path, verdict_path, files_path, scope_path, log_path, task, round_no, verdict_hint = sys.argv[1:9]

def read(path):
    try:
        return Path(path).read_text(encoding="utf-8", errors="replace")
    except Exception:
        return ""

verdict_text = read(verdict_path)
status = verdict_hint.strip().upper()
if status not in {"PASS", "FAIL", "BLOCKED"}:
    for line in verdict_text.splitlines():
        m = re.match(r"^\s*(PASS|FAIL|BLOCKED)\b", line, re.I)
        if m:
            status = m.group(1).upper()
            break
if status not in {"PASS", "FAIL", "BLOCKED"}:
    status = "FAIL"

files = [line.strip() for line in read(files_path).splitlines() if line.strip()]
scope_text = read(scope_path)
log_text = read(log_path) if log_path else ""

def compact(line, limit=220):
    line = re.sub(r"\s+", " ", line.strip())
    return line if len(line) <= limit else line[: limit - 3] + "..."

noise = re.compile(
    r"(session id:|using existing .* session|created new .* session|tokens used|total[_ ]tokens|"
    r"codex model at capacity|waiting .* before retrying|^user$|^assistant$)",
    re.I,
)
interesting = re.compile(
    r"(FAIL|BLOCKED|required|must|fix|missing|incorrect|wrong|regression|scope|out-of-scope|"
    r"validation|test|cargo|npm|pnpm|pytest|file|line|diff|acceptance)",
    re.I,
)

required = []
for source in [verdict_text, "\n".join(log_text.splitlines()[-500:])]:
    for raw in source.splitlines():
        line = compact(raw)
        if not line or noise.search(line):
            continue
        if interesting.search(line):
            required.append(line)

seen = set()
required_unique = []
for line in required:
    key = line.lower()
    if key in seen:
        continue
    seen.add(key)
    required_unique.append(line)
    if len(required_unique) >= 36:
        break

scope_violations = []
for raw in (verdict_text + "\n" + scope_text).splitlines():
    line = compact(raw)
    if re.search(r"(out-of-scope|scopeguard|scope violation|forbidden)", line, re.I):
        scope_violations.append(line)
scope_violations = list(dict.fromkeys(scope_violations))[:20]

tests = []
for raw in (verdict_text + "\n" + log_text).splitlines():
    line = compact(raw)
    if re.search(r"(test|validation|cargo|npm|pnpm|yarn|pytest|vitest|playwright|check)", line, re.I):
        tests.append(line)
tests = list(dict.fromkeys(tests))[:20]

if status == "PASS":
    required_unique = ["No required fixes; reviewer passed."]
    scope_violations = scope_violations or ["None reported."]
    tests = tests or ["No additional tests required by reviewer."]
else:
    required_unique = required_unique or ["Reviewer failed, but no concise finding was parsed. Re-read reviewer-diff.patch and current task before repairing."]
    scope_violations = scope_violations or ["None reported."]
    tests = tests or ["Re-run the validation commands listed in the task after applying required fixes."]

failed_files = files if status != "PASS" else []
lines = [
    "# Reviewer Summary",
    "",
    f"Generated: {datetime.datetime.now(datetime.timezone.utc).astimezone().isoformat(timespec='seconds')}",
    f"Task: {task}",
    f"Round: {round_no}",
    f"Status: {status}",
    "",
    "## Failed Files",
]
lines.extend([f"- {item}" for item in failed_files] or ["- None"])
lines.extend(["", "## Required Fixes"])
lines.extend([f"- {item}" for item in required_unique])
lines.extend(["", "## Scope Violations"])
lines.extend([f"- {item}" for item in scope_violations])
lines.extend(["", "## Required Tests"])
lines.extend([f"- {item}" for item in tests])
lines.append("")

Path(out_path).parent.mkdir(parents=True, exist_ok=True)
Path(out_path).write_text("\n".join(lines), encoding="utf-8")
PY
}

run_task_agent_role() {
  local model="${1:-}"
  local role="${2:-}"
  if [[ -z "$model" || -z "$role" ]]; then
    echo "BUG: run_task_agent_role requires model and role" >&2
    exit 2
  fi
  local cli
  cli="$(agent_cli_for_role "$role")" || exit $?
  local prompt_file="$RUNTIME/merged-${role}.prompt.md"
  local effort=""
  local repair_mode="${REPAIR_MODE:-false}"
  if declare -F agent_task_adaptive_apply >/dev/null 2>&1; then
    agent_task_adaptive_apply "$role" "$AI_DIR" "$PROJECT_ROOT" >/dev/null || true
  fi
  case "$role" in
    coder) effort="${CODER_LEVEL:-high}" ;;
    reviewer) effort="${REVIEWER_LEVEL:-low}" ;;
    reviewer-final) effort="${FINAL_REVIEWER_LEVEL:-xhigh}" ;;
    planner) effort="${PLANNER_LEVEL:-xhigh}" ;;
  esac
  if [[ "$role" == "coder" && "${round:-1}" -gt 1 ]]; then
    repair_mode="true"
  fi
  if [[ -x "$AI_DIR/scripts/agent-context-package.sh" ]]; then
    CONTEXT_ROLE="$role" REPAIR_MODE="$repair_mode" CURRENT_TASK="$TASK" bash "$AI_DIR/scripts/agent-context-package.sh" >/dev/null || true
  fi
  {
    echo "# Invocation Context"
    echo
    echo "- Role: $role"
    echo "- Current task path: $TASK"
    echo "- Round: $round / $MAX_ROUNDS"
    echo "- Compact context package: .ai-agent/generated/runtime/context-package.md"
    echo "- Search allowlist: .ai-agent/generated/runtime/search-allowlist.txt"
    echo "- Runtime context: .ai-agent/generated/runtime/runtime-context.md"
    echo "- Repair mode: $repair_mode"
    echo "- FAST_REVIEW_UNIT_TEST: $FAST_REVIEW_UNIT_TEST"
    echo "- FAST_REVIEW_ARCHITECTURE: $FAST_REVIEW_ARCHITECTURE"
    echo "- If this is a resumed persistent session, ignore previous task context unless the current task explicitly references it."
    echo "- The current task path above is authoritative; do not act on a task remembered from earlier session history."
    echo
    echo "---"
    echo
    if [[ "${INLINE_CONTEXT_PACKAGE:-false}" == "true" && -s "$RUNTIME/context-package.md" ]]; then
      cat "$RUNTIME/context-package.md"
      echo
      echo "# End Compact Context Package"
      echo
      echo "---"
      echo
    fi
    "$AI_DIR/bin/aia" prompt "$role"
  } > "$prompt_file"
  event "$role" "started cli=$cli model=$model effort=${effort:-default}"
  write_status "$role" "$TASK" "$round" "Running $role with $cli/$model" ""
  local output_log="$RUNTIME/${role}-round-${round}.log"
  local code=0
  : > "$output_log"
  set +e
  CODEX_SESSION_SCOPE_ID="$TASK" run_agent_role "$role" "$prompt_file" "$model" "$cli" "$CODEX_TIMEOUT" "$output_log" "$effort" "$SANDBOX"
  code=$?
  set -e
  record_token_usage "$role" "$model" "$round" "$output_log"
  if [[ "$code" -ne 0 ]]; then
    event "$role" "failed exit=$code"
    return "$code"
  fi
  event "$role" "finished"
}

TASK="$(find_current_task || true)"
if [[ -z "$TASK" || ! -f "$TASK" ]]; then
  event "loop" "no current task found"; write_status "idle" "" 0 "No current task found" ""; echo "No current task found." >&2; exit 1
fi

if [[ "$TASK_BASE_COMMIT" == "true" ]]; then TASK_BASE_SHA="$(git rev-parse HEAD 2>/dev/null || true)"; fi
write_verdict_file "PENDING"
event "task" "started $TASK base=$TASK_BASE_SHA"
write_status "task" "$TASK" 0 "Starting current task" ""

if [[ "$PRE_TASK_SCOPE_GATE" == "true" && ( "$STRICT_SCOPE" == "true" || "$REVIEW_ALLOWED_ONLY" == "true" ) ]]; then
  echo "== ScopeGuard: pre-task dirty check =="
  write_status "ScopeGuard" "$TASK" 0 "Checking existing diff before coder" ""
  if declare -F scope_guard_preflight >/dev/null 2>&1; then
    if ! scope_guard_preflight "$TASK" 0; then
      write_status "blocked" "$TASK" 0 "Existing out-of-scope implementation diff before coder" "BLOCKED"
      echo "ScopeGuard blocked before coder. See $VERDICT_FILE" >&2
      event "ScopeGuard" "blocked before coder"
      exit 1
    fi
  else
    echo "Scope guard script missing; pre-task scope gate cannot run." >&2
    printf 'BLOCKED\nScopeGuard script missing\n' > "$VERDICT_FILE"
    write_status "blocked" "$TASK" 0 "ScopeGuard script missing" "BLOCKED"
    exit 1
  fi
fi

mark_task_status "$TASK" "In Progress"

for round in $(seq 1 "$MAX_ROUNDS"); do
  write_verdict_file "PENDING"
  echo "== Round $round/$MAX_ROUNDS =="
  event "round" "$round/$MAX_ROUNDS started"
  write_status "context" "$TASK" "$round" "Building runtime context" ""
  if declare -F agent_task_adaptive_apply >/dev/null 2>&1; then
    agent_task_adaptive_apply coder "$AI_DIR" "$PROJECT_ROOT" >/dev/null || true
  fi
  if [[ -x "$AI_DIR/scripts/agent-context-build.sh" ]]; then
    CONTEXT_MODE="$CONTEXT_MODE" CODEGRAPH_MODE="$CODEGRAPH_MODE" CURRENT_TASK="$TASK" bash "$AI_DIR/scripts/agent-context-build.sh" >/dev/null || true
  fi

  echo "== Coder: $CODER_MODEL (${CODER_CLI}) =="
  run_task_agent_role "$CODER_MODEL" "coder"

  if [[ "$STRICT_SCOPE" == "true" || "$REVIEW_ALLOWED_ONLY" == "true" ]]; then
    echo "== ScopeGuard: checking allowed diff =="
    write_status "ScopeGuard" "$TASK" "$round" "Checking diff scope before reviewer" ""
    if declare -F scope_guard_check >/dev/null 2>&1; then
      if ! scope_guard_check "$TASK" "$round"; then
        mark_task_status "$TASK" "Failed"
        write_reviewer_summary "$round" "" "FAIL"
        echo "ScopeGuard failed before reviewer. See $VERDICT_FILE" >&2
        event "ScopeGuard" "failed before reviewer"
        continue
      fi
    else
      echo "Scope guard script missing; strict scope cannot run." >&2
      printf 'FAIL\nScopeGuard script missing\n' > "$VERDICT_FILE"
      mark_task_status "$TASK" "Failed"
      continue
    fi
  fi

  write_status "context" "$TASK" "$round" "Refreshing context for reviewer" ""
  if declare -F agent_task_adaptive_apply >/dev/null 2>&1; then
    agent_task_adaptive_apply reviewer "$AI_DIR" "$PROJECT_ROOT" >/dev/null || true
  fi
  if [[ -x "$AI_DIR/scripts/agent-context-build.sh" ]]; then
    CONTEXT_MODE=minimal CODEGRAPH_MODE="$CODEGRAPH_MODE" CURRENT_TASK="$TASK" bash "$AI_DIR/scripts/agent-context-build.sh" >/dev/null || true
  fi
  write_verdict_file "PENDING"
  if declare -F prepare_reviewer_diff >/dev/null 2>&1; then
    prepare_reviewer_diff "$TASK" "$round"
  fi
  echo "== Fast Reviewer: $REVIEWER_MODEL (${REVIEWER_CLI}) =="
  if [[ "$FAST_REVIEW_UNIT_TEST" == "true" ]]; then
    echo "  [review-scope] FAST_REVIEW_UNIT_TEST=true — unit test enabled in Fast Reviewer"
    event "fast-reviewer" "unit_test=enabled"
  else
    echo "  [review-scope] FAST_REVIEW_UNIT_TEST=false — unit test skipped in Fast Reviewer (deferred to Final Reviewer)"
    event "fast-reviewer" "unit_test=skipped"
  fi
  if [[ "$FAST_REVIEW_ARCHITECTURE" == "true" ]]; then
    echo "  [review-scope] FAST_REVIEW_ARCHITECTURE=true — architecture review enabled in Fast Reviewer"
    event "fast-reviewer" "architecture=enabled"
  else
    echo "  [review-scope] FAST_REVIEW_ARCHITECTURE=false — architecture review skipped in Fast Reviewer (deferred to Final Reviewer)"
    event "fast-reviewer" "architecture=skipped"
  fi
  run_task_agent_role "$REVIEWER_MODEL" "reviewer"
  write_reviewer_summary "$round" "$RUNTIME/reviewer-round-${round}.log" ""

  verdict="$(read_latest_verdict || true)"
  normalize_verdict_file "${verdict:-FAIL}"
  event "reviewer" "verdict=${verdict:-unknown}"
  if [[ "$verdict" == "PASS" ]]; then
    echo "PASS"
    if [[ "$AUTO_MARK_TASK_PASSED" == "true" ]]; then mark_task_status "$TASK" "Passed"; fi
    if [[ "$TASK_CHECKPOINT_ON_PASS" == "true" ]] && declare -F scope_guard_checkpoint >/dev/null 2>&1; then
      scope_guard_checkpoint "$TASK" "$round"
    fi
    event "task" "passed $TASK"
    write_status "passed" "$TASK" "$round" "Task passed and was marked Passed" "PASS"
    if [[ "$AUTO_COMMIT" == "true" ]]; then
      git add -A -- . ':(exclude).ai-agent/**' ':(exclude).agent/loop-verdict.txt' ':(exclude)AGENTS.md' ':(exclude).gitignore'
      git add "$TASK" 2>/dev/null || true
      git commit -m "agent: complete $(basename "$TASK" .md)" || true
    fi
    exit 0
  fi

  mark_task_status "$TASK" "Failed"
  write_status "failed" "$TASK" "$round" "Reviewer did not pass; continuing if rounds remain" "${verdict:-FAIL}"
  echo "Reviewer did not pass. Continuing if rounds remain."
done

write_status "failed" "$TASK" "$MAX_ROUNDS" "FAILED after $MAX_ROUNDS rounds" "FAIL"
event "task" "failed after $MAX_ROUNDS rounds $TASK"
echo "FAILED after $MAX_ROUNDS rounds. See $VERDICT_FILE" >&2
exit 1
