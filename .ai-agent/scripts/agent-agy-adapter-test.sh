#!/usr/bin/env bash
set -euo pipefail

AI_DIR="${AI_DIR:-$(cd "$(dirname "$0")/.." && pwd)}"
source "$AI_DIR/scripts/agent-codex-retry.sh"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp/bin" "$tmp/runtime"

cat > "$tmp/bin/agy" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' "$@" > "$AGY_TEST_ARGS"
echo OK
EOF
chmod +x "$tmp/bin/agy"
printf 'Do the task.\n' > "$tmp/prompt.md"

export PATH="$tmp/bin:$PATH"
export AGY_TEST_ARGS="$tmp/args.txt"
export AGY_NEW_PROJECT=auto
export AGY_MODE=accept-edits
export AGY_LOG_FILE=auto
export AGY_SKIP_PERMISSIONS=true
export AGY_SANDBOX=false
unset AGY_PROJECT AGY_CONVERSATION AGY_CONTINUE CODER_AGY_MODEL

test "$(agy_role_model coder gpt-5.6-sol)" = "Gemini 3.5 Flash (Low)"
test "$(agy_role_model reviewer gemini-3.5-flash)" = "Gemini 3.5 Flash (Medium)"
CODER_AGY_MODEL=gemini-3.5-flash-high
test "$(agy_role_model coder ignored)" = "Gemini 3.5 Flash (High)"
unset CODER_AGY_MODEL

run_agy_role coder "$tmp/prompt.md" gpt-5.6-sol 1m "$tmp/runtime/coder.log" >/dev/null
grep -qx -- '--model' "$tmp/args.txt"
grep -qx -- 'Gemini 3.5 Flash (Low)' "$tmp/args.txt"
grep -qx -- '--new-project' "$tmp/args.txt"
grep -qx -- '--mode' "$tmp/args.txt"
grep -qx -- 'accept-edits' "$tmp/args.txt"
grep -qx -- '--log-file' "$tmp/args.txt"
grep -qx -- "$tmp/runtime/coder.agy-cli.log" "$tmp/args.txt"

AGY_PROJECT=project-123
run_agy_role coder "$tmp/prompt.md" gemini-3.5-flash-low 1m "$tmp/runtime/coder-project.log" >/dev/null
grep -qx -- '--project' "$tmp/args.txt"
grep -qx -- 'project-123' "$tmp/args.txt"
! grep -qx -- '--new-project' "$tmp/args.txt"

echo "AGY adapter tests passed."
