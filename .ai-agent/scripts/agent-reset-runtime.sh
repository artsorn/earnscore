#!/usr/bin/env bash
set -euo pipefail
cmd="${1:-status}"
case "$(basename "$0")" in
  agent-git-paths.sh)
    agent_filtered_status() { git status --short | grep -vE '^.. \.ai-agent/(generated|runtime|cache|logs|tmp|knowledge|state)/' || true; }
    ;;
  agent-context-build.sh)
    AI_DIR="${AI_DIR:-.ai-agent}"; mkdir -p "$AI_DIR/generated/runtime"; echo "# Runtime Context" > "$AI_DIR/generated/runtime/runtime-context.md"; echo "Generated: $(date -Iseconds)" >> "$AI_DIR/generated/runtime/runtime-context.md" ;;
  update-manifest.sh)
    AI_DIR="${AI_DIR:-$(cd "$(dirname "$0")/.." && pwd)}"; (cd "$AI_DIR" && find . -type f ! -path './generated/*' ! -path './state/*' ! -path './backups/*' ! -path './config/user.env' ! -name MANIFEST.sha256 -print0 | sort -z | xargs -0 sha256sum | sed 's#  \./#  #' > MANIFEST.sha256) ;;
  *) echo "$(basename "$0") $cmd: no-op lite helper" ;;
esac
