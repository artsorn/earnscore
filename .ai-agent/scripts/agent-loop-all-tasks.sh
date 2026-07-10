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
MAX_TASKS="${MAX_TASKS:-100}"
PLANNER_CLI="${PLANNER_CLI:-codex}"
CODER_CLI="${CODER_CLI:-codex}"
REVIEWER_CLI="${REVIEWER_CLI:-codex}"
FINAL_REVIEWER_CLI="${FINAL_REVIEWER_CLI:-codex}"
FINAL_REVIEWER_MODEL="${FINAL_REVIEWER_MODEL:-gpt-5.6-sol}"
PLANNER_MODEL="${PLANNER_MODEL:-gpt-5.6-sol}"
CODER_MODEL="${CODER_MODEL:-${CODER_SESSION_MODEL:-gpt-5.6-sol}}"
REVIEWER_MODEL="${REVIEWER_MODEL:-${REVIEWER_SESSION_MODEL:-gpt-5.6-sol}}"
PLANNER_LEVEL="${PLANNER_LEVEL:-xhigh}"
RUN_FINAL_REVIEW="${RUN_FINAL_REVIEW:-true}"
CODEX_TIMEOUT="${CODEX_TIMEOUT:-180m}"
AGY_TIMEOUT="${AGY_TIMEOUT:-30m}"
AGY_SANDBOX="${AGY_SANDBOX:-false}"
AGY_SKIP_PERMISSIONS="${AGY_SKIP_PERMISSIONS:-true}"
AGY_PROJECT="${AGY_PROJECT:-}"
AGY_CONVERSATION="${AGY_CONVERSATION:-}"
AGY_CONTINUE="${AGY_CONTINUE:-false}"
AGY_ADD_DIRS="${AGY_ADD_DIRS:-}"
FINAL_REVIEW_TIMEOUT="${FINAL_REVIEW_TIMEOUT:-$CODEX_TIMEOUT}"
PLANNER_FINAL_FIX_TIMEOUT="${PLANNER_FINAL_FIX_TIMEOUT:-${PLANNER_TIMEOUT:-$CODEX_TIMEOUT}}"
FINAL_REVIEWER_LEVEL="${FINAL_REVIEWER_LEVEL:-xhigh}"
FINAL_REVIEWER_FAIL_MODE="${FINAL_REVIEWER_FAIL_MODE:-stop}"
FINAL_REVIEWER_FIX_ROUNDS="${FINAL_REVIEWER_FIX_ROUNDS:-1}"
SANDBOX="${SANDBOX:-workspace-write}"
RUNTIME="$AI_DIR/generated/runtime"
mkdir -p "$RUNTIME"
EVENTS="$RUNTIME/events.log"
# v2.3 review-scope config
FAST_REVIEW_UNIT_TEST="${FAST_REVIEW_UNIT_TEST:-false}"
FAST_REVIEW_ARCHITECTURE="${FAST_REVIEW_ARCHITECTURE:-false}"
FINAL_REVIEW_UNIT_TEST="${FINAL_REVIEW_UNIT_TEST:-true}"
FINAL_REVIEW_ARCHITECTURE="${FINAL_REVIEW_ARCHITECTURE:-true}"

event() { printf '%s [%s] %s\n' "$(date -Iseconds)" "$1" "$2" >> "$EVENTS"; }
die() { echo "ERROR: $*" >&2; exit 2; }

validate_final_fail_mode() {
  case "${1:-}" in
    stop|plan-code|planner-code|review-code) return 0 ;;
    *) die "Invalid FINAL_REVIEWER_FAIL_MODE: ${1:-}. Expected stop, plan-code, or review-code." ;;
  esac
}

normalize_final_fail_mode() {
  case "${1:-}" in
    planner-code) printf 'plan-code' ;;
    *) printf '%s' "${1:-}" ;;
  esac
}

validate_positive_int() {
  local name="$1" value="$2"
  [[ "$value" =~ ^[0-9]+$ && "$value" -ge 1 ]] || die "$name must be a positive integer."
}

record_token_usage() {
  local role="$1" model="$2" log_file="$3"
  [[ -s "$log_file" ]] || return 0
  python3 - "$RUNTIME/token-usage.jsonl" "$log_file" "$role" "$model" <<'PY'
import datetime, json, re, sys
out, log_file, role, model = sys.argv[1:5]
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
    "round": 0,
    "task": "final-review",
    "tokens": tokens,
    "log_file": log_file,
}
with open(out, "a", encoding="utf-8") as f:
    f.write(json.dumps(row, ensure_ascii=False) + "\n")
