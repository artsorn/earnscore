#!/usr/bin/env bash

codex_retry_parse_duration() {
  local value="${1:-}"
  if [[ -z "$value" ]]; then
    echo 0
    return 0
  fi
  case "$value" in
    *[!0-9smh]*)
      echo "Invalid duration: $value" >&2
      return 2
      ;;
  esac
  if [[ "$value" =~ ^([0-9]+)$ ]]; then
    echo "${BASH_REMATCH[1]}"
  elif [[ "$value" =~ ^([0-9]+)s$ ]]; then
    echo "${BASH_REMATCH[1]}"
  elif [[ "$value" =~ ^([0-9]+)m$ ]]; then
    echo $((BASH_REMATCH[1] * 60))
  elif [[ "$value" =~ ^([0-9]+)h$ ]]; then
    echo $((BASH_REMATCH[1] * 3600))
  else
    echo "Invalid duration: $value" >&2
    return 2
  fi
}

codex_retry_format_duration() {
  local seconds="${1:-0}"
  if (( seconds >= 3600 && seconds % 3600 == 0 )); then
    printf '%dh' "$((seconds / 3600))"
  elif (( seconds >= 60 && seconds % 60 == 0 )); then
    printf '%dm' "$((seconds / 60))"
  else
    printf '%ds' "$seconds"
  fi
}

codex_retry_log_event() {
  local stage="${1:-codex-retry}" msg="${2:-}"
  if [[ -n "${EVENTS:-}" ]]; then
    printf '%s [%s] %s\n' "$(date -Iseconds)" "$stage" "$msg" >> "$EVENTS"
  fi
}

codex_retry_last_error_is_capacity() {
  local log_file="$1"
  [[ -s "$log_file" ]] || return 1
  tail -n 80 "$log_file" | grep -qiE 'Selected model is at capacity|Please try a different model'
}

codex_prompt_guard_file() {
  local prompt_file="$1" log_file="$2" role="$3"
  [[ "${TOKEN_GUARD:-true}" == "true" ]] || return 0
  [[ -f "$prompt_file" ]] || return 0
  python3 - "$prompt_file" "${MAX_CONTEXT_TOKENS:-50000}" "$role" <<'PY' | tee -a "$log_file"
import re
import sys
from pathlib import Path

path = Path(sys.argv[1])
try:
    max_tokens = max(4000, int(sys.argv[2]))
except ValueError:
    max_tokens = 50000
role = sys.argv[3]

text = path.read_text(encoding="utf-8", errors="replace")

def estimate(value):
    return max(1, (len(value) + 3) // 4)

initial = estimate(text)
steps = []
if initial > max_tokens:
    start = text.find("# Compact Context Package")
    if start >= 0:
        marker = "\n# End Compact Context Package\n"
        end = text.find(marker, start)
        if end >= 0:
            end += len(marker)
        else:
            end = text.find("\n---\n", start)
        if end < 0:
            end = len(text)
        package = text[start:end]
        rest = text[end:]
        for section in [
            "Relevant Source Snippets",
            "Relevant Knowledge",
            "Relevant Codegraph",
            "Runtime Context Excerpt",
        ]:
            if estimate(package + rest) <= max_tokens:
                break
            pattern = r"\n## " + re.escape(section) + r"\n.*?(?=\n## |\Z)"
            package, count = re.subn(pattern, "\n## " + section + "\n\n_Trimmed by final token guard; use allowlisted targeted files only if needed._\n", package, count=1, flags=re.S)
            if count:
                steps.append(section)
        text = text[:start] + package + rest
        if estimate(text) > max_tokens:
            pattern = r"(\n## Reviewer Diff Excerpt\n.*?```diff\n)(.*?)(```)"
            def trim_diff(match):
                body = match.group(2).splitlines()
                if len(body) <= 260:
                    return match.group(0)
                steps.append("Reviewer Diff Excerpt")
                return match.group(1) + "\n".join(body[:260]) + "\n...[trimmed by final token guard]\n" + match.group(3)
            text = re.sub(pattern, trim_diff, text, count=1, flags=re.S)
        path.write_text(text, encoding="utf-8")

final = estimate(text)
print(f"Input context estimate for {role}: {final:,} tokens (limit {max_tokens:,}).")
if steps:
    print("Token guard trimmed optional package sections: " + ", ".join(dict.fromkeys(steps)))
elif initial > max_tokens and final > max_tokens:
    print("Token guard warning: required prompt sections still exceed MAX_CONTEXT_TOKENS.")
PY
}

codex_session_key_for_role() {
  case "${1:-codex}" in
    planner|planner-final-fix) echo "planner" ;;
    coder) echo "coder" ;;
    reviewer|reviewer-final-fix) echo "reviewer" ;;
    reviewer-final|final-reviewer|final_reviewer) echo "final_reviewer" ;;
    *) echo "${1:-codex}" | tr '[:upper:]-' '[:lower:]_' ;;
  esac
}

