#!/usr/bin/env bash
set -euo pipefail

AI_DIR="${AI_DIR:-$(cd "$(dirname "$0")/.." && pwd)}"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

resolve_preset() {
  local preset="$1" case_dir
  case_dir="$tmp/$(basename "$preset" .env)"
  mkdir -p "$case_dir/config" "$case_dir/scripts"
  cp "$AI_DIR/config/default.env" "$case_dir/config/default.env"
  cp "$preset" "$case_dir/config/user.env"
  cp "$AI_DIR/scripts/agent-load-env.sh" "$case_dir/scripts/agent-load-env.sh"
  cp "$AI_DIR/scripts/agent-codex-retry.sh" "$case_dir/scripts/agent-codex-retry.sh"
  env -i PATH="$PATH" HOME="${HOME:-}" bash -c '
    set -euo pipefail
    source "$1/scripts/agent-load-env.sh"
    load_agent_env "$1"
    source "$1/scripts/agent-codex-retry.sh"
    planner_cli="$(agent_cli_for_role planner)"
    coder_cli="$(agent_cli_for_role coder)"
    reviewer_cli="$(agent_cli_for_role reviewer)"
    final_cli="$(agent_cli_for_role reviewer-final)"
    printf "%s|%s|%s|%s|%s|%s|%s|%s|%s|%s\n" \
      "$AIA_PROFILE" "$TASK_SIZE_OVERRIDE" \
      "$planner_cli" "$(agent_effective_model_for_role planner "$PLANNER_MODEL" "$planner_cli")" \
      "$coder_cli" "$(agent_effective_model_for_role coder "$CODER_MODEL" "$coder_cli")" \
      "$reviewer_cli" "$(agent_effective_model_for_role reviewer "$REVIEWER_MODEL" "$reviewer_cli")" \
      "$final_cli" "$(agent_effective_model_for_role reviewer-final "$FINAL_REVIEWER_MODEL" "$final_cli")"
  ' _ "$case_dir"
}

test "$(resolve_preset "$AI_DIR/config/examples/small-codex.env")" = \
  'balanced|SMALL|codex|gpt-5.6-sol|codex|gpt-5.6-sol|codex|gpt-5.6-sol|codex|gpt-5.6-sol'
test "$(resolve_preset "$AI_DIR/config/examples/small-agy.env")" = \
  'agy-efficient|SMALL|agy|Gemini 3.1 Pro (Low)|agy|Gemini 3.5 Flash (Low)|agy|Gemini 3.5 Flash (Medium)|agy|Gemini 3.1 Pro (Low)'
test "$(resolve_preset "$AI_DIR/config/examples/medium-hybrid.env")" = \
  'hybrid-efficient|MEDIUM|codex|gpt-5.6-sol|agy|Gemini 3.5 Flash (Low)|agy|Gemini 3.5 Flash (Medium)|codex|gpt-5.6-sol'
test "$(resolve_preset "$AI_DIR/config/examples/medium-codex-split.env")" = \
  'codex-split|MEDIUM|codex|gpt-5.6-sol|codex|gpt-5.6-luna|codex|gpt-5.6-luna|codex|gpt-5.6-sol'
test "$(resolve_preset "$AI_DIR/config/examples/large-codex.env")" = \
  'balanced|LARGE|codex|gpt-5.6-sol|codex|gpt-5.6-sol|codex|gpt-5.6-sol|codex|gpt-5.6-sol'
test "$(resolve_preset "$AI_DIR/config/examples/large-hybrid.env")" = \
  'hybrid-efficient|LARGE|codex|gpt-5.6-sol|agy|Gemini 3.5 Flash (High)|agy|Gemini 3.1 Pro (Low)|codex|gpt-5.6-sol'
test "$(resolve_preset "$AI_DIR/config/examples/large-agy.env")" = \
  'agy-efficient|LARGE|agy|Gemini 3.1 Pro (High)|agy|Gemini 3.5 Flash (High)|agy|Gemini 3.1 Pro (Low)|agy|Gemini 3.1 Pro (High)'

status_root="$tmp/status-project"
mkdir -p "$status_root/.ai-agent/config" "$status_root/.ai-agent/scripts" "$status_root/.ai-agent/ai-plan/tasks" "$status_root/.ai-agent/generated/runtime"
cp "$AI_DIR/config/default.env" "$status_root/.ai-agent/config/default.env"
cp "$AI_DIR/config/examples/medium-hybrid.env" "$status_root/.ai-agent/config/user.env"
cp "$AI_DIR/scripts/agent-load-env.sh" "$status_root/.ai-agent/scripts/agent-load-env.sh"
cp "$AI_DIR/scripts/agent-status.sh" "$status_root/.ai-agent/scripts/agent-status.sh"
cp "$AI_DIR/VERSION" "$status_root/.ai-agent/VERSION"
(cd "$status_root" && bash .ai-agent/scripts/agent-status.sh) > "$tmp/status.out"
grep -q '^Config$' "$tmp/status.out"
grep -q 'USE_PERSISTENT_SESSIONS.*: true' "$tmp/status.out"

echo "Config example tests passed."