PY
}

read_final_verdict() {
  python3 - "$RUNTIME/final-verdict.txt" <<'PY'
import re, sys
path = sys.argv[1]
try:
    lines = open(path, encoding="utf-8", errors="replace").read().splitlines()
except OSError:
    sys.exit(0)
latest = ""
for line in lines:
    match = re.match(r"^\s*(PASS|FAIL|BLOCKED)\b", line.strip(), re.I)
    if match:
        latest = match.group(1).upper()
        break
if latest:
    print(latest)
PY
}

run_agent_prompt() {
  local role="$1" model="$2" effort="$3" timeout_value="$4" prompt_file="$5" log_file="$6"
  local cli code=0
  cli="$(agent_cli_for_role "$role")" || return $?
  : > "$log_file"
  event "$role" "started cli=$cli model=$model effort=${effort:-default}"
  set +e
  run_agent_role "$role" "$prompt_file" "$model" "$cli" "$timeout_value" "$log_file" "$effort" "$SANDBOX"
  code=$?
  set -e
  record_token_usage "$role" "$model" "$log_file"
  if [[ "$code" -ne 0 ]]; then
    event "$role" "failed exit=$code"
    return "$code"
  fi
  event "$role" "finished"
}

write_context_preface() {
  local role="$1" repair_mode="${2:-false}"
  if declare -F agent_task_adaptive_apply >/dev/null 2>&1; then
    agent_task_adaptive_apply "$role" "$AI_DIR" "$PROJECT_ROOT" >/dev/null || true
  fi
  if [[ -x "$AI_DIR/scripts/agent-context-package.sh" ]]; then
    CONTEXT_ROLE="$role" REPAIR_MODE="$repair_mode" bash "$AI_DIR/scripts/agent-context-package.sh" >/dev/null || true
  fi
  echo "# Invocation Context"
  echo
  echo "- Role: $role"
  echo "- Compact context package: .ai-agent/generated/runtime/context-package.md"
  echo "- Search allowlist: .ai-agent/generated/runtime/search-allowlist.txt"
  echo "- Runtime context: .ai-agent/generated/runtime/runtime-context.md"
  echo "- Repair mode: $repair_mode"
  echo "- FINAL_REVIEW_UNIT_TEST: $FINAL_REVIEW_UNIT_TEST"
  echo "- FINAL_REVIEW_ARCHITECTURE: $FINAL_REVIEW_ARCHITECTURE"
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
}

