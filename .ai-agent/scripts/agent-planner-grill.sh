#!/usr/bin/env bash
set -euo pipefail

AI_DIR="${AI_DIR:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
PROJECT_ROOT="${PROJECT_ROOT:-$(cd "$AI_DIR/.." && pwd)}"
if [[ -f "$AI_DIR/scripts/agent-load-env.sh" ]]; then
  # shellcheck disable=SC1090
  source "$AI_DIR/scripts/agent-load-env.sh"
  load_agent_env "$AI_DIR"
fi
if [[ -f "$AI_DIR/scripts/agent-codex-retry.sh" ]]; then source "$AI_DIR/scripts/agent-codex-retry.sh"; fi
if [[ -f "$AI_DIR/scripts/agent-task-size.sh" ]]; then source "$AI_DIR/scripts/agent-task-size.sh"; fi

RUNTIME="${RUNTIME_DIR:-$AI_DIR/generated/runtime}"
GRILL_DIR="$AI_DIR/ai-plan/grill"
mkdir -p "$RUNTIME" "$GRILL_DIR" "$AI_DIR/ai-plan/tasks"

die() { echo "ERROR: $*" >&2; exit 1; }

usage() {
  cat <<'USAGE'
Usage:
  .ai-agent/scripts/agent-planner-grill.sh start
  .ai-agent/scripts/agent-planner-grill.sh next
  .ai-agent/scripts/agent-planner-grill.sh approve
  .ai-agent/scripts/agent-planner-grill.sh freeze
  .ai-agent/scripts/agent-planner-grill.sh validate-questions <questions.md>

Files:
  .agent/requirement.md
  .ai-agent/ai-plan/draft-plan.md
  .ai-agent/ai-plan/revised-plan.md
  .ai-agent/ai-plan/grill/questions.md
  .ai-agent/ai-plan/grill/answers.md
  .ai-agent/ai-plan/grill/round-001-questions.md
  .ai-agent/ai-plan/grill/round-001-answers.md
  .ai-agent/ai-plan/grill/round-001-revised-plan.md
USAGE
}

round_file() {
  local round="$1" suffix="$2"
  printf '%s/round-%03d-%s.md' "$GRILL_DIR" "$round" "$suffix"
}

latest_round() {
  local latest
  latest="$(find "$GRILL_DIR" -maxdepth 1 -type f -name 'round-*-questions.md' -printf '%f\n' 2>/dev/null | sed -E 's/^round-([0-9]+)-questions\.md$/\1/' | sort -n | tail -n1 || true)"
  if [[ -n "$latest" ]]; then
    printf '%d' "$((10#$latest))"
  else
    printf '0'
  fi
}

latest_plan_file() {
  local round="$1" revised
  while (( round >= 1 )); do
    revised="$(round_file "$round" "revised-plan")"
    if [[ -s "$revised" ]]; then
      printf '%s' "$revised"
      return 0
    fi
    round=$((round - 1))
  done
  if [[ -s "$AI_DIR/ai-plan/revised-plan.md" ]]; then
    printf '%s' "$AI_DIR/ai-plan/revised-plan.md"
  elif [[ -s "$AI_DIR/ai-plan/draft-plan.md" ]]; then
    printf '%s' "$AI_DIR/ai-plan/draft-plan.md"
  else
    return 1
  fi
}

answers_approved() {
  local answers="$1"
  [[ -f "$answers" ]] || return 1
  grep -Eiq '^[[:space:]]*(PLAN[[:space:]]+)?APPROVED[[:space:]]*$' "$answers"
}

