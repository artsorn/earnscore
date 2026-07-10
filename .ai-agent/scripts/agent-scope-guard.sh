#!/usr/bin/env bash
set -euo pipefail

AI_DIR="${AI_DIR:-.ai-agent}"
RUNTIME_DIR="${RUNTIME_DIR:-$AI_DIR/generated/runtime}"
STATUS_JSON="${STATUS_JSON:-$AI_DIR/generated/status.json}"
LEGACY_STATUS_JSON="${LEGACY_STATUS_JSON:-$RUNTIME_DIR/status.json}"
VERDICT_FILE="${VERDICT_FILE:-$RUNTIME_DIR/loop-verdict.txt}"
EVENTS="${EVENTS:-$RUNTIME_DIR/events.log}"
REVIEWER_DIFF_FILE="${REVIEWER_DIFF_FILE:-$RUNTIME_DIR/reviewer-diff.patch}"
REVIEWER_FILES_FILE="${REVIEWER_FILES_FILE:-$RUNTIME_DIR/reviewer-files.txt}"
REVIEWER_SCOPE_FILE="${REVIEWER_SCOPE_FILE:-$RUNTIME_DIR/reviewer-scope.txt}"
TASK_CHECKPOINT_DIR="${TASK_CHECKPOINT_DIR:-$RUNTIME_DIR/task-checkpoints}"
TASK_CHECKPOINT_LEDGER="${TASK_CHECKPOINT_LEDGER:-$TASK_CHECKPOINT_DIR/accepted-files.tsv}"
mkdir -p "$RUNTIME_DIR" "$(dirname "$STATUS_JSON")"

scope_event() { printf '%s [ScopeGuard] %s\n' "$(date -Iseconds)" "$*" >> "$EVENTS"; }

