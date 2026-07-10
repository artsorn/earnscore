#!/usr/bin/env bash
set -euo pipefail

SELF="${BASH_SOURCE[0]}"
AI_DIR="${AI_DIR:-$(cd "$(dirname "$SELF")/.." && pwd)}"
PROJECT_ROOT="${PROJECT_ROOT:-$(cd "$AI_DIR/.." && pwd)}"

if [[ -f "$AI_DIR/scripts/agent-load-env.sh" ]]; then
  # shellcheck disable=SC1090
  source "$AI_DIR/scripts/agent-load-env.sh"
  load_agent_env "$AI_DIR"
fi

CONTEXT_MODE="${CONTEXT_MODE:-minimal}"
CODEGRAPH_MODE="${CODEGRAPH_MODE:-smart}"
LAZY_KNOWLEDGE="${LAZY_KNOWLEDGE:-true}"
if [[ -n "${RUNTIME_DIR:-}" && "$(basename "$AI_DIR")" == ".ai-agent" ]]; then
  if [[ "$RUNTIME_DIR" = /* ]]; then
    RUNTIME="$RUNTIME_DIR"
  else
    RUNTIME="$PROJECT_ROOT/$RUNTIME_DIR"
  fi
else
  RUNTIME="$AI_DIR/generated/runtime"
fi
CACHE="$AI_DIR/generated/cache"
KNOWLEDGE="$AI_DIR/generated/knowledge"
OUT="$RUNTIME/runtime-context.md"

mkdir -p "$RUNTIME"

if [[ "${CONTEXT_SKIP_ENSURE:-false}" != "true" ]]; then
  CODEGRAPH_MODE="$CODEGRAPH_MODE" bash "$AI_DIR/scripts/agent-codegraph.sh" ensure >/dev/null || true
  bash "$AI_DIR/scripts/agent-knowledge-scan.sh" ensure >/dev/null || true
fi

first_lines() {
  local path="$1" lines="${2:-120}"
  [[ -f "$path" ]] || return 0
  sed -n "1,${lines}p" "$path"
}

{
  echo "# Runtime Context"
  echo
  echo "Generated: $(date -Iseconds)"
  echo "Context mode: $CONTEXT_MODE"
  echo "CodeGraph mode: $CODEGRAPH_MODE"
  echo "Project root: $PROJECT_ROOT"
  echo

  if [[ -n "${CURRENT_TASK:-}" && -f "${CURRENT_TASK:-}" ]]; then
    echo "## Current Task"
    echo
    echo "\`\`\`md"
    first_lines "$CURRENT_TASK" 220
    echo "\`\`\`"
    echo
  fi

  if [[ -f "$PROJECT_ROOT/.agent/requirement.md" ]]; then
    echo "## Requirement"
    echo
    echo "\`\`\`md"
    first_lines "$PROJECT_ROOT/.agent/requirement.md" 180
    echo "\`\`\`"
    echo
  fi

  echo "## Project Summary"
  echo
  if [[ -f "$CACHE/project-summary.json" ]]; then
    echo "\`\`\`json"
    first_lines "$CACHE/project-summary.json" 120
    echo "\`\`\`"
  else
    echo "(missing .ai-agent/generated/cache/project-summary.json)"
  fi
  echo

  echo "## Reference Map"
  echo
  echo "- Prefer the compact context package before broad source exploration."
  echo "- Compact package: \`.ai-agent/generated/runtime/context-package.md\`"
  echo "- Search allowlist: \`.ai-agent/generated/runtime/search-allowlist.txt\`"
  echo "- CodeGraph project: \`.ai-agent/generated/cache/codegraph-project.md\`"
  echo "- CodeGraph task/changed subset: \`.ai-agent/generated/cache/codegraph-lite.md\`"
  echo "- Architecture knowledge: \`.ai-agent/generated/knowledge/architecture.md\`"
  echo "- API knowledge: \`.ai-agent/generated/knowledge/api.md\`"
  echo "- Database knowledge: \`.ai-agent/generated/knowledge/database.md\`"
  echo "- Frontend knowledge: \`.ai-agent/generated/knowledge/frontend.md\`"
  echo "- Documentation knowledge: \`.ai-agent/generated/knowledge/documentation.md\`"
  echo "- Reviewer diff: \`.ai-agent/generated/runtime/reviewer-diff.patch\`"
  echo

  echo "## CodeGraph Lite"
  echo
  first_lines "$CACHE/codegraph-lite.md" 260
  echo

  case "$CONTEXT_MODE" in
    balanced|strict|full)
      if [[ "$LAZY_KNOWLEDGE" == "true" ]]; then
        echo "## Lazy Knowledge"
        echo
        echo "Knowledge excerpts are selected in \`.ai-agent/generated/runtime/context-package.md\` based on the current task, allowed files, and role."
        echo "Do not preload every generated knowledge document unless \`LAZY_KNOWLEDGE=false\` or the task explicitly requires broad project planning."
        echo
      else
        echo "## Architecture Knowledge"
        echo
        first_lines "$KNOWLEDGE/architecture.md" 220
        echo
        echo "## API Knowledge"
        echo
        first_lines "$KNOWLEDGE/api.md" 220
        echo
        echo "## Database Knowledge"
        echo
        first_lines "$KNOWLEDGE/database.md" 220
        echo
        echo "## Frontend Knowledge"
        echo
        first_lines "$KNOWLEDGE/frontend.md" 220
        echo
      fi
      ;;
  esac

  case "$CONTEXT_MODE" in
    strict|full)
      echo "## Project CodeGraph Excerpt"
      echo
      first_lines "$CACHE/codegraph-project.md" 500
      echo
      ;;
  esac
} > "$OUT"

if [[ -x "$AI_DIR/scripts/agent-context-package.sh" ]]; then
  CONTEXT_ROLE="${CONTEXT_ROLE:-context}" bash "$AI_DIR/scripts/agent-context-package.sh" >/dev/null || true
fi

echo "$OUT"