assert_no_final_plan_modified() {
  local marker="$1" changed=""
  if [[ -f "$AI_DIR/ai-plan/overview.md" && "$AI_DIR/ai-plan/overview.md" -nt "$marker" ]]; then
    changed="$changed"$'\n'"$AI_DIR/ai-plan/overview.md"
  fi
  if [[ -f "$AI_DIR/ai-plan/context.md" && "$AI_DIR/ai-plan/context.md" -nt "$marker" ]]; then
    changed="$changed"$'\n'"$AI_DIR/ai-plan/context.md"
  fi
  changed="$changed"$'\n'"$(find "$AI_DIR/ai-plan/tasks" -maxdepth 1 -type f -name 'task-*.md' -newer "$marker" 2>/dev/null || true)"
  changed="$(printf '%s\n' "$changed" | sed '/^[[:space:]]*$/d' || true)"
  if [[ -n "$changed" ]]; then
    echo "Planner Grill rule violation: final plan/task files were modified before approval:" >&2
    printf '%s\n' "$changed" >&2
    echo "Only draft-plan.md, revised-plan.md, and ai-plan/grill/* may be changed before freeze." >&2
    return 1
  fi
}

sync_answers_for_round() {
  local round="$1" round_answers alias_answers
  round_answers="$(round_file "$round" "answers")"
  alias_answers="$GRILL_DIR/answers.md"
  if [[ -s "$alias_answers" ]]; then
    cp "$alias_answers" "$round_answers"
  elif [[ ! -f "$round_answers" ]]; then
    write_answer_placeholder "$round_answers" "$round"
  fi
}

write_answer_placeholder() {
  local path="$1" round="$2"
  cat > "$path" <<EOF
# Planner Grill Answers - Round $(printf '%03d' "$round")

ตอบสั้น ๆ ด้วยหมายเลขตัวเลือก หรือกำหนดเอง เช่น:

Q1: 1
Q2: 3
Q3: CUSTOM - ใช้แนวทาง ... เพราะ ...

ถ้าต้องการเลือกข้อที่ AI แนะนำทุกคำถาม ให้ตอบ:

USE ALL AI RECOMMENDATIONS

ถ้าแผนพร้อม freeze แล้ว ให้ใส่บรรทัดใดบรรทัดหนึ่งแบบเดี่ยว ๆ:

APPROVED

หรือ

PLAN APPROVED
EOF
}

print_grill_question_contract() {
  cat <<'EOF'

# Required Choice Format for Every Grill Question

Ask no more than 5 blocker questions per round. Every question must use this exact Markdown structure and literal field labels:

```md
## Q1: <specific decision in one short sentence>
**Why this matters:** <one short sentence explaining impact>

1. **<recommended choice>** [AI RECOMMENDED]
   - Explanation: <one or two short sentences>
   - Example: <one short concrete result or scenario>
2. **<alternative choice>**
   - Explanation: <one or two short sentences>
   - Example: <one short concrete result or scenario>
3. **<alternative choice>**
   - Explanation: <one or two short sentences>
   - Example: <one short concrete result or scenario>
4. **<alternative choice>**
   - Explanation: <one or two short sentences>
   - Example: <one short concrete result or scenario>

**Custom:** Describe a different choice and the result you expect.
```

Choice rules:
- Choice 1 must always be the AI recommendation and must include `[AI RECOMMENDED]` exactly.
- Choices 2-4 must be realistic, distinct alternatives with meaningful tradeoffs, not filler.
- Each Explanation must be concise and make the consequence understandable to a non-expert.
- Each Example must be one short, concrete output, UI behavior, data shape, command result, or user flow.
- Preserve the literal labels `Explanation:`, `Example:`, and `**Custom:**`; question content may be in the user's language.
- Do not put `[AI RECOMMENDED]` on choices 2-4.
- Do not ask open-ended questions outside this structure.
- If fewer than four safe alternatives exist, use conservative variations in scope, behavior, or implementation strategy; do not omit an option.
- If no blocker remains, write no question blocks and state `No blocker questions were written by Planner.`
EOF
}