is_agent_state_path() {
  local f="$1"
  f="${f#./}"
  case "$f" in
    "$AI_DIR"/*|.ai-agent/*|AGENTS.md|.gitignore|\
    "$AI_DIR"/generated/*|"$AI_DIR"/runtime/*|"$AI_DIR"/cache/*|"$AI_DIR"/knowledge/*|"$AI_DIR"/state/*|"$AI_DIR"/backups/*|"$AI_DIR"/tmp/*|"$AI_DIR"/logs/*|"$AI_DIR"/ai-plan/*|\
    .ai-agent/generated/*|.ai-agent/runtime/*|.ai-agent/cache/*|.ai-agent/knowledge/*|.ai-agent/state/*|.ai-agent/backups/*|.ai-agent/tmp/*|.ai-agent/logs/*|.ai-agent/ai-plan/*|\
    .agent/loop-verdict.txt|.agent/requirement.md)
      return 0 ;;
    *) return 1 ;;
  esac
}

review_diff_exclude_pathspecs() {
  printf '%s\n' \
    ":(exclude)$AI_DIR/**" \
    ":(exclude)$AI_DIR/generated/**" \
    ":(exclude)$AI_DIR/runtime/**" \
    ":(exclude)$AI_DIR/cache/**" \
    ":(exclude)$AI_DIR/knowledge/**" \
    ":(exclude)$AI_DIR/state/**" \
    ":(exclude)$AI_DIR/backups/**" \
    ":(exclude)$AI_DIR/tmp/**" \
    ":(exclude)$AI_DIR/logs/**" \
    ":(exclude)$AI_DIR/ai-plan/**" \
    ':(exclude).ai-agent/**' \
    ':(exclude).ai-agent/generated/**' \
    ':(exclude).ai-agent/runtime/**' \
    ':(exclude).ai-agent/cache/**' \
    ':(exclude).ai-agent/knowledge/**' \
    ':(exclude).ai-agent/state/**' \
    ':(exclude).ai-agent/backups/**' \
    ':(exclude).ai-agent/tmp/**' \
    ':(exclude).ai-agent/logs/**' \
    ':(exclude).ai-agent/ai-plan/**' \
    ':(exclude).agent/loop-verdict.txt' \
    ':(exclude).agent/requirement.md' \
    ':(exclude)AGENTS.md' \
    ':(exclude).gitignore'
}

extract_allowed_paths() {
  local task="$1"
  python3 - "$task" <<'PY'
import re, sys
from pathlib import Path
if len(sys.argv) < 2 or not sys.argv[1]:
    sys.exit(0)
try:
    p = Path(sys.argv[1])
except (OSError, ValueError, UnicodeError):
    sys.exit(0)
try:
    if not p.exists():
        sys.exit(0)
    lines = p.read_text(encoding='utf-8', errors='replace').splitlines()
except (OSError, ValueError, UnicodeError):
    sys.exit(0)

positive_headers = {
    'allowed edit area',
    'allowed files',
    'allowed scope',
    'files to edit',
    'implementation files',
    'required files',
    'scope',
    'target files',
}
negative_headers = (
    'forbidden',
    'not in scope',
    'out of scope',
    'out-of-scope',
    'non-goals',
)
in_sec = False
in_fence = False
found = []

def valid_scope_path(raw: str) -> str | None:
    c = (raw or '').strip().strip('"\'')
    c = c.strip('.,;')
    c = c.replace('\\', '/')
    if not c or len(c) >= 512:
        return None
    if any(ch in c for ch in '\n\r\t'):
        return None
    if any(ord(ch) < 32 for ch in c):
        return None
    if c.startswith(('```', 'text', '#', 'http://', 'https://', '/', './../', '../')):
        return None
    if c.startswith(('-', '*', '+', '>', '|')):
        return None
    if re.search(r'\s', c):
        return None
    if any(token in c for token in ['```', '<', '>', '[', ']', '(', ')', ':']):
        return None
    if c.lower() in {'none','n/a','only','file','files','path','paths','and','or'}:
        return None
    if not any(ch in c for ch in ['/', '*', '.', '?']):
        return None
    parts = [part for part in c.split('/') if part]
    if any(part == '..' for part in parts):
        return None
    return c.lstrip('./')

for line in lines:
    raw = line.rstrip()
    if raw.lstrip().startswith('```'):
        in_fence = not in_fence
        continue
    if in_fence:
        continue
    m = re.match(r'^#{1,6}\s+(.+?)\s*$', raw)
    if m:
        name = m.group(1).strip().lower()
        if any(h in name for h in negative_headers):
            in_sec = False
        else:
            in_sec = name in positive_headers
        continue
    if not in_sec:
        continue
    stripped = raw.strip()
    if not stripped or stripped.startswith('>'):
        continue
    bullet = re.match(r'^\s*(?:[-*+]|\d+[.)])\s+(?:\[[ xX]\]\s*)?(.+?)\s*$', raw)
    if not bullet:
        continue
    body = bullet.group(1).strip()
    candidates = re.findall(r'`([^`\n\r]+)`', body) or [body.split()[0] if body.split() else '']
    for c in candidates:
        valid = valid_scope_path(c)
        if valid:
            found.append(valid)
# de-dupe stable
seen=set()
for x in found:
    if x not in seen:
        seen.add(x)
        print(x)
PY
}

path_allowed_by_patterns() {
  local file="$1"; shift || true
  local pat p
  [[ "$#" -gt 0 ]] || return 1
  file="${file#./}"
  for pat in "$@"; do
    p="${pat#./}"
    [[ -z "$p" ]] && continue
    if [[ "$p" == *"/**" ]]; then
      prefix="${p%/**}"
      [[ "$file" == "$prefix" || "$file" == "$prefix/"* ]] && return 0
    elif [[ "$p" == */ ]]; then
      prefix="${p%/}"
      [[ "$file" == "$prefix" || "$file" == "$prefix/"* ]] && return 0
    elif [[ "$p" == *"*"* || "$p" == *"?"* ]]; then
      case "$file" in $p) return 0 ;; esac
    else
      [[ "$file" == "$p" ]] && return 0
    fi
  done
  return 1
}

changed_paths_raw() {
  python3 <<'PY'
import subprocess, sys

def norm(path):
    path = path.replace('\\', '/')
    if path.startswith('./'):
        path = path[2:]
    return path

try:
    out = subprocess.check_output(
        ['git', 'status', '--porcelain=v1', '-z', '--untracked-files=all'],
        stderr=subprocess.DEVNULL,
    )
except Exception:
    sys.exit(0)

parts = out.decode('utf-8', 'replace').split('\0')
seen = set()
i = 0
while i < len(parts):
    entry = parts[i]
    i += 1
    if not entry:
        continue
    status = entry[:2]
    path = norm(entry[3:])
    paths = [path] if path else []
    if ('R' in status or 'C' in status) and i < len(parts) and parts[i]:
        paths.append(norm(parts[i]))
        i += 1
    for p in paths:
        if p and p not in seen:
            seen.add(p)
            print(p)
PY
}