codex_session_env_for_key() {
  case "${1:-}" in
    planner) echo "PLANNER_SESSION" ;;
    coder) echo "CODER_SESSION" ;;
    reviewer) echo "REVIEWER_SESSION" ;;
    final_reviewer) echo "FINAL_REVIEWER_SESSION" ;;
    *) echo "" ;;
  esac
}

codex_session_display_for_key() {
  case "${1:-}" in
    planner) echo "Planner" ;;
    coder) echo "Coder" ;;
    reviewer) echo "Reviewer" ;;
    final_reviewer) echo "FinalReviewer" ;;
    *) echo "${1:-Codex}" ;;
  esac
}

codex_session_file_for_key() {
  local key="$1" log_file="$2" base_dir
  if [[ -n "${CODEX_SESSION_DIR:-}" ]]; then
    base_dir="$CODEX_SESSION_DIR"
  elif [[ -n "${RUNTIME:-}" ]]; then
    base_dir="$RUNTIME/sessions"
  elif [[ -n "${RUNTIME_DIR:-}" ]]; then
    base_dir="$RUNTIME_DIR/sessions"
  else
    base_dir="$(dirname "$log_file")/sessions"
  fi
  if [[ "${PERSISTENT_SESSION_SCOPE:-task}" == "task" && -n "${CODEX_SESSION_SCOPE_ID:-}" ]]; then
    local scope_slug="${CODEX_SESSION_SCOPE_ID##*/}"
    scope_slug="${scope_slug%.md}"
    scope_slug="${scope_slug//[^A-Za-z0-9_.-]/_}"
    [[ -n "$scope_slug" ]] || scope_slug="task"
    base_dir="$base_dir/tasks/$scope_slug"
  fi
  mkdir -p "$base_dir"
  echo "$base_dir/${key}_session_id.txt"
}

codex_session_is_valid() {
  [[ "${1:-}" =~ ^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$ ]]
}

codex_session_short() {
  local id="${1:-}"
  if [[ "${#id}" -gt 12 ]]; then
    printf '%s...%s' "${id:0:8}" "${id: -4}"
  else
    printf '%s' "$id"
  fi
}

codex_extract_session_id_from_log() {
  local log_file="$1"
  [[ -s "$log_file" ]] || return 1
  python3 - "$log_file" <<'PY'
import json, re, sys
path = sys.argv[1]
latest = ""
uuid_re = re.compile(r"[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}")
with open(path, encoding="utf-8", errors="replace") as f:
    for line in f:
        text = line.strip()
        if text == "user":
            break
        match = re.match(r"^session id:\s*(" + uuid_re.pattern + r")\s*$", text, re.I)
        if match:
            print(match.group(1))
            raise SystemExit
with open(path, encoding="utf-8", errors="replace") as f:
    for line in f:
        text = line.strip()
        if not text:
            continue
        if (
            text.startswith("codex resume ")
            or text.startswith("codex exec resume ")
            or text.startswith("To continue this session")
        ):
            matches = uuid_re.findall(text)
            if matches:
                latest = matches[-1]
        try:
            data = json.loads(text)
        except Exception:
            continue
        stack = [data]
        while stack:
            item = stack.pop()
            if isinstance(item, dict):
                for key, value in item.items():
                    if key in {"session_id", "conversation_id", "id"} and isinstance(value, str):
                        matches = uuid_re.findall(value)
                        if matches:
                            latest = matches[-1]
                    elif isinstance(value, (dict, list)):
                        stack.append(value)
            elif isinstance(item, list):
                stack.extend(item)
if latest:
    print(latest)
PY
}

