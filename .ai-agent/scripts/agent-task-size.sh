#!/usr/bin/env bash
set -euo pipefail

agent_runtime_dir() {
  local ai_dir="${1:-${AI_DIR:-.ai-agent}}" project_root="${2:-}"
  if [[ -z "$project_root" ]]; then
    project_root="$(cd "$ai_dir/.." && pwd)"
  fi
  if [[ -n "${RUNTIME_DIR:-}" && "$(basename "$ai_dir")" == ".ai-agent" ]]; then
    if [[ "$RUNTIME_DIR" = /* ]]; then
      printf '%s\n' "$RUNTIME_DIR"
    else
      printf '%s\n' "$project_root/$RUNTIME_DIR"
    fi
  else
    printf '%s\n' "$ai_dir/generated/runtime"
  fi
}

agent_var_explicit() {
  local name="$1"
  case " ${AGENT_PRESET_ENV_KEYS:-} ${AGENT_USER_ENV_KEYS:-} " in
    *" $name "*) return 0 ;;
    *) return 1 ;;
  esac
}

agent_task_size_classify() {
  local ai_dir="${1:-${AI_DIR:-.ai-agent}}" project_root="${2:-}" runtime
  if [[ -z "$project_root" ]]; then
    project_root="$(cd "$ai_dir/.." && pwd)"
  fi
  runtime="$(agent_runtime_dir "$ai_dir" "$project_root")"
  mkdir -p "$runtime"
  python3 - "$project_root" "$ai_dir" "$runtime" "${TASK_SIZE_OVERRIDE:-}" "${TASK_SIZE_AUTO:-true}" "${CURRENT_TASK:-}" <<'PY'
import hashlib
import json
import os
import re
import sys
from pathlib import Path

root = Path(sys.argv[1]).resolve()
ai_dir = Path(sys.argv[2]).resolve()
runtime = Path(sys.argv[3]).resolve()
override = (sys.argv[4] or "").strip().upper()
auto = (sys.argv[5] or "true").strip().lower() not in {"0", "false", "no", "off"}
current_task_arg = sys.argv[6] if len(sys.argv) > 6 else ""

def read(path):
    try:
        return Path(path).read_text(encoding="utf-8", errors="replace")
    except Exception:
        return ""

def rel_exists(value):
    try:
        return (root / value).exists()
    except Exception:
        return False

def add_if_exists(items, *values):
    for value in values:
        if rel_exists(value) and value not in items:
            items.append(value)

def parse_task_scope(text):
    positive_headers = {
        "allowed edit area",
        "allowed files",
        "allowed scope",
        "files to edit",
        "implementation files",
        "implementation scope",
        "required files",
        "scope",
        "target files",
    }
    ignored_headers = {
        "reference map",
        "reference files",
        "references",
        "forbidden implementation scope",
        "forbidden scope",
        "validation commands",
        "reviewer checklist",
    }
    lines = []
    section = ""
    in_fence = False
    for raw in text.splitlines():
        if raw.lstrip().startswith("```"):
            in_fence = not in_fence
            continue
        if in_fence:
            continue
        header = re.match(r"^#{1,6}\s+(.+?)\s*$", raw)
        if header:
            name = header.group(1).strip().lower()
            if name in positive_headers:
                section = "positive"
            elif name in ignored_headers or "forbidden" in name:
                section = ""
            else:
                section = "general"
            continue
        if section in {"positive", "general"}:
            lines.append(raw)
    return "\n".join(lines)

def normalize_ref(value):
    value = (value or "").strip().strip("'\"").lstrip("./")
    value = value.replace("\\", "/")
    if not value:
        return ""
    blocked_prefixes = (
        ".ai-agent/generated/",
        ".ai-agent/runtime/",
        ".ai-agent/generated",
        ".ai-agent/runtime",
        "ai-agent/generated/",
        "ai-agent/runtime/",
        "agent/generated/",
        "agent/runtime/",
        ".agent/",
    )
    blocked_exact = {
        ".ai-agent/generated/runtime/context-package.md",
        ".ai-agent/generated/runtime/runtime-context.md",
    }
    if value in blocked_exact or value.startswith(blocked_prefixes):
        return ""
    return value

requirement_path = root / ".agent" / "requirement.md"
requirement = read(requirement_path)
task_text = ""
if current_task_arg:
    task_path = Path(current_task_arg)
    if not task_path.is_absolute():
        task_path = root / task_path
    task_text = read(task_path)
task_scope_text = parse_task_scope(task_text)
blob = (requirement + "\n" + task_scope_text).lower()
blob_for_classification = re.sub(r"(?:^|\s)(?:\.?ai-agent|\.agent)/[^\s`]+", " ", blob)

file_refs = set()
for raw in re.findall(r"`([^`\n]+\.[A-Za-z0-9]+)`", requirement + "\n" + task_scope_text):
    value = normalize_ref(raw)
    if value:
        file_refs.add(value)
for raw in re.findall(r"\b(?:src|public|app|pages|tests|migrations|scripts|agent|\.ai-agent)/[A-Za-z0-9_./-]+\.[A-Za-z0-9]+\b", requirement + "\n" + task_scope_text):
    value = normalize_ref(raw)
    if value:
        file_refs.add(value)

large_re = re.compile(
    r"\b(database|schema|migration|migrations|auth|permission|permissions|role|roles|rbac|"
    r"architecture|workflow|cross-cutting|หลาย module|multiple modules|agent framework|scopeguard|"
    r"planner|coder|reviewer|final reviewer|token guard|codegraph)\b",
    re.I,
)
backend_re = re.compile(r"\b(api|route|router|endpoint|backend|worker|handler|src/|server)\b", re.I)
frontend_re = re.compile(r"\b(frontend|ui|css|layout|html|landing|footer|header|button|link|image|copy|text|content|public/)\b", re.I)
admin_re = re.compile(r"\b(admin|dashboard|backoffice|public/admin)\b", re.I)
small_re = re.compile(r"\b(text|content|copy|copywriting|css|layout|footer|header|landing|button|link|image|single file|one file|สี|ข้อความ|ลิงก์|ปุ่ม|รูป)\b", re.I)

if override in {"SMALL", "MEDIUM", "LARGE"}:
    size = override
    reason = "TASK_SIZE_OVERRIDE"
elif not auto:
    size = "MEDIUM"
    reason = "TASK_SIZE_AUTO=false"
elif large_re.search(blob_for_classification) or len(file_refs) > 5:
    size = "LARGE"
    reason = "large keywords or more than five referenced files"
elif (
    len(file_refs) <= 5
    and not backend_re.search(blob_for_classification)
    and not re.search(r"\b(database|schema|migration|migrations|sql|d1|sqlite|auth|permission)\b", blob_for_classification)
    and all(ref.startswith(("public/", "app/", "pages/")) for ref in file_refs)
    and (small_re.search(blob_for_classification) or admin_re.search(blob_for_classification) or frontend_re.search(blob_for_classification))
):
    size = "SMALL"
    reason = "small frontend/content/admin scope with limited implementation files"
elif backend_re.search(blob_for_classification) and frontend_re.search(blob_for_classification):
    size = "MEDIUM"
    reason = "combined frontend/backend scope"
elif backend_re.search(blob_for_classification) or 2 <= len(file_refs) <= 5:
    size = "MEDIUM"
    reason = "backend/API or 2-5 referenced files"
elif small_re.search(blob_for_classification) or admin_re.search(blob_for_classification) or frontend_re.search(blob_for_classification):
    size = "SMALL"
    reason = "small frontend/content/admin signal"
else:
    size = "MEDIUM"
    reason = "default when requirement is not clearly small or large"

task_type = "general"
if re.search(r"\b(agent framework|workflow|planner|coder|reviewer|scopeguard|token guard|codegraph)\b", blob_for_classification):
    task_type = "agent"
elif re.search(r"\b(database|schema|migration|migrations|sql|d1|sqlite)\b", blob_for_classification):
    task_type = "database"
elif backend_re.search(blob_for_classification):
    task_type = "backend"
elif admin_re.search(blob_for_classification):
    task_type = "admin"
elif frontend_re.search(blob_for_classification):
    task_type = "frontend"

inferred = []
if task_type == "admin":
    add_if_exists(inferred, "public/admin/index.html", "public/admin.js", "public/admin.css")
    if rel_exists("public/admin"):
        inferred.append("public/admin/")
elif task_type == "frontend":
    add_if_exists(inferred, "public/index.html")
    if rel_exists("public/css"):
        inferred.append("public/css/")
    if rel_exists("public/js"):
        inferred.append("public/js/")
    if rel_exists("public/assets"):
        inferred.append("public/assets/")
elif task_type == "backend":
    if rel_exists("src"):
        inferred.append("src/")
    if rel_exists("tests"):
        inferred.append("tests/")
    add_if_exists(inferred, "src/lib.rs", "src/main.rs")
    if re.search(r"\b(schema|migration|database|sql|d1)\b", blob_for_classification):
        add_if_exists(inferred, "schema.sql")
        if rel_exists("migrations"):
            inferred.append("migrations/")
elif task_type == "database":
    add_if_exists(inferred, "schema.sql")
    if rel_exists("migrations"):
        inferred.append("migrations/")
    if rel_exists("src"):
        inferred.append("src/")
elif task_type == "agent":
    add_if_exists(inferred, ".ai-agent/AGENTS.md", "AGENTS.md")
    if rel_exists(".ai-agent/scripts"):
        inferred.append(".ai-agent/scripts/")
    if rel_exists(".ai-agent/prompts"):
        inferred.append(".ai-agent/prompts/")
    if rel_exists(".ai-agent/workflows"):
        inferred.append(".ai-agent/workflows/")

for ref in sorted(file_refs):
    value = normalize_ref(ref)
    if value and value not in inferred:
        inferred.append(value)

req_hash = hashlib.sha256(requirement.encode("utf-8")).hexdigest() if requirement else ""
hash_path = runtime / "requirement.sha256"
previous_hash = read(hash_path).strip()
changed = bool(req_hash and previous_hash and req_hash != previous_hash)
isolation = "changed" if changed else "same-or-new"

(runtime / "task-size.txt").write_text(size + "\n", encoding="utf-8")
(runtime / "task-type.txt").write_text(task_type + "\n", encoding="utf-8")
(runtime / "inferred-allowed-area.txt").write_text("\n".join(inferred).rstrip() + ("\n" if inferred else ""), encoding="utf-8")
(runtime / "planning-isolation.txt").write_text(isolation + "\n", encoding="utf-8")
if req_hash:
    hash_path.write_text(req_hash + "\n", encoding="utf-8")

def cfg(name, default):
    value = os.environ.get(name)
    return value if value not in {None, ""} else default

budgets = {
    "SMALL": {
        "planner_context_tokens": cfg("SMALL_PLANNER_CONTEXT_TOKENS", "15000"),
        "coder_context_tokens": cfg("SMALL_CODER_CONTEXT_TOKENS", "25000"),
        "search_budget": cfg("SMALL_SEARCH_BUDGET", "5"),
        "codegraph_depth": cfg("SMALL_CODEGRAPH_DEPTH", "1"),
        "max_context_level": cfg("SMALL_MAX_CONTEXT_LEVEL", "1"),
        "context_target_tokens": cfg("SMALL_CONTEXT_TARGET_TOKENS", "25000"),
        "context_hard_cap_tokens": cfg("SMALL_CONTEXT_HARD_CAP_TOKENS", "40000"),
        "max_tasks_from_planner": cfg("SMALL_MAX_TASKS_FROM_PLANNER", "1"),
        "planner_level": "medium",
        "coder_level": "medium",
        "reviewer_level": "medium",
        "final_reviewer_level": "high",
        "planner_task_detail_level": "low",
    },
    "MEDIUM": {
        "planner_context_tokens": cfg("MEDIUM_PLANNER_CONTEXT_TOKENS", "30000"),
        "coder_context_tokens": cfg("MEDIUM_CODER_CONTEXT_TOKENS", "50000"),
        "search_budget": cfg("MEDIUM_SEARCH_BUDGET", "10"),
        "codegraph_depth": cfg("MEDIUM_CODEGRAPH_DEPTH", "2"),
        "max_context_level": cfg("MEDIUM_MAX_CONTEXT_LEVEL", "2"),
        "context_target_tokens": cfg("MEDIUM_CONTEXT_TARGET_TOKENS", "50000"),
        "context_hard_cap_tokens": cfg("MEDIUM_CONTEXT_HARD_CAP_TOKENS", "80000"),
        "max_tasks_from_planner": cfg("MEDIUM_MAX_TASKS_FROM_PLANNER", "3"),
        "planner_level": "high",
        "coder_level": "high",
        "reviewer_level": "medium",
        "final_reviewer_level": "high",
        "planner_task_detail_level": "high",
    },
    "LARGE": {
        "planner_context_tokens": cfg("LARGE_PLANNER_CONTEXT_TOKENS", "80000"),
        "coder_context_tokens": cfg("LARGE_CODER_CONTEXT_TOKENS", "120000"),
        "search_budget": cfg("LARGE_SEARCH_BUDGET", "20"),
        "codegraph_depth": cfg("LARGE_CODEGRAPH_DEPTH", "3"),
        "max_context_level": cfg("LARGE_MAX_CONTEXT_LEVEL", "3"),
        "context_target_tokens": cfg("LARGE_CONTEXT_TARGET_TOKENS", "120000"),
        "context_hard_cap_tokens": cfg("LARGE_CONTEXT_HARD_CAP_TOKENS", "160000"),
        "max_tasks_from_planner": cfg("LARGE_MAX_TASKS_FROM_PLANNER", "unlimited"),
        "planner_level": "high",
        "coder_level": "high",
        "reviewer_level": "medium",
        "final_reviewer_level": "xhigh",
        "planner_task_detail_level": "high",
    },
}[size]
env_lines = [f"TASK_SIZE={size}", f"TASK_TYPE={task_type}", f"TASK_SIZE_REASON={reason}", f"PLANNING_ISOLATION={isolation}"]
env_lines.extend(f"{key.upper()}={value}" for key, value in budgets.items())
(runtime / "task-size.env").write_text("\n".join(env_lines) + "\n", encoding="utf-8")
(runtime / "task-size.json").write_text(json.dumps({
    "task_size": size,
    "task_type": task_type,
    "reason": reason,
    "planning_isolation": isolation,
    "inferred_allowed_area": inferred,
    "budgets": budgets,
}, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
print(size)
PY
}

agent_task_size_load_env_file() {
  local runtime="$1" line key value
  [[ -f "$runtime/task-size.env" ]] || return 0
  while IFS= read -r line || [[ -n "$line" ]]; do
    [[ "$line" == *=* ]] || continue
    key="${line%%=*}"
    value="${line#*=}"
    if agent_var_explicit "$key"; then
      continue
    fi
    printf -v "$key" '%s' "$value"
    export "$key"
  done < "$runtime/task-size.env"
}

agent_task_adaptive_apply() {
  local role="${1:-context}" ai_dir="${2:-${AI_DIR:-.ai-agent}}" project_root="${3:-}" runtime size
  if [[ -z "$project_root" ]]; then
    project_root="$(cd "$ai_dir/.." && pwd)"
  fi
  runtime="$(agent_runtime_dir "$ai_dir" "$project_root")"
  mkdir -p "$runtime"
  size="$(agent_task_size_classify "$ai_dir" "$project_root" | tail -n1)"
  agent_task_size_load_env_file "$runtime"

  case "$role" in
    planner|planner-final-fix)
      if ! agent_var_explicit MAX_CONTEXT_TOKENS && ! agent_var_explicit PLANNER_CONTEXT_TOKENS; then
        export MAX_CONTEXT_TOKENS="${PLANNER_CONTEXT_TOKENS:-${MAX_CONTEXT_TOKENS:-50000}}"
      fi
      if ! agent_var_explicit PLANNER_LEVEL; then export PLANNER_LEVEL="${PLANNER_LEVEL:-high}"; fi
      if ! agent_var_explicit PLANNER_TASK_DETAIL_LEVEL; then export PLANNER_TASK_DETAIL_LEVEL="${PLANNER_TASK_DETAIL_LEVEL:-high}"; fi
      ;;
    coder|reviewer|reviewer-final|reviewer-final-fix|context)
      if ! agent_var_explicit MAX_CONTEXT_TOKENS && ! agent_var_explicit CODER_CONTEXT_TOKENS; then
        export MAX_CONTEXT_TOKENS="${CODER_CONTEXT_TOKENS:-${MAX_CONTEXT_TOKENS:-50000}}"
      fi
      ;;
  esac

  if ! agent_var_explicit SEARCH_BUDGET; then export SEARCH_BUDGET="${SEARCH_BUDGET:-10}"; fi
  if ! agent_var_explicit CODEGRAPH_DEPTH; then export CODEGRAPH_DEPTH="${CODEGRAPH_DEPTH:-2}"; fi
  if [[ "$role" == "coder" ]] && ! agent_var_explicit CODER_LEVEL; then export CODER_LEVEL="${CODER_LEVEL:-high}"; fi
  if [[ "$role" == "reviewer" || "$role" == "reviewer-final-fix" ]] && ! agent_var_explicit REVIEWER_LEVEL; then export REVIEWER_LEVEL="${REVIEWER_LEVEL:-medium}"; fi
  if [[ "$role" == "reviewer-final" ]] && ! agent_var_explicit FINAL_REVIEWER_LEVEL; then export FINAL_REVIEWER_LEVEL="${FINAL_REVIEWER_LEVEL:-high}"; fi

  printf '%s\n' "$size" > "$runtime/task-size.txt"
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  AI_DIR="${AI_DIR:-$(cd "$(dirname "$0")/.." && pwd)}"
  PROJECT_ROOT="${PROJECT_ROOT:-$(cd "$AI_DIR/.." && pwd)}"
  cmd="${1:-classify}"
  case "$cmd" in
    classify) agent_task_size_classify "$AI_DIR" "$PROJECT_ROOT" ;;
    apply) agent_task_adaptive_apply "${2:-context}" "$AI_DIR" "$PROJECT_ROOT" ;;
    *) echo "Usage: $0 classify|apply [role]" >&2; exit 2 ;;
  esac
fi