changed_paths_for_scope() {
  local f
  changed_paths_raw | while IFS= read -r f; do
    [[ -z "$f" ]] && continue
    is_agent_state_path "$f" && continue
    printf '%s\n' "$f"
  done
}

path_checkpoint_fingerprint() {
  local f="$1"
  if [[ -e "$f" || -L "$f" ]]; then
    git hash-object --no-filters -- "$f" 2>/dev/null || sha256sum -- "$f" 2>/dev/null | awk '{print $1}'
  else
    printf 'MISSING\n'
  fi
}

checkpoint_accepts_path() {
  local f="$1" expected current
  [[ "${TASK_SCOPE_CARRYOVER:-true}" == "true" ]] || return 1
  [[ -f "$TASK_CHECKPOINT_LEDGER" ]] || return 1
  expected="$(awk -F '\t' -v p="$f" '$2 == p { h = $1 } END { print h }' "$TASK_CHECKPOINT_LEDGER" 2>/dev/null || true)"
  [[ -n "$expected" ]] || return 1
  current="$(path_checkpoint_fingerprint "$f" || true)"
  [[ -n "$current" && "$current" == "$expected" ]]
}

scope_collect_violations() {
  local task="$1" f
  SCOPE_ALLOWED=()
  SCOPE_OUT=()
  SCOPE_CARRIED=()
  mapfile -t SCOPE_ALLOWED < <(extract_allowed_paths "$task")
  if [[ "${#SCOPE_ALLOWED[@]}" -eq 0 ]]; then
    return 2
  fi

  mapfile -t changed < <(changed_paths_for_scope)
  for f in "${changed[@]}"; do
    [[ -z "$f" ]] && continue
    if path_allowed_by_patterns "$f" "${SCOPE_ALLOWED[@]}"; then
      continue
    fi
    if checkpoint_accepts_path "$f"; then
      SCOPE_CARRIED+=("$f")
      continue
    fi
    SCOPE_OUT+=("$f")
  done
}