codex_session_resolve() {
  local role="$1" log_file="$2" key env_name env_value file file_value display
  CODEX_SESSION_KEY="$(codex_session_key_for_role "$role")"
  display="$(codex_session_display_for_key "$CODEX_SESSION_KEY")"
  CODEX_SESSION_FILE="$(codex_session_file_for_key "$CODEX_SESSION_KEY" "$log_file")"
  CODEX_SESSION_ID=""
  CODEX_SESSION_SOURCE="new"
  env_name="$(codex_session_env_for_key "$CODEX_SESSION_KEY")"
  if [[ -n "$env_name" ]]; then
    env_value="${!env_name:-}"
    if codex_session_is_valid "$env_value"; then
      CODEX_SESSION_ID="$env_value"
      CODEX_SESSION_SOURCE="env:$env_name"
      printf 'Using explicit %s from env: %s\n' "$env_name" "$(codex_session_short "$CODEX_SESSION_ID")" | tee -a "$log_file"
      return 0
    elif [[ -n "$env_value" ]]; then
      printf 'Ignoring invalid %s from env.\n' "$env_name" | tee -a "$log_file"
    fi
  fi
  file="$CODEX_SESSION_FILE"
  if [[ -f "$file" ]]; then
    file_value="$(tr -d '[:space:]' < "$file" 2>/dev/null || true)"
    if codex_session_is_valid "$file_value"; then
      CODEX_SESSION_ID="$file_value"
      CODEX_SESSION_SOURCE="file"
      printf 'Using existing %s session: %s\n' "$display" "$(codex_session_short "$CODEX_SESSION_ID")" | tee -a "$log_file"
      return 0
    fi
    printf 'Ignoring invalid saved %s session file: %s\n' "$CODEX_SESSION_KEY" "$file" | tee -a "$log_file"
  fi
  printf 'No saved %s session found; creating a new session.\n' "$display" | tee -a "$log_file"
}

codex_session_save_created() {
  local role="$1" log_file="$2" session_id display
  [[ "${USE_PERSISTENT_SESSIONS:-true}" == "true" ]] || return 0
  [[ "${CODEX_SESSION_SOURCE:-new}" == "new" ]] || return 0
  session_id="$(codex_extract_session_id_from_log "$log_file" || true)"
  if codex_session_is_valid "$session_id"; then
    display="$(codex_session_display_for_key "$CODEX_SESSION_KEY")"
    mkdir -p "$(dirname "$CODEX_SESSION_FILE")"
    printf '%s\n' "$session_id" > "$CODEX_SESSION_FILE"
    printf 'Created new %s session: %s\n' "$display" "$(codex_session_short "$session_id")" | tee -a "$log_file"
    codex_retry_log_event "$role" "created session key=$CODEX_SESSION_KEY id=$(codex_session_short "$session_id")"
  else
    printf 'WARNING: could not capture new %s session id from Codex output.\n' "$CODEX_SESSION_KEY" | tee -a "$log_file"
    codex_retry_log_event "$role" "could not capture session id key=$CODEX_SESSION_KEY"
  fi
}

