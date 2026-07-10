#!/usr/bin/env bash
set -euo pipefail

AI_DIR="${AI_DIR:-$(cd "$(dirname "$0")/.." && pwd)}"

cd "$AI_DIR"
find . -type f \
  ! -path './generated/*' \
  ! -path './runtime/*' \
  ! -path './cache/*' \
  ! -path './knowledge/*' \
  ! -path './custom/*' \
  ! -path './state/*' \
  ! -path './backups/*' \
  ! -path './config/user.env' \
  ! -name 'MANIFEST.sha256' \
  ! -name '.DS_Store' \
  -print0 \
| sort -z \
| xargs -0 sha256sum \
| sed 's#  \./#  #' > MANIFEST.sha256