validate_grill_questions() {
  local path="$1"
  [[ -s "$path" ]] || { echo "Grill questions file is missing or empty: $path" >&2; return 1; }
  python3 - "$path" <<'PY'
import re
import sys
from pathlib import Path

path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8", errors="replace")
headers = list(re.finditer(r"(?m)^## Q([1-9][0-9]*):\s+\S.*$", text))

if not headers:
    if "No blocker questions were written by Planner." in text:
        raise SystemExit(0)
    print(f"Invalid grill questions: {path}", file=sys.stderr)
    print("Expected at least one '## Q1: ...' block or the no-blocker sentence.", file=sys.stderr)
    raise SystemExit(1)

errors = []
if len(headers) > 5:
    errors.append(f"found {len(headers)} questions; maximum is 5")

numbers = [int(match.group(1)) for match in headers]
if numbers != list(range(1, len(headers) + 1)):
    errors.append(f"question numbers must be sequential from Q1; found {numbers}")

for index, header in enumerate(headers):
    qnum = int(header.group(1))
    end = headers[index + 1].start() if index + 1 < len(headers) else len(text)
    block = text[header.start():end]
    if not re.search(r"(?m)^\*\*Why this matters:\*\*\s+\S", block):
        errors.append(f"Q{qnum}: missing '**Why this matters:**'")

    option_matches = list(re.finditer(r"(?m)^([1-4])\. \*\*(\S.*?)\*\*(?: \[AI RECOMMENDED\])?\s*$", block))
    option_numbers = [int(match.group(1)) for match in option_matches]
    if option_numbers != [1, 2, 3, 4]:
        errors.append(f"Q{qnum}: choices must be exactly 1, 2, 3, 4; found {option_numbers}")
        continue

    recommended_lines = re.findall(r"(?m)^[1-4]\. .*\[AI RECOMMENDED\]\s*$", block)
    if len(recommended_lines) != 1 or not recommended_lines[0].startswith("1. "):
        errors.append(f"Q{qnum}: choice 1 must be the only [AI RECOMMENDED] choice")

    for opt_index, match in enumerate(option_matches):
        opt_num = int(match.group(1))
        opt_end = option_matches[opt_index + 1].start() if opt_index + 1 < len(option_matches) else len(block)
        option_block = block[match.end():opt_end]
        if not re.search(r"(?m)^\s+- Explanation:\s+\S", option_block):
            errors.append(f"Q{qnum} choice {opt_num}: missing Explanation")
        if not re.search(r"(?m)^\s+- Example:\s+\S", option_block):
            errors.append(f"Q{qnum} choice {opt_num}: missing Example")

    if not re.search(r"(?m)^\*\*Custom:\*\*\s+\S", block):
        errors.append(f"Q{qnum}: missing '**Custom:**'")

    if re.search(r"(?m)^[5-9]\. \*\*", block):
        errors.append(f"Q{qnum}: numeric choices beyond 4 are not allowed")

if errors:
    print(f"Invalid grill questions: {path}", file=sys.stderr)
    for error in errors:
        print(f"- {error}", file=sys.stderr)
    raise SystemExit(1)

print(f"Valid grill choice format: {path}")
PY
}

prepare_planner_context() {
  bash "$AI_DIR/scripts/agent-codegraph.sh" ensure >/dev/null || true
  bash "$AI_DIR/scripts/agent-knowledge-scan.sh" ensure >/dev/null || true
  if declare -F agent_task_adaptive_apply >/dev/null 2>&1; then
    agent_task_adaptive_apply planner "$AI_DIR" "$PROJECT_ROOT" >/dev/null || true
  fi
  CONTEXT_SKIP_ENSURE=true CONTEXT_ROLE=planner CONTEXT_MODE=balanced bash "$AI_DIR/scripts/agent-context-build.sh" >/dev/null || true
}