codex_run_once() {
  local model="$1" effort="$2" sandbox="$3" timeout_value="$4" prompt_file="$5" log_file="$6" role="$7"
  local code
  codex_prompt_guard_file "$prompt_file" "$log_file" "$role"

  if [[ "${USE_PERSISTENT_SESSIONS:-true}" != "true" ]]; then
    printf 'USE_PERSISTENT_SESSIONS=false; starting fresh %s session for this invocation.\n' "$role" | tee -a "$log_file"
    set +e
    if [[ -n "$effort" ]]; then
      timeout "$timeout_value" codex exec -m "$model" -c "model_reasoning_effort=$effort" -s "$sandbox" - < "$prompt_file" 2>&1 | tee -a "$log_file"
      code=${PIPESTATUS[0]}
    else
      timeout "$timeout_value" codex exec -m "$model" -s "$sandbox" - < "$prompt_file" 2>&1 | tee -a "$log_file"
      code=${PIPESTATUS[0]}
    fi
    set -e
    return "$code"
  fi

  codex_session_resolve "$role" "$log_file"
  set +e
  if [[ -n "${CODEX_SESSION_ID:-}" ]]; then
    if [[ -n "$effort" ]]; then
      timeout "$timeout_value" codex exec resume -m "$model" -c "model_reasoning_effort=$effort" "$CODEX_SESSION_ID" - < "$prompt_file" 2>&1 | tee -a "$log_file"
      code=${PIPESTATUS[0]}
    else
      timeout "$timeout_value" codex exec resume -m "$model" "$CODEX_SESSION_ID" - < "$prompt_file" 2>&1 | tee -a "$log_file"
      code=${PIPESTATUS[0]}
    fi
  else
    if [[ -n "$effort" ]]; then
      timeout "$timeout_value" codex exec -m "$model" -c "model_reasoning_effort=$effort" -s "$sandbox" - < "$prompt_file" 2>&1 | tee -a "$log_file"
      code=${PIPESTATUS[0]}
    else
      timeout "$timeout_value" codex exec -m "$model" -s "$sandbox" - < "$prompt_file" 2>&1 | tee -a "$log_file"
      code=${PIPESTATUS[0]}
    fi
  fi
  set -e
  if [[ "$code" -eq 0 ]]; then
    codex_session_save_created "$role" "$log_file"
  fi
  return "$code"
}

agent_role_config_key() {
  case "${1:-}" in
    planner|planner-final-fix) echo "PLANNER" ;;
    coder) echo "CODER" ;;
    reviewer|reviewer-final-fix) echo "REVIEWER" ;;
    reviewer-final|final-reviewer|final_reviewer) echo "FINAL_REVIEWER" ;;
    *) echo "${1:-AGENT}" | tr '[:lower:]-' '[:upper:]_' ;;
  esac
}

agent_validate_cli_name() {
  case "${1:-}" in
    codex|agy) return 0 ;;
    *) echo "Unsupported AI CLI: ${1:-}. Expected codex or agy." >&2; return 2 ;;
  esac
}

agent_cli_for_role() {
  local key cli
  key="$(agent_role_config_key "$1")"
  cli="${key}_CLI"
  cli="${!cli:-codex}"
  agent_validate_cli_name "$cli" || return $?
  printf '%s' "$cli"
}

agent_role_default_model() {
  case "$(agent_role_config_key "$1")" in
    *) echo "gpt-5.6-sol" ;;
  esac
}

agy_role_default_model() {
  case "$(agent_role_config_key "$1")" in
    PLANNER|FINAL_REVIEWER) echo "Gemini 3.1 Pro (High)" ;;
    CODER) echo "Gemini 3.5 Flash (Low)" ;;
    REVIEWER) echo "Gemini 3.5 Flash (Medium)" ;;
    *) echo "Gemini 3.5 Flash (Medium)" ;;
  esac
}

agy_normalize_model() {
  local role="$1" value="${2:-}"
  case "${value,,}" in
    ""|gpt-5.*|o[0-9]*|codex*) agy_role_default_model "$role" ;;
    gemini-3.5-flash|"gemini 3.5 flash") agy_role_default_model "$role" ;;
    gemini-3.5-flash-low|"gemini 3.5 flash (low)") echo "Gemini 3.5 Flash (Low)" ;;
    gemini-3.5-flash-medium|"gemini 3.5 flash (medium)") echo "Gemini 3.5 Flash (Medium)" ;;
    gemini-3.5-flash-high|"gemini 3.5 flash (high)") echo "Gemini 3.5 Flash (High)" ;;
    gemini-3.1-pro|gemini-3.1-pro-high|"gemini 3.1 pro"|"gemini 3.1 pro (high)") echo "Gemini 3.1 Pro (High)" ;;
    gemini-3.1-pro-low|"gemini 3.1 pro (low)") echo "Gemini 3.1 Pro (Low)" ;;
    claude-sonnet-4.6-thinking|"claude sonnet 4.6 (thinking)") echo "Claude Sonnet 4.6 (Thinking)" ;;
    claude-opus-4.6-thinking|"claude opus 4.6 (thinking)") echo "Claude Opus 4.6 (Thinking)" ;;
    gpt-oss-120b-medium|"gpt-oss 120b (medium)") echo "GPT-OSS 120B (Medium)" ;;
    *) printf '%s\n' "$value" ;;
  esac
}