write_planner_final_fix_prompt() {
  local round_no="$1"
  local prompt="$RUNTIME/merged-planner-final-fix.prompt.md"
  {
  write_context_preface "planner-final-fix" "true"
  cat <<PROMPT
You are the Planner running in final-fix coding mode.

Reason:
- The Final Reviewer did not pass after all task-level Coder/Reviewer loops completed.
- Runtime setting FINAL_REVIEWER_FAIL_MODE=plan-code authorizes you to edit source code directly until the final goal is satisfied.

Your job:
- Read the final review findings.
- Act as the implementation agent, not as a planning-only agent.
- Inspect the current implementation diff, task scopes, final review findings, and relevant source files.
- Edit source code directly until the project reaches the final goal or this fix round is exhausted.
- Do not hand the work back to Coder.
- Do not only rewrite tasks, create repair plans, or describe work for another agent.
- Do not create new task files.
- Do not split tasks.
- Do not modify .ai-agent/ai-plan/overview.md.
- Do not modify .ai-agent/ai-plan/context.md.
- Do not modify .ai-agent/ai-plan/tasks/*.md.
- Do not run normal Planner repair mode.
- Keep changes minimal and limited to the final review findings.

Required inputs:
- .ai-agent/generated/runtime/final-verdict.txt
- .ai-agent/generated/runtime/runtime-context.md if it exists
- .ai-agent/ai-plan/overview.md
- .ai-agent/ai-plan/context.md
- .ai-agent/ai-plan/tasks/*.md
- git status --short
- .ai-agent/bin/aia task-files
- git diff --stat
- git diff --name-only
- targeted git diff for implementation files

Editing rules:
- You may edit implementation files needed to resolve the final-review findings.
- Obey allowed files, forbidden files, task scope, and no unrelated refactor.
- Do not edit .ai-agent/generated/** except transient runtime files created by commands.
- Do not edit .ai-agent scripts/config/prompts unless the final-review failure is explicitly about the agent framework itself.
- Do not ask Coder to continue.

Before finishing:
- Run required validation commands with real command output evidence, such as cargo test, npm test/frontend checks, scope guard, or direct API validation when required by the tasks.
- Do not claim "verified" without command evidence or a concrete manual validation transcript.
- If a validation command cannot run, record the exact command, output/error, and reason.

Final response:
- Summarize what you changed.
- List validation commands run with real output/results.
- State whether the final goal is satisfied and ready for another Final Reviewer pass.

Final fix round: $round_no / $FINAL_REVIEWER_FIX_ROUNDS
PROMPT
  } > "$prompt"
  echo "$prompt"
}

write_reviewer_final_fix_prompt() {
  local round_no="$1"
  local prompt="$RUNTIME/merged-reviewer-final-fix.prompt.md"
  {
  write_context_preface "reviewer-final-fix" "true"
  cat <<PROMPT
You are the Reviewer running in final-fix coding mode.

Reason:
- The Final Reviewer did not pass after all task-level Coder/Reviewer loops completed.
- Runtime setting FINAL_REVIEWER_FAIL_MODE=review-code authorizes you to edit source code directly until the final goal is satisfied.

Your job:
- Read the final review findings.
- Act as the implementation agent, not as a report-only reviewer.
- Inspect the current implementation diff, task scopes, final review findings, and relevant source files.
- Edit source code directly until the project reaches the final goal or this fix round is exhausted.
- Do not hand the work back to Coder or Planner.
- Do not only write PASS/FAIL.
- Do not only describe problems.
- Fix the exact issues found by Final Reviewer.
- Do not create new task files.
- Do not split tasks.
- Do not modify .ai-agent/ai-plan/overview.md.
- Do not modify .ai-agent/ai-plan/context.md.
- Do not modify .ai-agent/ai-plan/tasks/*.md.
- Keep changes minimal and limited to the final review findings.

Required inputs:
- .ai-agent/generated/runtime/final-verdict.txt
- .ai-agent/generated/runtime/runtime-context.md if it exists
- .ai-agent/ai-plan/overview.md
- .ai-agent/ai-plan/context.md
- .ai-agent/ai-plan/tasks/*.md
- git status --short
- .ai-agent/bin/aia task-files
- git diff --stat
- git diff --name-only
- targeted git diff for implementation files

Editing rules:
- You may edit implementation files needed to resolve the final-review findings.
- Obey allowed files, forbidden files, task scope, and no unrelated refactor.
- Do not edit .ai-agent/generated/** except transient runtime files created by commands.
- Do not edit .ai-agent scripts/config/prompts unless the final-review failure is explicitly about the agent framework itself.

Before finishing:
- Run required validation commands with real command output evidence, such as cargo test, npm test/frontend checks, scope guard, or direct API validation when required by the tasks.
- Do not claim "verified" without command evidence or a concrete manual validation transcript.
- If a validation command cannot run, record the exact command, output/error, and reason.

Final response:
- Summarize what you changed.
- List validation commands run with real output/results.
- State whether the final goal is satisfied and ready for another Final Reviewer pass.

Final fix round: $round_no / $FINAL_REVIEWER_FIX_ROUNDS
PROMPT
  } > "$prompt"
  echo "$prompt"
}

run_final_review_once() {
  local round_label="$1"
  echo "== Final review: $FINAL_REVIEWER_MODEL (${FINAL_REVIEWER_CLI}, $round_label) =="
  if [[ "$FINAL_REVIEW_UNIT_TEST" == "true" ]]; then
    echo "  [review-scope] FINAL_REVIEW_UNIT_TEST=true — unit test ENABLED in Final Reviewer"
    event "final-review" "unit_test=enabled round=$round_label"
  else
    echo "  [review-scope] FINAL_REVIEW_UNIT_TEST=false — unit test SKIPPED in Final Reviewer"
    event "final-review" "unit_test=skipped round=$round_label"
  fi
  if [[ "$FINAL_REVIEW_ARCHITECTURE" == "true" ]]; then
    echo "  [review-scope] FINAL_REVIEW_ARCHITECTURE=true — architecture review ENABLED in Final Reviewer"
    event "final-review" "architecture=enabled round=$round_label"
  else
    echo "  [review-scope] FINAL_REVIEW_ARCHITECTURE=false — architecture review SKIPPED in Final Reviewer"
    event "final-review" "architecture=skipped round=$round_label"
  fi
  event "final-review" "started model=$FINAL_REVIEWER_MODEL round=$round_label"
  : > "$RUNTIME/final-verdict.txt"
  if declare -F agent_task_adaptive_apply >/dev/null 2>&1; then
    agent_task_adaptive_apply reviewer-final "$AI_DIR" "$PROJECT_ROOT" >/dev/null || true
  fi
  CONTEXT_ROLE=reviewer-final CONTEXT_MODE=balanced CODEGRAPH_MODE=strict bash "$AI_DIR/scripts/agent-context-build.sh" >/dev/null || true
  {
    write_context_preface "reviewer-final" "false"
    "$AI_DIR/bin/aia" prompt reviewer-final
  } > "$RUNTIME/merged-reviewer-final.prompt.md"
  local final_log="$RUNTIME/reviewer-final-${round_label}.log"
  run_agent_prompt "reviewer-final" "$FINAL_REVIEWER_MODEL" "$FINAL_REVIEWER_LEVEL" "$FINAL_REVIEW_TIMEOUT" "$RUNTIME/merged-reviewer-final.prompt.md" "$final_log" || return "$?"
  local verdict
  verdict="$(read_final_verdict || true)"
  if [[ -z "$verdict" ]]; then
    verdict="FAIL"
    printf 'FAIL\nFinal reviewer did not write a parseable verdict.\n' > "$RUNTIME/final-verdict.txt"
  fi
  echo "Final reviewer verdict: $verdict"
  event "final-review" "verdict=$verdict round=$round_label"
  if [[ "$verdict" == "PASS" ]]; then
    return 0
  fi
  return 1
}

run_planner_final_fix_once() {
  local round_no="$1"
  local prompt_file log_file
  if declare -F agent_task_adaptive_apply >/dev/null 2>&1; then
    agent_task_adaptive_apply planner-final-fix "$AI_DIR" "$PROJECT_ROOT" >/dev/null || true
  fi
  prompt_file="$(write_planner_final_fix_prompt "$round_no")"
  log_file="$RUNTIME/planner-final-fix-round-${round_no}.log"
  echo "== Planner final-fix coding: $PLANNER_MODEL (${PLANNER_CLI}, round $round_no/$FINAL_REVIEWER_FIX_ROUNDS) =="
  run_agent_prompt "planner-final-fix" "$PLANNER_MODEL" "$PLANNER_LEVEL" "$PLANNER_FINAL_FIX_TIMEOUT" "$prompt_file" "$log_file"
}

run_reviewer_final_fix_once() {
  local round_no="$1"
  local prompt_file log_file reviewer_model reviewer_level
  if declare -F agent_task_adaptive_apply >/dev/null 2>&1; then
    agent_task_adaptive_apply reviewer-final-fix "$AI_DIR" "$PROJECT_ROOT" >/dev/null || true
  fi
  prompt_file="$(write_reviewer_final_fix_prompt "$round_no")"
  log_file="$RUNTIME/reviewer-final-fix-round-${round_no}.log"
  reviewer_model="${REVIEWER_MODEL:-${REVIEWER_SESSION_MODEL:-gpt-5.6-sol}}"
  reviewer_level="${REVIEWER_LEVEL:-low}"
  echo "== Reviewer final-fix coding: $reviewer_model (${REVIEWER_CLI}, round $round_no/$FINAL_REVIEWER_FIX_ROUNDS) =="
  run_agent_prompt "reviewer-final-fix" "$reviewer_model" "$reviewer_level" "$PLANNER_FINAL_FIX_TIMEOUT" "$prompt_file" "$log_file"
}

status_of() { awk '/^## Status/{getline; print; exit}' "$1" 2>/dev/null | tr '[:upper:]' '[:lower:]' | xargs || true; }
next_task() {
  find "$AI_DIR/ai-plan/tasks" -maxdepth 1 -type f -name 'task-*.md' 2>/dev/null | sort | while read -r f; do
    st="$(status_of "$f")"
    case "$st" in
      passed|done|complete|completed) continue ;;
      pending|failed|"in progress"|"pending review"|"") echo "$f"; break ;;
      *) echo "$f"; break ;;
    esac
  done
}

if [[ -x "$AI_DIR/scripts/agent-codegraph.sh" ]]; then bash "$AI_DIR/scripts/agent-codegraph.sh" ensure >/dev/null || true; fi
shopt -s nullglob
task_files=("$AI_DIR"/ai-plan/tasks/task-*.md)
shopt -u nullglob
if [[ "${#task_files[@]}" -eq 0 ]]; then
  die "No task files found. Add the requirement to .agent/requirement.md and run: .ai-agent/bin/aia plan"
fi
count=0
event "loop" "started max_tasks=$MAX_TASKS"

while [[ "$count" -lt "$MAX_TASKS" ]]; do
  task="$(next_task || true)"
  [[ -n "$task" ]] || break
  echo "== Running $task =="
  event "loop" "running $task"
  CURRENT_TASK="$task" bash "$AI_DIR/scripts/agent-loop-current-task.sh"
  count=$((count+1))
  event "loop" "completed count=$count task=$task"
done

event "loop" "tasks loop finished completed_count=$count"

if [[ "$RUN_FINAL_REVIEW" == "true" ]]; then
  validate_final_fail_mode "$FINAL_REVIEWER_FAIL_MODE"
  FINAL_REVIEWER_FAIL_MODE="$(normalize_final_fail_mode "$FINAL_REVIEWER_FAIL_MODE")"
  export FINAL_REVIEWER_FAIL_MODE
  validate_positive_int "FINAL_REVIEWER_FIX_ROUNDS" "$FINAL_REVIEWER_FIX_ROUNDS"
  remaining="$(next_task || true)"
  if [[ -n "$remaining" ]]; then
    echo "== Final review skipped: unfinished task remains: $remaining =="
    event "final-review" "skipped unfinished_task=$remaining"
    exit 0
  fi

  set +e
  run_final_review_once "initial"
  final_status=$?
  set -e
  if [[ "$final_status" -eq 0 ]]; then
    event "final-review" "finished pass"
    exit 0
  elif [[ "$final_status" -ne 1 ]]; then
    event "final-review" "failed exit=$final_status"
    exit "$final_status"
  fi

  if [[ "$FINAL_REVIEWER_FAIL_MODE" == "stop" ]]; then
    event "final-review" "failed mode=stop"
    echo "Final reviewer did not pass. Set FINAL_REVIEWER_FAIL_MODE=plan-code or review-code to let the selected role directly edit code and rerun final review." >&2
    exit 1
  fi

  echo "Final Reviewer failed."
  event "final-review" "failed mode=$FINAL_REVIEWER_FAIL_MODE entering_direct_fix"
  case "$FINAL_REVIEWER_FAIL_MODE" in
    plan-code)
      echo "FINAL_REVIEWER_FAIL_MODE=plan-code"
      echo "Planner will directly edit code until final goal is satisfied."
      echo "Planner will not rewrite task files."
      event "final-review" "FINAL_REVIEWER_FAIL_MODE=plan-code"
      event "final-review" "Planner will directly edit code until final goal is satisfied."
      event "final-review" "Planner will not rewrite task files."
      ;;
    review-code)
      echo "FINAL_REVIEWER_FAIL_MODE=review-code"
      echo "Reviewer will directly edit code until final goal is satisfied."
      echo "Reviewer will not only report issues."
      event "final-review" "FINAL_REVIEWER_FAIL_MODE=review-code"
      event "final-review" "Reviewer will directly edit code until final goal is satisfied."
      event "final-review" "Reviewer will not only report issues."
      ;;
  esac

  for fix_round in $(seq 1 "$FINAL_REVIEWER_FIX_ROUNDS"); do
    case "$FINAL_REVIEWER_FAIL_MODE" in
      plan-code) run_planner_final_fix_once "$fix_round" ;;
      review-code) run_reviewer_final_fix_once "$fix_round" ;;
      *) die "Unsupported FINAL_REVIEWER_FAIL_MODE during fix loop: $FINAL_REVIEWER_FAIL_MODE" ;;
    esac
    set +e
    run_final_review_once "after-${FINAL_REVIEWER_FAIL_MODE}-fix-$fix_round"
    final_status=$?
    set -e
    if [[ "$final_status" -eq 0 ]]; then
      event "final-review" "finished pass after_${FINAL_REVIEWER_FAIL_MODE}_fix_round=$fix_round"
      exit 0
    elif [[ "$final_status" -ne 1 ]]; then
      event "final-review" "failed exit=$final_status after_${FINAL_REVIEWER_FAIL_MODE}_fix_round=$fix_round"
      exit "$final_status"
    fi
  done

  event "final-review" "failed after mode=$FINAL_REVIEWER_FAIL_MODE rounds=$FINAL_REVIEWER_FIX_ROUNDS"
  echo "Final reviewer still did not pass after $FINAL_REVIEWER_FIX_ROUNDS $FINAL_REVIEWER_FAIL_MODE direct fix round(s)." >&2
  echo "Final failure summary is in $RUNTIME/final-verdict.txt" >&2
  exit 1
fi