restore_out_of_scope_paths() {
  local f
  for f in "$@"; do
    [[ -z "$f" ]] && continue
    scope_event "restoring out-of-scope file: $f"
    if git ls-files --error-unmatch -- "$f" >/dev/null 2>&1; then
      git restore --staged --worktree -- "$f" 2>/dev/null || git checkout -- "$f" 2>/dev/null || true
    elif [[ "${AUTO_REMOVE_OUT_OF_SCOPE_UNTRACKED:-false}" == "true" ]]; then
      case "$f" in
        /*|../*|*/../*|"")
          scope_event "skip unsafe untracked path removal: $f"
          ;;
        *)
          rm -rf -- "$f"
          ;;
      esac
    fi
  done
}

scope_guard_preflight() {
  local task="$1" round="${2:-0}"
  if ! scope_collect_violations "$task"; then
    scope_event "no allowed paths parsed from $task; preflight fail closed"
    printf 'BLOCKED\nScopeGuard: no Allowed Edit Area/Allowed Files/Target Files/Scope found in %s\n' "$task" > "$VERDICT_FILE"
    write_scope_status "$task" "$round" "Blocked" "No allowed scope parsed from task"
    return 1
  fi

  if [[ "${#SCOPE_OUT[@]}" -gt 0 ]]; then
    {
      echo 'BLOCKED'
      echo "ScopeGuard: existing implementation diff is outside the current task before coder."
      echo "Finish/checkpoint the previous task, stash it, or move the file into the correct task scope before continuing."
      printf 'Allowed patterns:\n'
      printf -- '- %s\n' "${SCOPE_ALLOWED[@]}"
      if [[ "${#SCOPE_CARRIED[@]}" -gt 0 ]]; then
        printf 'Accepted carryover files:\n'
        printf -- '- %s\n' "${SCOPE_CARRIED[@]}"
      fi
      printf 'Out-of-scope files:\n'
      printf -- '- %s\n' "${SCOPE_OUT[@]}"
    } > "$VERDICT_FILE"
    write_scope_status "$task" "$round" "Blocked" "Existing out-of-scope implementation diff before coder" "${SCOPE_OUT[@]}"
    scope_event "preflight blocked out-of-scope: ${SCOPE_OUT[*]}"
    return 1
  fi

  write_scope_status "$task" "$round" "Passed" "Pre-task scope clean"
  if [[ "${#SCOPE_CARRIED[@]}" -gt 0 ]]; then
    scope_event "preflight passed with accepted carryover: ${SCOPE_CARRIED[*]}"
  else
    scope_event "preflight passed for $task"
  fi
  return 0
}

write_scope_status() {
  local task="$1" round="$2" status="$3" msg="$4"; shift 4 || true
  python3 - "$STATUS_JSON" "$LEGACY_STATUS_JSON" "$task" "$round" "$status" "$msg" "$@" <<'PY'
import json, sys, datetime, os
status_path, legacy_path, task, round_s, status, msg, *files = sys.argv[1:]
data = {
  "updated_at": datetime.datetime.now(datetime.timezone.utc).isoformat(),
  "stage": "ScopeGuard",
  "status": status,
  "task": task,
  "round": int(round_s or 0),
  "message": msg,
  "out_of_scope_files": files,
}
for out in {status_path, legacy_path}:
    if not out: continue
    os.makedirs(os.path.dirname(out), exist_ok=True)
    with open(out, 'w', encoding='utf-8') as f:
        json.dump(data, f, ensure_ascii=False, indent=2)
        f.write('\n')
PY
}

prepare_reviewer_diff() {
  local task="${1:-}" round="${2:-0}" f
  mkdir -p "$RUNTIME_DIR"
  mapfile -t allowed < <(extract_allowed_paths "$task")
  mapfile -t changed < <(changed_paths_for_scope)

  {
    echo "# Reviewer Scope"
    echo "Task: ${task:-unknown}"
    echo "Round: $round"
    echo
    if [[ "${#allowed[@]}" -gt 0 ]]; then
      printf -- '- %s\n' "${allowed[@]}"
    else
      echo "- No allowed paths parsed"
    fi
  } > "$REVIEWER_SCOPE_FILE"

  : > "$REVIEWER_FILES_FILE"
  for f in "${changed[@]}"; do
    [[ -z "$f" ]] && continue
    if [[ "${#allowed[@]}" -eq 0 ]] || path_allowed_by_patterns "$f" "${allowed[@]}"; then
      printf '%s\n' "$f" >> "$REVIEWER_FILES_FILE"
    fi
  done

  mapfile -t reviewer_files < "$REVIEWER_FILES_FILE"
  {
    echo "# Reviewer Diff"
    echo "# Generated by ScopeGuard at $(date -Iseconds)"
    echo "# Excludes agent runtime/state/task metadata."
    echo "# Files: $REVIEWER_FILES_FILE"
    echo "# Scope: $REVIEWER_SCOPE_FILE"
    echo
    if [[ "${#reviewer_files[@]}" -gt 0 ]]; then
      git diff -- "${reviewer_files[@]}" 2>/dev/null || true
      git diff --cached -- "${reviewer_files[@]}" 2>/dev/null || true
    fi
    while IFS= read -r f; do
      [[ -z "$f" ]] && continue
      if ! git ls-files --error-unmatch -- "$f" >/dev/null 2>&1; then
        printf '\n# Untracked allowed file: %s\n' "$f"
        if [[ -f "$f" || -L "$f" ]]; then
          git diff --no-index -- /dev/null "$f" 2>/dev/null || true
        fi
      fi
    done < "$REVIEWER_FILES_FILE"
  } > "$REVIEWER_DIFF_FILE"
}

scope_guard_check() {
  local task="$1" round="${2:-0}"
  if ! scope_collect_violations "$task"; then
    scope_event "no allowed paths parsed from $task; fail closed"
    printf 'FAIL\nScopeGuard: no Allowed Edit Area/Allowed Files/Target Files/Scope found in %s\n' "$task" > "$VERDICT_FILE"
    write_scope_status "$task" "$round" "Failed" "No allowed scope parsed from task"
    return 1
  fi

  if [[ "${#SCOPE_OUT[@]}" -gt 0 && "${AUTO_RESTORE_OUT_OF_SCOPE:-true}" == "true" ]]; then
    restore_out_of_scope_paths "${SCOPE_OUT[@]}"
  fi

  scope_collect_violations "$task" || true

  if [[ "${#SCOPE_OUT[@]}" -gt 0 ]]; then
    {
      echo 'FAIL'
      echo "ScopeGuard: out-of-scope diff detected before reviewer."
      printf 'Allowed patterns:\n'
      printf -- '- %s\n' "${SCOPE_ALLOWED[@]}"
      if [[ "${#SCOPE_CARRIED[@]}" -gt 0 ]]; then
        printf 'Accepted carryover files:\n'
        printf -- '- %s\n' "${SCOPE_CARRIED[@]}"
      fi
      printf 'Out-of-scope files:\n'
      printf -- '- %s\n' "${SCOPE_OUT[@]}"
    } > "$VERDICT_FILE"
    write_scope_status "$task" "$round" "Failed" "Out-of-scope diff detected before reviewer" "${SCOPE_OUT[@]}"
    scope_event "failed out-of-scope: ${SCOPE_OUT[*]}"
    return 1
  fi

  prepare_reviewer_diff "$task" "$round"
  write_scope_status "$task" "$round" "Passed" "Scope clean before reviewer"
  if [[ "${#SCOPE_CARRIED[@]}" -gt 0 ]]; then
    scope_event "passed for $task with accepted carryover: ${SCOPE_CARRIED[*]}"
  else
    scope_event "passed for $task"
  fi
  return 0
}

scope_guard_checkpoint() {
  local task="$1" round="${2:-0}" f hash safe_name task_file
  [[ "${TASK_CHECKPOINT_ON_PASS:-true}" == "true" ]] || return 0
  mapfile -t allowed < <(extract_allowed_paths "$task")
  [[ "${#allowed[@]}" -gt 0 ]] || return 0
  mkdir -p "$TASK_CHECKPOINT_DIR"
  safe_name="$(basename "$task" .md | tr -c 'A-Za-z0-9._-' '_')"
  task_file="$TASK_CHECKPOINT_DIR/${safe_name}.round-${round}.tsv"
  : > "$task_file"

  mapfile -t changed < <(changed_paths_for_scope)
  for f in "${changed[@]}"; do
    [[ -z "$f" ]] && continue
    path_allowed_by_patterns "$f" "${allowed[@]}" || continue
    hash="$(path_checkpoint_fingerprint "$f" || true)"
    [[ -n "$hash" ]] || continue
    printf '%s\t%s\t%s\t%s\n' "$hash" "$f" "$task" "$(date -Iseconds)" >> "$task_file"
    printf '%s\t%s\t%s\t%s\n' "$hash" "$f" "$task" "$(date -Iseconds)" >> "$TASK_CHECKPOINT_LEDGER"
  done

  if [[ -s "$task_file" ]]; then
    scope_event "checkpointed accepted task files for $task -> $task_file"
  else
    rm -f "$task_file"
    scope_event "checkpoint skipped for $task; no changed allowed files"
  fi
  return 0
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  cmd="${1:-check}"; shift || true
  case "$cmd" in
    check) scope_guard_check "${1:?task file required}" "${2:-0}" ;;
    preflight) scope_guard_preflight "${1:?task file required}" "${2:-0}" ;;
    checkpoint) scope_guard_checkpoint "${1:?task file required}" "${2:-0}" ;;
    allowed) extract_allowed_paths "${1:?task file required}" ;;
    changed|files) changed_paths_for_scope ;;
    diff) prepare_reviewer_diff "${1:-}" "${2:-0}"; cat "$REVIEWER_DIFF_FILE" ;;
    prepare-reviewer-diff) prepare_reviewer_diff "${1:-}" "${2:-0}"; echo "$REVIEWER_DIFF_FILE" ;;
    *) echo "Usage: $0 check <task.md> [round] | preflight <task.md> [round] | checkpoint <task.md> [round] | allowed <task.md> | changed | diff [task.md] [round] | prepare-reviewer-diff [task.md] [round]" >&2; exit 2 ;;
  esac
fi