agy_role_model() {
  local role="$1" generic_model="${2:-}" key role_agy_model
  key="$(agent_role_config_key "$role")"
  role_agy_model="${key}_AGY_MODEL"
  if [[ -n "${!role_agy_model:-}" ]]; then
    agy_normalize_model "$role" "${!role_agy_model}"
  elif [[ -n "$generic_model" ]]; then
    agy_normalize_model "$role" "$generic_model"
  else
    agy_role_default_model "$role"
  fi
}

agent_effective_model_for_role() {
  local role="$1" model="${2:-}" cli="${3:-}"
  [[ -n "$cli" ]] || cli="$(agent_cli_for_role "$role")" || return $?
  case "$cli" in
    agy) agy_role_model "$role" "$model" ;;
    codex) printf '%s\n' "${model:-$(agent_role_default_model "$role")}" ;;
  esac
}

agy_role_project() {
  local key role_project
  key="$(agent_role_config_key "$1")"
  role_project="${key}_AGY_PROJECT"
  printf '%s' "${!role_project:-${AGY_PROJECT:-}}"
}

agy_role_conversation() {
  local key role_conversation
  key="$(agent_role_config_key "$1")"
  role_conversation="${key}_AGY_CONVERSATION"
  printf '%s' "${!role_conversation:-${AGY_CONVERSATION:-}}"
}

agy_add_dirs_to_cmd() {
  local -n _cmd_ref="$1"
  local value="${AGY_ADD_DIRS:-}" item old_ifs
  [[ -n "$value" ]] || return 0
  old_ifs="$IFS"
  if [[ "$value" == *:* ]]; then
    IFS=':'
  else
    IFS=$' \t\n'
  fi
  # shellcheck disable=SC2206
  local items=( $value )
  IFS="$old_ifs"
  for item in "${items[@]}"; do
    [[ -n "$item" ]] || continue
    _cmd_ref+=(--add-dir "$item")
  done
}

run_codex_role() {
  local role="$1" prompt_file="$2" model="$3" timeout_value="$4" log_file="$5" effort="${6:-}" sandbox="${7:-${SANDBOX:-workspace-write}}"
  if [[ -z "$role" || -z "$prompt_file" || -z "$model" || -z "$timeout_value" || -z "$log_file" ]]; then
    echo "run_codex_role: role, prompt-file, model, timeout, and log-file are required" >&2
    return 2
  fi
  if ! command -v codex >/dev/null 2>&1; then
    echo "codex command not found. Install Codex CLI or select a different role CLI." >&2
    return 127
  fi
  CODEX_SESSION_SCOPE_ID="${CODEX_SESSION_SCOPE_ID:-${TASK:-}}" run_codex_exec_with_capacity_retry \
    --model "$model" \
    --effort "$effort" \
    --sandbox "$sandbox" \
    --timeout "$timeout_value" \
    --prompt-file "$prompt_file" \
    --log-file "$log_file" \
    --role "$role"
}