planner_prompt_prelude() {
  local mode="$1"
  echo "# Invocation Context"
  echo
  echo "- Role: planner"
  echo "- Planner grill mode: $mode"
  echo "- Requirement: .agent/requirement.md"
  echo "- Draft plan: .ai-agent/ai-plan/draft-plan.md"
  echo "- Revised plan: .ai-agent/ai-plan/revised-plan.md"
  echo "- Grill directory: .ai-agent/ai-plan/grill/"
  echo "- Compact context package: .ai-agent/generated/runtime/context-package.md"
  echo "- Search allowlist: .ai-agent/generated/runtime/search-allowlist.txt"
  echo "- Runtime context: .ai-agent/generated/runtime/runtime-context.md"
  echo "- Important: do not implement code."
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

run_planner_grill_prompt() {
  local mode="$1" prompt_file="$2" log_file="$3"
  local model="${PLANNER_MODEL:-gpt-5.6-sol}" cli="${PLANNER_CLI:-codex}" effort="${PLANNER_LEVEL:-xhigh}" timeout_value="${PLANNER_TIMEOUT:-180m}" sandbox="${SANDBOX:-workspace-write}" code
  : > "$log_file"
  set +e
  run_agent_role "planner" "$prompt_file" "$model" "$cli" "$timeout_value" "$log_file" "$effort" "$sandbox"
  code=$?
  set -e
  if declare -F record_token_usage >/dev/null 2>&1; then
    record_token_usage "planner-grill-$mode" "$model" "$log_file" "$mode" || true
  fi
  return "$code"
}

write_start_prompt() {
  local round="$1" prompt_file="$2" questions_file
  questions_file="$(round_file "$round" "questions")"
  {
    planner_prompt_prelude "start"
    cat <<EOF
# Planner Grill Start

Create a draft implementation plan and challenge it before any implementation task files exist.

Inputs to read:
- .agent/requirement.md
- .ai-agent/generated/runtime/context-package.md if present
- .ai-agent/generated/runtime/runtime-context.md if needed
- Existing .ai-agent/ai-plan/overview.md, context.md, and tasks only as historical context. Do not treat them as approved for this new grill loop unless the requirement explicitly says to continue them.

Required outputs:
- Write the draft plan to .ai-agent/ai-plan/draft-plan.md
- Write grill questions to $questions_file
- Also copy the same questions to .ai-agent/ai-plan/grill/questions.md

Hard rules:
- Do not create, edit, or delete .ai-agent/ai-plan/tasks/task-*.md
- Do not finalize .ai-agent/ai-plan/overview.md or .ai-agent/ai-plan/context.md
- Treat draft-plan.md as not approved and not executable by Coder.
- Ask challenge questions that would reduce scope leaks, wrong assumptions, risky migrations, API/schema/UI regressions, and vague task boundaries.
- If a requirement is ambiguous, ask instead of guessing.
- If a point is not a blocker, write a clear assumption in draft-plan.md and ask whether the user accepts it.

Draft plan must include:
- Goal
- Known constraints
- Risks
- Scope boundaries
- Affected files/modules
- Forbidden files/modules
- Open questions
- Proposed task breakdown, but explicitly label it DRAFT ONLY.

Questions must cover:
- unclear requirement boundaries
- undecided edge cases
- large or risky scope
- files/modules that should be forbidden
- existing behavior that must not break
- migration/schema/API/UI/test risks
- whether tasks should be split smaller
- how the plan might affect existing flows

Do not implement code.
EOF
    print_grill_question_contract
  } > "$prompt_file"
}

write_next_prompt() {
  local round="$1" next_round="$2" prompt_file="$3" plan_file="$4" answers_file="$5" revised_file questions_file
  revised_file="$(round_file "$round" "revised-plan")"
  questions_file="$(round_file "$next_round" "questions")"
  {
    planner_prompt_prelude "next"
    cat <<EOF
# Planner Grill Next

Read the current plan and the user's grill answers, then revise the plan and ask only remaining blocker questions.

Inputs to read:
- $plan_file
- $(round_file "$round" "questions")
- $answers_file
- .agent/requirement.md
- .ai-agent/generated/runtime/context-package.md if present

Answer syntax to interpret:
- `Q1: 1` means select numbered choice 1 for Q1.
- `Q1: CUSTOM - ...` means use the user's custom decision for Q1.
- `USE ALL AI RECOMMENDATIONS` means select choice 1 for every unanswered question in that round.
- Free-form notes after a choice refine that selected choice and must not be ignored.

Required outputs:
- Write the revised plan to $revised_file
- Also copy the revised plan to .ai-agent/ai-plan/revised-plan.md
- If blocker questions remain, write only those questions to $questions_file
- Also copy those questions to .ai-agent/ai-plan/grill/questions.md

Hard rules:
- Do not create, edit, or delete .ai-agent/ai-plan/tasks/task-*.md
- Do not finalize .ai-agent/ai-plan/overview.md or .ai-agent/ai-plan/context.md
- Do not ask questions that are not blockers; record non-blocker assumptions clearly in the revised plan.
- If the user's answer is incomplete on a blocker, ask again with a narrower question.
- If the work is too large, force smaller task boundaries in the revised plan.

Revised plan must include:
- Goal
- Decisions from user answers
- Explicit assumptions
- Risks
- Scope
- Affected files/modules
- Forbidden files/modules
- Acceptance criteria
- Proposed task breakdown with tasks small enough for Coder to finish one task in one round

Do not implement code.
EOF
    print_grill_question_contract
  } > "$prompt_file"
}

write_freeze_prompt() {
  local prompt_file="$1" plan_file="$2" answers_file="$3"
  {
    planner_prompt_prelude "freeze"
    cat <<EOF
# Planner Grill Freeze

The user approved the grill plan. Convert the approved plan into the normal implementation plan that Coder/Reviewer can execute.

Inputs to read:
- $plan_file
- $answers_file
- .agent/requirement.md
- .ai-agent/generated/runtime/context-package.md if present
- Existing .ai-agent/ai-plan/overview.md, context.md, and tasks only to preserve useful continuity.

Required outputs:
- Update or create .ai-agent/ai-plan/overview.md
- Update or create .ai-agent/ai-plan/context.md
- Update or create .ai-agent/ai-plan/tasks/task-*.md

Hard rules:
- Only freeze because the user answer contains APPROVED or PLAN APPROVED.
- Create implementation task files only now.
- Each task must have narrow scope, explicit affected files, forbidden files, acceptance criteria, validation commands, and a Reference Map.
- Each task must be small enough for Coder to complete in one round.
- If a task would be too broad, split it.
- Do not implement code.
- Do not include agent framework/runtime/task-state paths such as .ai-agent/**, root AGENTS.md, .gitignore, or .agent/loop-verdict.txt as implementation scope unless the approved plan is explicitly about this agent framework.
EOF
  } > "$prompt_file"
}

cmd_start() {
  [[ -s "$PROJECT_ROOT/.agent/requirement.md" ]] || die ".agent/requirement.md not found or empty"
  prepare_planner_context
  local round=1 prompt_file="$RUNTIME/planner-grill-start.prompt.md" log_file="$RUNTIME/planner-grill-start.log" answers_file questions_file marker
  marker="$RUNTIME/planner-grill-start.marker"
  : > "$marker"
  questions_file="$(round_file "$round" "questions")"
  answers_file="$(round_file "$round" "answers")"
  write_start_prompt "$round" "$prompt_file"
  run_planner_grill_prompt "start" "$prompt_file" "$log_file"
  assert_no_final_plan_modified "$marker"
  [[ -s "$questions_file" ]] || die "Planner did not write $questions_file"
  validate_grill_questions "$questions_file" || die "Planner wrote invalid choice questions. See the format errors above."
  cp "$questions_file" "$GRILL_DIR/questions.md"
  write_answer_placeholder "$answers_file" "$round"
  cp "$answers_file" "$GRILL_DIR/answers.md"
  echo "Draft plan: $AI_DIR/ai-plan/draft-plan.md"
  echo "Questions: $GRILL_DIR/questions.md"
  echo "Answer in: $GRILL_DIR/answers.md"
}

cmd_next() {
  local round next_round answers_file plan_file prompt_file log_file marker next_questions
  round="$(latest_round)"
  (( round > 0 )) || die "No grill round found. Run: .ai-agent/bin/aia plan grill-start"
  sync_answers_for_round "$round"
  answers_file="$(round_file "$round" "answers")"
  [[ -s "$answers_file" ]] || die "Answer file is empty: $answers_file"
  if answers_approved "$answers_file"; then
    echo "Plan approved in $answers_file"
    echo "Next: .ai-agent/bin/aia plan freeze"
    return 0
  fi
  plan_file="$(latest_plan_file "$round")" || die "No draft or revised plan found."
  next_round=$((round + 1))
  prepare_planner_context
  prompt_file="$RUNTIME/planner-grill-next-round-$(printf '%03d' "$round").prompt.md"
  log_file="$RUNTIME/planner-grill-next-round-$(printf '%03d' "$round").log"
  marker="$RUNTIME/planner-grill-next-round-$(printf '%03d' "$round").marker"
  : > "$marker"
  write_next_prompt "$round" "$next_round" "$prompt_file" "$plan_file" "$answers_file"
  run_planner_grill_prompt "next" "$prompt_file" "$log_file"
  assert_no_final_plan_modified "$marker"
  [[ -s "$(round_file "$round" "revised-plan")" ]] || die "Planner did not write $(round_file "$round" "revised-plan")"
  cp "$(round_file "$round" "revised-plan")" "$AI_DIR/ai-plan/revised-plan.md"
  next_questions="$(round_file "$next_round" "questions")"
  if [[ ! -s "$next_questions" ]]; then
    cat > "$next_questions" <<EOF
# Planner Grill Questions - Round $(printf '%03d' "$next_round")

No blocker questions were written by Planner.

If the revised plan is acceptable, write APPROVED or PLAN APPROVED in answers.md.
If not, write the remaining concerns or corrections.
EOF
  fi
  validate_grill_questions "$next_questions" || die "Planner wrote invalid choice questions. See the format errors above."
  cp "$next_questions" "$GRILL_DIR/questions.md"
  write_answer_placeholder "$(round_file "$next_round" "answers")" "$next_round"
  cp "$(round_file "$next_round" "answers")" "$GRILL_DIR/answers.md"
  echo "Revised plan: $AI_DIR/ai-plan/revised-plan.md"
  echo "Next questions: $GRILL_DIR/questions.md"
  echo "Answer in: $GRILL_DIR/answers.md"
}

cmd_approve() {
  local round answers_file
  round="$(latest_round)"
  (( round > 0 )) || die "No grill round found. Run: .ai-agent/bin/aia plan grill-start"
  answers_file="$(round_file "$round" "answers")"
  printf 'PLAN APPROVED\n' > "$answers_file"
  cp "$answers_file" "$GRILL_DIR/answers.md"
  echo "Plan marked approved in $GRILL_DIR/answers.md"
  cmd_freeze
}

cmd_freeze() {
  local round answers_file plan_file prompt_file log_file
  round="$(latest_round)"
  (( round > 0 )) || die "No grill round found. Run: .ai-agent/bin/aia plan grill-start"
  sync_answers_for_round "$round"
  answers_file="$(round_file "$round" "answers")"
  answers_approved "$answers_file" || die "Plan is not approved. Add APPROVED or PLAN APPROVED to $GRILL_DIR/answers.md first."
  plan_file="$(latest_plan_file "$round")" || die "No draft or revised plan found."
  prepare_planner_context
  prompt_file="$RUNTIME/planner-grill-freeze.prompt.md"
  log_file="$RUNTIME/planner-grill-freeze.log"
  write_freeze_prompt "$prompt_file" "$plan_file" "$answers_file"
  run_planner_grill_prompt "freeze" "$prompt_file" "$log_file"
  if ! find "$AI_DIR/ai-plan/tasks" -maxdepth 1 -type f -name 'task-*.md' | grep -q .; then
    die "Planner did not create task files under $AI_DIR/ai-plan/tasks"
  fi
  echo "Frozen plan:"
  echo "- $AI_DIR/ai-plan/overview.md"
  echo "- $AI_DIR/ai-plan/context.md"
  echo "- $AI_DIR/ai-plan/tasks/"
}

case "${1:-}" in
  start|grill-start) cmd_start ;;
  next|grill-next) cmd_next ;;
  approve) cmd_approve ;;
  freeze) cmd_freeze ;;
  validate-questions) [[ -n "${2:-}" ]] || die "questions file required"; validate_grill_questions "$2" ;;
  -h|--help|help|"") usage ;;
  *) usage >&2; exit 2 ;;
esac
