#!/usr/bin/env bash
set -euo pipefail

AI_DIR="${AI_DIR:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
bash "$AI_DIR/scripts/agent-planner-grill.sh" start "$@"