run_agy_role() {
  local role="$1" prompt_file="$2" model="$3" timeout_value="$4" log_file="$5"
  if [[ -z "$role" || -z "$prompt_file" || -z "$timeout_value" || -z "$log_file" ]]; then
    echo "run_agy_role: role, prompt-file, timeout, and log-file are required" >&2
    return 2
  fi
  if [[ ! -f "$prompt_file" ]]; then
    echo "run_agy_role: prompt file not found: $prompt_file" >&2
    return 2
  fi
  if ! command -v agy >/dev/null 2>&1; then
    echo "agy command not found. Install Google Antigravity CLI or select a different role CLI." >&2
    return 127
  fi

  local agy_model project conversation prompt prompt_bytes max_inline code prompt_dir agy_cli_log
  agy_model="$(agy_role_model "$role" "$model")"
  project="$(agy_role_project "$role")"
  conversation="$(agy_role_conversation "$role")"
  prompt_bytes="$(wc -c < "$prompt_file" | tr -d '[:space:]')"
  max_inline="${AGY_INLINE_PROMPT_MAX_BYTES:-120000}"

  local cmd=(agy)
  [[ -n "$agy_model" ]] && cmd+=(--model "$agy_model")
  cmd+=(--print-timeout "${AGY_TIMEOUT:-$timeout_value}")
  [[ -n "${AGY_MODE:-}" ]] && cmd+=(--mode "$AGY_MODE")
  [[ "${AGY_SKIP_PERMISSIONS:-true}" == "true" ]] && cmd+=(--dangerously-skip-permissions)
  [[ "${AGY_SANDBOX:-false}" == "true" ]] && cmd+=(--sandbox)
  [[ -n "$project" ]] && cmd+=(--project "$project")
  [[ -n "$conversation" ]] && cmd+=(--conversation "$conversation")
  [[ "${AGY_CONTINUE:-false}" == "true" ]] && cmd+=(--continue)
  if [[ "${AGY_NEW_PROJECT:-auto}" == "true" || ( "${AGY_NEW_PROJECT:-auto}" == "auto" && -z "$project" && -z "$conversation" && "${AGY_CONTINUE:-false}" != "true" ) ]]; then
    cmd+=(--new-project)
  fi
  if [[ "${AGY_LOG_FILE:-auto}" != "off" ]]; then
    if [[ -n "${AGY_LOG_FILE:-}" && "${AGY_LOG_FILE:-}" != "auto" ]]; then
      agy_cli_log="$AGY_LOG_FILE"
    else
      agy_cli_log="${log_file%.log}.agy-cli.log"
    fi
    mkdir -p "$(dirname "$agy_cli_log")"
    cmd+=(--log-file "$agy_cli_log")
  fi
  agy_add_dirs_to_cmd cmd

  if [[ "$prompt_bytes" =~ ^[0-9]+$ && "$prompt_bytes" -gt "$max_inline" ]]; then
    prompt_dir="$(cd "$(dirname "$prompt_file")" && pwd)"
    cmd+=(--add-dir "$prompt_dir")
    prompt="The full prompt for role '$role' is too large to pass safely as one CLI argument. Read and follow this prompt file exactly: $prompt_file"
    printf 'AGY prompt indirection: %s bytes exceeds AGY_INLINE_PROMPT_MAX_BYTES=%s; using prompt file path %s\n' "$prompt_bytes" "$max_inline" "$prompt_file" | tee -a "$log_file"
  else
    prompt="$(cat "$prompt_file")"
  fi
  cmd+=(--print "$prompt")

  printf 'AGY effective model: %s\n' "$agy_model" | tee -a "$log_file"

  set +e
  timeout "$timeout_value" "${cmd[@]}" 2>&1 | tee -a "$log_file"
  code=${PIPESTATUS[0]}
  set -e
  return "$code"
}

run_agent_role() {
  local role="$1" prompt_file="$2" model="$3" cli="${4:-}" timeout_value="$5" log_file="$6" effort="${7:-}" sandbox="${8:-${SANDBOX:-workspace-write}}"
  if [[ -z "$cli" ]]; then
    cli="$(agent_cli_for_role "$role")" || return $?
  else
    agent_validate_cli_name "$cli" || return $?
  fi
  case "$cli" in
    codex) run_codex_role "$role" "$prompt_file" "$model" "$timeout_value" "$log_file" "$effort" "$sandbox" ;;
    agy) run_agy_role "$role" "$prompt_file" "$model" "$timeout_value" "$log_file" ;;
    *) echo "Unsupported AI CLI: $cli" >&2; return 2 ;;
  esac
}

