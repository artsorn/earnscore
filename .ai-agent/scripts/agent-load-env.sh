#!/usr/bin/env bash

agent_profile_set() {
  local name="$1" value="$2"
  case " ${AGENT_PRESET_ENV_KEYS:-} ${AGENT_USER_ENV_KEYS:-} " in
    *" $name "*) return 0 ;;
  esac
  printf -v "$name" '%s' "$value"
  export "$name"
}

agent_apply_profile() {
  case "${AIA_PROFILE:-balanced}" in
    balanced)
      # Provider-safe default: one available Codex model, adaptive effort per role.
      ;;
    codex-split)
      agent_profile_set PLANNER_CLI codex
      agent_profile_set CODER_CLI codex
      agent_profile_set REVIEWER_CLI codex
      agent_profile_set FINAL_REVIEWER_CLI codex
      agent_profile_set PLANNER_MODEL gpt-5.6-sol
      agent_profile_set CODER_MODEL gpt-5.6-luna
      agent_profile_set REVIEWER_MODEL gpt-5.6-luna
      agent_profile_set FINAL_REVIEWER_MODEL gpt-5.6-sol
      ;;
    hybrid-efficient)
      agent_profile_set PLANNER_CLI codex
      agent_profile_set CODER_CLI agy
      agent_profile_set REVIEWER_CLI agy
      agent_profile_set FINAL_REVIEWER_CLI codex
      agent_profile_set PLANNER_MODEL gpt-5.6-sol
      agent_profile_set CODER_AGY_MODEL 'Gemini 3.5 Flash (Low)'
      agent_profile_set REVIEWER_AGY_MODEL 'Gemini 3.5 Flash (Medium)'
      agent_profile_set FINAL_REVIEWER_MODEL gpt-5.6-sol
      ;;
    agy-efficient)
      agent_profile_set PLANNER_CLI agy
      agent_profile_set CODER_CLI agy
      agent_profile_set REVIEWER_CLI agy
      agent_profile_set FINAL_REVIEWER_CLI agy
      agent_profile_set PLANNER_AGY_MODEL 'Gemini 3.1 Pro (High)'
      agent_profile_set CODER_AGY_MODEL 'Gemini 3.5 Flash (Low)'
      agent_profile_set REVIEWER_AGY_MODEL 'Gemini 3.5 Flash (Medium)'
      agent_profile_set FINAL_REVIEWER_AGY_MODEL 'Gemini 3.1 Pro (High)'
      ;;
    custom)
      ;;
    *)
      echo "Invalid AIA_PROFILE: ${AIA_PROFILE:-}. Expected balanced, codex-split, hybrid-efficient, agy-efficient, or custom." >&2
      return 2
      ;;
  esac
}