run_codex_exec_with_capacity_retry() {
  local model="" effort="" sandbox="" timeout_value="" prompt_file="" log_file="" role="codex"
  while [[ "$#" -gt 0 ]]; do
    case "$1" in
      --model) model="${2:-}"; shift 2 ;;
      --effort) effort="${2:-}"; shift 2 ;;
      --sandbox) sandbox="${2:-}"; shift 2 ;;
      --timeout) timeout_value="${2:-}"; shift 2 ;;
      --prompt-file) prompt_file="${2:-}"; shift 2 ;;
      --log-file) log_file="${2:-}"; shift 2 ;;
      --role) role="${2:-codex}"; shift 2 ;;
      *) echo "run_codex_exec_with_capacity_retry: unknown option $1" >&2; return 2 ;;
    esac
  done

  if [[ -z "$model" || -z "$sandbox" || -z "$timeout_value" || -z "$prompt_file" || -z "$log_file" ]]; then
    echo "run_codex_exec_with_capacity_retry: model, sandbox, timeout, prompt-file, and log-file are required" >&2
    return 2
  fi

  local retry_enabled="${CODEX_CAPACITY_RETRY:-true}"
  local initial_delay max_delay total_timeout
  initial_delay="$(codex_retry_parse_duration "${CODEX_CAPACITY_RETRY_INITIAL_DELAY:-30s}")" || return $?
  max_delay="$(codex_retry_parse_duration "${CODEX_CAPACITY_RETRY_MAX_DELAY:-2h}")" || return $?
  total_timeout="$(codex_retry_parse_duration "${CODEX_CAPACITY_RETRY_TIMEOUT:-0}")" || return $?

  if (( initial_delay < 1 )); then initial_delay=30; fi
  if (( max_delay < initial_delay )); then max_delay="$initial_delay"; fi

  local attempt=1 delay="$initial_delay" started_at now elapsed remaining sleep_for code
  started_at="$(date +%s)"

  while true; do
    if (( attempt > 1 )); then
      printf '\n== Codex capacity retry attempt %d: model=%s ==\n' "$attempt" "$model" | tee -a "$log_file"
    fi

    set +e
    codex_run_once "$model" "$effort" "$sandbox" "$timeout_value" "$prompt_file" "$log_file" "$role"
    code=$?
    set -e

    if [[ "$code" -eq 0 ]]; then
      return 0
    fi
    if [[ "$retry_enabled" != "true" ]] || ! codex_retry_last_error_is_capacity "$log_file"; then
      return "$code"
    fi

    now="$(date +%s)"
    elapsed=$((now - started_at))
    if (( total_timeout > 0 && elapsed >= total_timeout )); then
      printf '\nERROR: Codex capacity retry timeout reached after %s.\n' "$(codex_retry_format_duration "$elapsed")" | tee -a "$log_file"
      codex_retry_log_event "$role" "capacity retry timeout after $(codex_retry_format_duration "$elapsed")"
      return "$code"
    fi

    sleep_for="$delay"
    if (( total_timeout > 0 )); then
      remaining=$((total_timeout - elapsed))
      if (( remaining <= 0 )); then
        printf '\nERROR: Codex capacity retry timeout reached.\n' | tee -a "$log_file"
        codex_retry_log_event "$role" "capacity retry timeout"
        return "$code"
      fi
      if (( sleep_for > remaining )); then sleep_for="$remaining"; fi
    fi

    printf '\nCodex model at capacity. Waiting %s before retrying model %s.\n' "$(codex_retry_format_duration "$sleep_for")" "$model" | tee -a "$log_file"
    codex_retry_log_event "$role" "capacity wait=$(codex_retry_format_duration "$sleep_for") model=$model attempt=$attempt"
    sleep "$sleep_for"

    if (( delay < max_delay )); then
      delay=$((delay * 2))
      if (( delay > max_delay )); then delay="$max_delay"; fi
    fi
    attempt=$((attempt + 1))
  done
}