load_agent_env() {
  local ai_dir="${1:-.ai-agent}"
  local default_env="$ai_dir/config/default.env"
  local user_env="$ai_dir/config/user.env"
  local tracked_var preset_keys="" user_keys=""

  for tracked_var in \
    AIA_PROFILE PLANNER_CLI CODER_CLI REVIEWER_CLI FINAL_REVIEWER_CLI \
    PLANNER_MODEL CODER_MODEL REVIEWER_MODEL FINAL_REVIEWER_MODEL \
    CODER_SESSION_MODEL REVIEWER_SESSION_MODEL \
    AGY_TIMEOUT AGY_SANDBOX AGY_SKIP_PERMISSIONS AGY_NEW_PROJECT AGY_MODE AGY_LOG_FILE AGY_PROJECT AGY_CONVERSATION AGY_CONTINUE AGY_ADD_DIRS \
    PLANNER_AGY_MODEL CODER_AGY_MODEL REVIEWER_AGY_MODEL FINAL_REVIEWER_AGY_MODEL \
    PLANNER_AGY_PROJECT CODER_AGY_PROJECT REVIEWER_AGY_PROJECT FINAL_REVIEWER_AGY_PROJECT \
    PLANNER_AGY_CONVERSATION CODER_AGY_CONVERSATION REVIEWER_AGY_CONVERSATION FINAL_REVIEWER_AGY_CONVERSATION \
    PLANNER_LEVEL CODER_LEVEL REVIEWER_LEVEL FINAL_REVIEWER_LEVEL \
    PLANNER_CONTEXT_TOKENS CODER_CONTEXT_TOKENS MAX_CONTEXT_TOKENS \
    SEARCH_BUDGET CODEGRAPH_DEPTH MAX_TASKS_FROM_PLANNER \
    PLANNER_TASK_DETAIL_LEVEL TASK_SIZE_OVERRIDE INLINE_CONTEXT_PACKAGE INLINE_REVIEWER_DIFF \
    CODEX_TOOL_OUTPUT_TOKEN_LIMIT CODEX_AUTO_COMPACT_TOKEN_LIMIT CODEX_ROLLOUT_BUDGET_TOKENS \
    VISIBLE_LOG_MAX_LINES VISIBLE_LOG_MAX_BYTES VISIBLE_LOG_DIFF_MAX_LINES RAW_LOG_COMPRESSION; do
    if [[ -n "${!tracked_var+x}" ]]; then
      preset_keys="${preset_keys} ${tracked_var} "
    fi
  done
  export AGENT_PRESET_ENV_KEYS="$preset_keys"

  if [[ -f "$default_env" ]]; then
    # shellcheck disable=SC1090
    source "$default_env"
  fi

  if [[ ! -f "$user_env" ]]; then
    if [[ "$preset_keys " != *" CODER_MODEL "* && "$preset_keys " == *" CODER_SESSION_MODEL "* ]]; then
      CODER_MODEL="$CODER_SESSION_MODEL"
      export CODER_MODEL
    fi
    if [[ "$preset_keys " != *" REVIEWER_MODEL "* && "$preset_keys " == *" REVIEWER_SESSION_MODEL "* ]]; then
      REVIEWER_MODEL="$REVIEWER_SESSION_MODEL"
      export REVIEWER_MODEL
    fi
    export AGENT_USER_ENV_KEYS=""
    agent_apply_profile
    return $?
  fi

  while IFS= read -r line || [[ -n "$line" ]]; do
    [[ "$line" =~ ^[[:space:]]*$ ]] && continue
    [[ "$line" =~ ^[[:space:]]*# ]] && continue

    if [[ "$line" =~ ^[[:space:]]*:[[:space:]]*\$\{([A-Za-z_][A-Za-z0-9_]*)\:=([^}]*)\}[[:space:]]*$ ]]; then
      local name="${BASH_REMATCH[1]}" value="${BASH_REMATCH[2]}"
      if [[ -z "${!name+x}" ]]; then
        printf -v "$name" '%s' "$value"
        export "$name"
        user_keys="${user_keys} ${name} "
      fi
      continue
    fi

    if [[ "$line" =~ ^[[:space:]]*([A-Za-z_][A-Za-z0-9_]*)=(.*)$ ]]; then
      local name="${BASH_REMATCH[1]}" value="${BASH_REMATCH[2]}"
      printf -v "$name" '%s' "$value"
      export "$name"
      user_keys="${user_keys} ${name} "
      continue
    fi

    eval "$line"
  done < "$user_env"

  if [[ "$preset_keys $user_keys " != *" CODER_MODEL "* && "$preset_keys $user_keys " == *" CODER_SESSION_MODEL "* ]]; then
    CODER_MODEL="$CODER_SESSION_MODEL"
    export CODER_MODEL
  fi
  if [[ "$preset_keys $user_keys " != *" REVIEWER_MODEL "* && "$preset_keys $user_keys " == *" REVIEWER_SESSION_MODEL "* ]]; then
    REVIEWER_MODEL="$REVIEWER_SESSION_MODEL"
    export REVIEWER_MODEL
  fi

  export AGENT_USER_ENV_KEYS="$user_keys"
  agent_apply_profile
}
