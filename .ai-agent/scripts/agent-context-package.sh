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

if [[ -n "${RUNTIME_DIR:-}" && "$(basename "$AI_DIR")" == ".ai-agent" ]]; then
  if [[ "$RUNTIME_DIR" = /* ]]; then
    RUNTIME="$RUNTIME_DIR"
  else
    RUNTIME="$PROJECT_ROOT/$RUNTIME_DIR"
  fi
else
  RUNTIME="$AI_DIR/generated/runtime"
fi

mkdir -p "$RUNTIME"

if [[ -f "$AI_DIR/scripts/agent-task-size.sh" ]]; then
  # shellcheck disable=SC1090
  source "$AI_DIR/scripts/agent-task-size.sh"
  agent_task_adaptive_apply "${CONTEXT_ROLE:-${ROLE:-context}}" "$AI_DIR" "$PROJECT_ROOT" >/dev/null || true
fi

python3 - "$PROJECT_ROOT" "$AI_DIR" "$RUNTIME" <<'PY'
import datetime
import json
import os
import re
import subprocess
import sys
from pathlib import Path

root = Path(sys.argv[1]).resolve()
ai_dir = Path(sys.argv[2]).resolve()
runtime = Path(sys.argv[3]).resolve()
cache = ai_dir / "generated" / "cache"
knowledge = ai_dir / "generated" / "knowledge"


def env_bool(name, default=False):
    value = os.environ.get(name)
    if value is None:
        return default
    return value.strip().lower() in {"1", "true", "yes", "on"}


def env_int(name, default, minimum=0):
    try:
        value = int(os.environ.get(name, str(default)))
    except ValueError:
        value = default
    return max(minimum, value)


role = os.environ.get("CONTEXT_ROLE") or os.environ.get("ROLE") or "context"
context_mode = os.environ.get("CONTEXT_MODE", "minimal")
context_escalation = env_bool("CONTEXT_ESCALATION", True)
context_default_level = (os.environ.get("CONTEXT_DEFAULT_LEVEL") or "auto").strip().lower()
token_guard = env_bool("TOKEN_GUARD", True)
configured_max_context_tokens = env_int("MAX_CONTEXT_TOKENS", 50000, 4000)
search_budget = env_int("SEARCH_BUDGET", 10, 1)
configured_codegraph_depth = env_int("CODEGRAPH_DEPTH", 2, 0)
lazy_knowledge = env_bool("LAZY_KNOWLEDGE", True)
compact_review = env_bool("COMPACT_REVIEW", True)
repair_mode = env_bool("REPAIR_MODE", False)
allow_runtime_log_read = env_bool("ALLOW_RUNTIME_LOG_READ", False) or repair_mode
allow_history_search = env_bool("ALLOW_HISTORY_SEARCH", False)
allow_context_escalation_on_failure = env_bool("ALLOW_CONTEXT_ESCALATION_ON_FAILURE", True)
escalate_on_review_fail = env_bool("ESCALATE_ON_REVIEW_FAIL", True)
escalate_on_build_fail = env_bool("ESCALATE_ON_BUILD_FAIL", True)
context_manifest_enabled = env_bool("CONTEXT_MANIFEST", True)
source_max_lines = env_int("CONTEXT_SOURCE_MAX_LINES", 180, 20)
runtime_max_lines = env_int("CONTEXT_RUNTIME_MAX_LINES", 140, 20)
knowledge_max_lines = env_int("CONTEXT_KNOWLEDGE_MAX_LINES", 160, 20)
review_diff_max_lines = env_int("CONTEXT_REVIEW_DIFF_MAX_LINES", 700, 80)

out_path = runtime / "context-package.md"
allowlist_path = runtime / "search-allowlist.txt"
meta_path = runtime / "context-package.meta.json"
manifest_path = runtime / "context-manifest.txt"
escalation_path = runtime / "context-escalation.txt"

TEXT_EXTENSIONS = {
    ".rs",
    ".js",
    ".jsx",
    ".ts",
    ".tsx",
    ".mjs",
    ".cjs",
    ".html",
    ".css",
    ".sql",
    ".toml",
    ".json",
    ".md",
    ".yml",
    ".yaml",
    ".sh",
    ".py",
}
SPECIAL_FILES = {
    "Cargo.toml",
    "package.json",
    "wrangler.toml",
    "schema.sql",
    "README.md",
}
EXCLUDE_DIRS = {
    ".git",
    "node_modules",
    "target",
    "dist",
    "build",
    "coverage",
    ".next",
    ".nuxt",
    ".wrangler",
    "__pycache__",
}
RUNTIME_ALLOWLIST = {
    ".ai-agent/generated/runtime/runtime-context.md",
    ".ai-agent/generated/runtime/context-package.md",
    ".ai-agent/generated/runtime/reviewer-summary.md",
    ".ai-agent/generated/runtime/reviewer-files.txt",
    ".ai-agent/generated/runtime/reviewer-scope.txt",
    ".ai-agent/generated/runtime/reviewer-diff.patch",
    ".ai-agent/generated/runtime/loop-verdict.txt",
    ".ai-agent/generated/runtime/final-verdict.txt",
    ".ai-agent/generated/runtime/task-size.txt",
    ".ai-agent/generated/runtime/task-type.txt",
    ".ai-agent/generated/runtime/context-manifest.txt",
    ".ai-agent/generated/runtime/context-escalation.txt",
}
RUNTIME_BLOCKED_PATTERNS = [
    ".ai-agent/generated/runtime/*.log",
    ".ai-agent/generated/runtime/coder-round-*.log",
    ".ai-agent/generated/runtime/reviewer-round-*.log",
    ".ai-agent/generated/runtime/token-usage*",
    ".ai-agent/generated/runtime/*.jsonl",
]
AUTO_SEARCH_IGNORES = [
    ".ai-agent/generated/runtime",
    ".ai-agent/generated/cache",
    ".ai-agent/generated/tmp",
    ".ai-agent/generated/history",
    ".ai-agent/generated/token*",
]
CONTEXT_LEVEL_LABELS = {
    0: "Minimal Context",
    1: "Target Files",
    2: "Direct References",
    3: "Relevant Knowledge",
    4: "Focused Project Search",
    5: "Broad Search",
}

try:
    AGENT_GENERATED_PREFIX = (ai_dir / "generated").resolve().relative_to(root).as_posix() + "/"
except Exception:
    AGENT_GENERATED_PREFIX = ".ai-agent/generated/"
try:
    AGENT_RUNTIME_PREFIX = runtime.resolve().relative_to(root).as_posix() + "/"
except Exception:
    AGENT_RUNTIME_PREFIX = ".ai-agent/generated/runtime/"


def generated_at():
    return datetime.datetime.now(datetime.timezone.utc).astimezone().isoformat(timespec="seconds")


def estimate_tokens(text):
    return max(1, (len(text) + 3) // 4)


def rel(path):
    return path.resolve().relative_to(root).as_posix()


def resolve_path(value):
    if not value:
        return None
    p = Path(value)
    if not p.is_absolute():
        p = root / value
    try:
        resolved = p.resolve()
        resolved.relative_to(root)
        return resolved
    except Exception:
        return None


def safe_read(path, max_chars=None):
    try:
        text = path.read_text(encoding="utf-8", errors="replace")
    except Exception:
        return ""
    if max_chars is not None and len(text) > max_chars:
        return text[:max_chars] + "\n...[truncated]\n"
    return text


def first_lines(path, limit):
    text = safe_read(path)
    if not text:
        return ""
    lines = text.splitlines()
    suffix = "\n...[truncated]\n" if len(lines) > limit else "\n"
    return "\n".join(lines[:limit]) + suffix


def compact_lines(values, empty="(none)"):
    values = [v for v in values if v]
    return "\n".join(f"- {v}" for v in values) if values else f"- {empty}"


def is_agent_generated_rel(value):
    value = value.replace("\\", "/").lstrip("./")
    return (
        value.startswith(".ai-agent/generated/")
        or value.startswith(".ai-agent/runtime/")
        or value.startswith(AGENT_GENERATED_PREFIX)
        or value.startswith(AGENT_RUNTIME_PREFIX)
    )


def is_runtime_artifact_rel(value):
    value = value.replace("\\", "/").lstrip("./")
    if value in RUNTIME_ALLOWLIST:
        return False
    blocked_prefixes = (
        ".ai-agent/generated/runtime/",
        ".ai-agent/generated/cache/",
        ".ai-agent/generated/tmp/",
        ".ai-agent/generated/history/",
        AGENT_RUNTIME_PREFIX,
        AGENT_GENERATED_PREFIX + "cache/",
        AGENT_GENERATED_PREFIX + "tmp/",
        AGENT_GENERATED_PREFIX + "history/",
    )
    return value.startswith(blocked_prefixes) or value.startswith(".ai-agent/generated/token") or value.startswith(AGENT_GENERATED_PREFIX + "token")


def valid_repo_reference(raw):
    value = (raw or "").strip().strip("\"'")
    value = value.strip(".,;")
    value = value.replace("\\", "/")
    if not value or len(value) >= 512:
        return None
    if any(ch in value for ch in "\n\r\t"):
        return None
    if any(ord(ch) < 32 for ch in value):
        return None
    if value.startswith(("```", "text", "#", "http://", "https://", "/", "./../", "../")):
        return None
    if value.startswith(("-", "*", "+", ">", "|")):
        return None
    if re.search(r"\s", value):
        return None
    if any(token in value for token in ["```", "<", ">", "[", "]", "(", ")", ":"]):
        return None
    if value.lower() in {"none", "n/a", "only", "file", "files", "path", "paths", "and", "or"}:
        return None
    if not any(ch in value for ch in ["/", "*", ".", "?"]):
        return None
    parts = [part for part in value.split("/") if part]
    if any(part == ".." for part in parts):
        return None
    return value.lstrip("./")


def current_task_path():
    for value in [os.environ.get("CURRENT_TASK", "")]:
        p = resolve_path(value)
        if p and p.is_file():
            return p
    for status in [ai_dir / "generated" / "status.json", runtime / "status.json"]:
        try:
            value = json.loads(status.read_text(encoding="utf-8")).get("task") or ""
        except Exception:
            continue
        p = resolve_path(value)
        if p and p.is_file():
            return p
    return None


def parse_task(task):
    result = {
        "allowed": [],
        "references": [],
        "validation": [],
        "acceptance": [],
        "text": "",
    }
    if not task:
        return result
    text = safe_read(task)
    result["text"] = text
    positive_headers = {
        "allowed edit area",
        "allowed files",
        "allowed scope",
        "files to edit",
        "implementation files",
        "required files",
        "scope",
        "target files",
    }
    reference_headers = {"reference files", "reference map", "references"}
    validation_headers = {"validation", "validation commands", "tests"}
    acceptance_headers = {"acceptance criteria", "acceptance"}
    negative_headers = ("forbidden", "not in scope", "out of scope", "out-of-scope", "non-goals")
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
            if any(h in name for h in negative_headers):
                section = ""
            elif name in positive_headers:
                section = "allowed"
            elif name in reference_headers:
                section = "references"
            elif name in validation_headers:
                section = "validation"
            elif name in acceptance_headers:
                section = "acceptance"
            else:
                section = ""
            continue
        stripped = raw.strip()
        if not stripped or stripped.startswith(">"):
            continue
        bullet = re.match(r"^\s*(?:[-*+]|\d+[.)])\s+(?:\[[ xX]\]\s*)?(.+?)\s*$", raw)
        if not bullet:
            continue
        body = bullet.group(1).strip()
        if section in {"allowed", "references"}:
            tokens = re.findall(r"`([^`\n\r]+)`", body) or [body.split()[0] if body.split() else ""]
            for token in tokens:
                valid = valid_repo_reference(token)
                if valid:
                    result[section].append(valid)
        elif section == "validation":
            result["validation"].append(body)
        elif section == "acceptance":
            result["acceptance"].append(body)
    for key in ["allowed", "references", "validation", "acceptance"]:
        seen = set()
        values = []
        for value in result[key]:
            if value not in seen:
                seen.add(value)
                values.append(value)
        result[key] = values
    return result


def should_skip_dir(path):
    name = path.name
    if name in EXCLUDE_DIRS:
        return True
    try:
        r = rel(path)
    except Exception:
        return True
    if r.startswith(".ai-agent/") and not r.startswith(".ai-agent/ai-plan/"):
        return True
    if r.startswith(AGENT_GENERATED_PREFIX):
        return True
    if r.startswith(".code-agent-uninstall-archive/"):
        return True
    return False


def is_source_candidate(path):
    try:
        r = rel(path)
    except Exception:
        return False
    if r == "AGENTS.md":
        return False
    if r.startswith(".ai-agent/") or r.startswith(".agent/"):
        return False
    if r.startswith(AGENT_GENERATED_PREFIX):
        return False
    if is_runtime_artifact_rel(r):
        return False
    if path.name in SPECIAL_FILES:
        return True
    return path.suffix.lower() in TEXT_EXTENSIONS


def repo_files():
    files = []
    for dirpath, dirnames, filenames in os.walk(root):
        dpath = Path(dirpath)
        dirnames[:] = [d for d in dirnames if not should_skip_dir(dpath / d)]
        for name in filenames:
            path = dpath / name
            if is_source_candidate(path):
                files.append(path)
    return sorted(files, key=lambda p: rel(p))


def expand_patterns(patterns, files_by_rel):
    selected = set()
    for raw in patterns:
        value = raw.replace("\\", "/").lstrip("./")
        if not value or is_agent_generated_rel(value):
            continue
        if value in files_by_rel:
            selected.add(value)
            continue
        if "*" in value or "?" in value:
            try:
                matches = sorted(root.glob(value))
            except Exception:
                matches = []
            for match in matches[:40]:
                if match.is_file() and is_source_candidate(match):
                    selected.add(rel(match))
            continue
        p = resolve_path(value)
        if p and p.is_file() and is_source_candidate(p):
            selected.add(rel(p))
            continue
        if p and p.is_dir():
            count = 0
            for child in sorted(p.rglob("*")):
                if count >= 40:
                    break
                if child.is_file() and is_source_candidate(child):
                    selected.add(rel(child))
                    count += 1
    return selected


def git_changed(files_by_rel):
    changed = set()
    try:
        proc = subprocess.run(
            ["git", "-C", str(root), "status", "--porcelain=v1"],
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            check=False,
        )
    except Exception:
        return changed
    for line in proc.stdout.splitlines():
        if not line:
            continue
        name = line[3:].strip()
        if " -> " in name:
            name = name.split(" -> ", 1)[1]
        name = name.replace("\\", "/")
        if name in files_by_rel:
            changed.add(name)
    return changed


def reviewer_files(files_by_rel):
    out = set()
    path = runtime / "reviewer-files.txt"
    if not path.exists():
        return out
    for raw in safe_read(path).splitlines():
        value = raw.strip().replace("\\", "/").lstrip("./")
        if value in files_by_rel:
            out.add(value)
    return out


def resolve_local_import(base_rel, spec, files_by_rel):
    if not spec.startswith((".", "/")):
        return None
    base = root / base_rel
    candidate = root / spec.lstrip("/") if spec.startswith("/") else base.parent / spec
    suffixes = ["", ".ts", ".tsx", ".js", ".jsx", ".mjs", ".cjs", ".rs", ".css", ".json", ".html"]
    candidates = []
    for suffix in suffixes:
        candidates.append(Path(str(candidate) + suffix))
    for suffix in suffixes[1:]:
        candidates.append(candidate / ("index" + suffix))
    for item in candidates:
        try:
            r = rel(item)
        except Exception:
            continue
        if r in files_by_rel:
            return r
    return None


def direct_dependencies(file_rel, files_by_rel):
    path = files_by_rel.get(file_rel)
    if not path:
        return set()
    text = safe_read(path, 240000)
    deps = set()
    for pattern in [
        r"(?:import|from)\s+(?:[^'\"]+\s+from\s+)?['\"]([^'\"]+)['\"]",
        r"require\(\s*['\"]([^'\"]+)['\"]\s*\)",
        r"@import\s+['\"]([^'\"]+)['\"]",
        r"(?:src|href)=['\"]([^'\"]+)['\"]",
    ]:
        for spec in re.findall(pattern, text):
            found = resolve_local_import(file_rel, spec, files_by_rel)
            if found:
                deps.add(found)
    for mod in re.findall(r"^\s*(?:pub\s+)?mod\s+([A-Za-z_][A-Za-z0-9_]*)\s*;", text, flags=re.M):
        base = Path(file_rel).parent
        for candidate in [base / f"{mod}.rs", base / mod / "mod.rs"]:
            r = candidate.as_posix()
            if r in files_by_rel:
                deps.add(r)
    for crate_use in re.findall(r"\buse\s+crate::([A-Za-z_][A-Za-z0-9_:]*)", text):
        parts = [p for p in crate_use.split("::") if p]
        if parts:
            for candidate in [Path("src").joinpath(*parts).with_suffix(".rs"), Path("src").joinpath(*parts[:-1]).with_suffix(".rs")]:
                r = candidate.as_posix()
                if r in files_by_rel:
                    deps.add(r)
    return deps


def reverse_references(selected, files_by_rel):
    tokens = set()
    common = {"index", "main", "mod", "lib", "app", "page", "style", "styles"}
    for value in selected:
        p = Path(value)
        for token in {p.stem, p.name, value, value.rsplit(".", 1)[0]}:
            token = token.strip()
            if len(token) >= 4 and token not in common:
                tokens.add(token)
    refs = set()
    if not tokens:
        return refs
    for candidate, path in files_by_rel.items():
        if candidate in selected:
            continue
        text = safe_read(path, 160000)
        if any(token in text for token in tokens):
            refs.add(candidate)
            if len(refs) >= 30:
                break
    return refs


def traverse_codegraph(seed, files_by_rel, depth):
    selected = set(seed)
    frontier = set(seed)
    for _ in range(depth):
        next_frontier = set()
        for value in sorted(frontier):
            next_frontier.update(direct_dependencies(value, files_by_rel))
        next_frontier.update(reverse_references(frontier, files_by_rel))
        next_frontier.difference_update(selected)
        if not next_frontier:
            break
        selected.update(next_frontier)
        frontier = next_frontier
        if len(selected) >= 80:
            return set(sorted(selected)[:80])
    return selected


def extract_heading_sections(path, wanted, max_lines=420):
    if not path.exists() or not wanted:
        return ""
    lines = safe_read(path).splitlines()
    out = []
    capture = False
    for line in lines:
        heading = re.match(r"^(#{2,3})\s+(.+?)\s*$", line)
        if heading:
            capture = heading.group(2).strip() in wanted
        if capture:
            out.append(line)
            if len(out) >= max_lines:
                out.append("...[truncated]")
                break
    return "\n".join(out).strip()


def source_snippet(file_rel, max_lines, files_by_rel):
    path = files_by_rel.get(file_rel)
    if not path:
        return ""
    text = safe_read(path)
    if not text:
        return ""
    lines = text.splitlines()
    shown = lines[:max_lines]
    numbered = [f"{idx + 1}: {line}" for idx, line in enumerate(shown)]
    if len(lines) > max_lines:
        numbered.append("...[truncated; open the targeted file for omitted lines if needed]")
    return "\n".join(numbered)


def target_file_hints(task_type):
    by_type = {
        "frontend": ["public/index.html", "public/css/index.css", "public/js/index.js"],
        "admin": ["public/admin/index.html", "public/admin.css", "public/admin.js"],
        "backend": ["src/lib.rs", "src/main.rs", "Cargo.toml"],
        "database": ["schema.sql", "migrations/"],
        "agent": [".ai-agent/scripts/", ".ai-agent/prompts/", ".ai-agent/AGENTS.md"],
    }
    return by_type.get(task_type, ["Cargo.toml", "package.json", "wrangler.toml", "schema.sql", "src/lib.rs", "public/index.html"])


def detect_failure_signals():
    signals = []
    if not repair_mode:
        return signals
    if reviewer_summary.exists():
        signals.append("repair mode requested reviewer context")
    for path, label in [
        (runtime / "reviewer-summary.md", "reviewer failure summary"),
        (runtime / "final-verdict.txt", "final review verdict"),
        (runtime / "runtime-context.md", "runtime validation context"),
    ]:
        text = safe_read(path, 12000).lower()
        if not text:
            continue
        if label == "reviewer failure summary" and "fail" in text and escalate_on_review_fail:
            signals.append(label)
        elif label == "final review verdict" and "fail" in text and escalate_on_review_fail:
            signals.append(label)
        elif label == "runtime validation context" and re.search(r"\b(fail|failed|error|panic|missing module|undefined|not found|no such file)\b", text) and allow_context_escalation_on_failure and escalate_on_build_fail:
            signals.append(label)
    return signals


def select_knowledge(task_text, requirement_text, selected_files):
    docs = {
        "architecture": knowledge / "architecture.md",
        "api": knowledge / "api.md",
        "database": knowledge / "database.md",
        "frontend": knowledge / "frontend.md",
        "documentation": knowledge / "documentation.md",
    }
    if not lazy_knowledge:
        return [name for name, path in docs.items() if path.exists()]
    blob = (task_text + "\n" + requirement_text + "\n" + "\n".join(selected_files)).lower()
    chosen = set()
    if re.search(r"\b(auth|permission|role|workflow|architecture|integration|cross-cutting)\b", blob):
        chosen.add("architecture")
    if re.search(r"\b(api|route|router|endpoint|fetch|request|response|worker|handler|webhook)\b", blob):
        chosen.add("api")
    if re.search(r"\b(database|schema|migration|sql|table|column|index|d1|sqlite)\b", blob):
        chosen.add("database")
    if re.search(r"\b(booking|payment|checkout|reservation|calendar|availability|guest)\b", blob):
        chosen.add("frontend")
    if re.search(r"\b(directions|map|location|route guidance)\b", blob):
        chosen.add("documentation")
    if re.search(r"\b(customer ai|assistant|ai answer|bot|prompt|skill)\b", blob):
        chosen.add("architecture")
    return [name for name in ["architecture", "api", "database", "frontend", "documentation"] if name in chosen and docs[name].exists()]


task = current_task_path()
task_info = parse_task(task)
requirement_path = root / ".agent" / "requirement.md"
requirement_text = safe_read(requirement_path) if requirement_path.exists() else ""
task_size = safe_read(runtime / "task-size.txt").strip().upper() or "MEDIUM"
task_type = safe_read(runtime / "task-type.txt").strip().lower() or "general"
planning_isolation = safe_read(runtime / "planning-isolation.txt").strip().lower() or "same-or-new"
inferred_allowed = [line.strip() for line in safe_read(runtime / "inferred-allowed-area.txt").splitlines() if line.strip()]
runtime_context = runtime / "runtime-context.md"
reviewer_summary = runtime / "reviewer-summary.md"
reviewer_scope = runtime / "reviewer-scope.txt"
reviewer_files_path = runtime / "reviewer-files.txt"
reviewer_diff = runtime / "reviewer-diff.patch"
final_verdict = runtime / "final-verdict.txt"

files = repo_files()
files_by_rel = {rel(path): path for path in files}
changed_files = git_changed(files_by_rel)

size_defaults = {
    "SMALL": {"max_level": env_int("SMALL_MAX_CONTEXT_LEVEL", 1, 0), "target_tokens": env_int("SMALL_CONTEXT_TARGET_TOKENS", 25000, 4000), "hard_cap_tokens": env_int("SMALL_CONTEXT_HARD_CAP_TOKENS", 40000, 4000)},
    "MEDIUM": {"max_level": env_int("MEDIUM_MAX_CONTEXT_LEVEL", 2, 0), "target_tokens": env_int("MEDIUM_CONTEXT_TARGET_TOKENS", 50000, 4000), "hard_cap_tokens": env_int("MEDIUM_CONTEXT_HARD_CAP_TOKENS", 80000, 4000)},
    "LARGE": {"max_level": env_int("LARGE_MAX_CONTEXT_LEVEL", 3, 0), "target_tokens": env_int("LARGE_CONTEXT_TARGET_TOKENS", 120000, 4000), "hard_cap_tokens": env_int("LARGE_CONTEXT_HARD_CAP_TOKENS", 160000, 4000)},
}.get(task_size, {"max_level": 2, "target_tokens": configured_max_context_tokens, "hard_cap_tokens": max(configured_max_context_tokens, 80000)})
size_default_max_level = min(size_defaults["max_level"], 5)
target_context_tokens = size_defaults["target_tokens"]
hard_cap_tokens = min(size_defaults["hard_cap_tokens"], max(size_defaults["hard_cap_tokens"], configured_max_context_tokens))
max_context_tokens = min(configured_max_context_tokens, hard_cap_tokens)

base_target_patterns = []
base_target_patterns.extend(task_info["allowed"])
if not task_info["allowed"] or task_size != "SMALL":
    base_target_patterns.extend(inferred_allowed)
if role in {"reviewer", "reviewer-final"}:
    base_target_patterns.extend(task_info["references"])
target_files = expand_patterns(base_target_patterns, files_by_rel)
reference_files = expand_patterns(task_info["references"], files_by_rel)
review_files = reviewer_files(files_by_rel)
if role in {"reviewer", "reviewer-final"}:
    target_files.update(review_files)
if task_size == "SMALL" and inferred_allowed:
    inferred_selected = expand_patterns(inferred_allowed, files_by_rel)
    target_files.update(f for f in changed_files if f in inferred_selected)
else:
    target_files.update(f for f in changed_files if f in reference_files or f in target_files)

failure_signals = detect_failure_signals()
focused_hints = [hint for hint in target_file_hints(task_type) if hint]
focused_target_files = set(target_files)
if not focused_target_files:
    focused_target_files.update(expand_patterns(focused_hints, files_by_rel))
focused_target_files.update(reference_files)

if context_default_level.isdigit():
    requested_level = max(0, min(5, int(context_default_level)))
else:
    if role == "planner":
        requested_level = 0
    elif role in {"reviewer", "reviewer-final"}:
        requested_level = 1
    elif task_size == "SMALL":
        requested_level = 1 if target_files else 0
    elif task_size == "MEDIUM":
        requested_level = 2 if target_files else 1
    else:
        requested_level = 3 if target_files else 1

normal_max_level = size_default_max_level if context_escalation else max(size_default_max_level, 3)
selected_level = min(requested_level, normal_max_level)
escalation_reason = "direct target files are sufficient" if target_files else "minimal context is sufficient"
escalated = selected_level > 0
search_budget_used = 0

if not context_escalation:
    selected_level = max(selected_level, min(3, max(size_default_max_level, 1)))
    escalation_reason = "CONTEXT_ESCALATION=false; fallback to previous broader packaging"
elif not target_files and focused_target_files:
    fallback_level = 4 if task_size == "SMALL" else 4
    selected_level = max(selected_level, min(fallback_level, 4))
    escalation_reason = "target file could not be identified from task scope; used focused project hints"
    search_budget_used = min(search_budget, 1)
elif failure_signals:
    fail_level = 4 if task_size in {"SMALL", "MEDIUM"} else 4
    selected_level = max(selected_level, fail_level)
    escalation_reason = "failure signals requested more context: " + ", ".join(sorted(set(failure_signals)))
    search_budget_used = min(search_budget, 2)

if context_escalation and not repair_mode and target_files:
    selected_level = min(selected_level, normal_max_level)

selected_level = max(0, min(5, selected_level))

target_files = focused_target_files if selected_level >= 1 else set()
if role == "planner" and selected_level == 0:
    target_files = set()

codegraph_depth_used = 0
direct_files = set(target_files)
if selected_level >= 2 and target_files:
    codegraph_depth_used = min(configured_codegraph_depth, 1 if selected_level == 2 else configured_codegraph_depth)
    direct_files = traverse_codegraph(target_files, files_by_rel, codegraph_depth_used)
    if direct_files != target_files and escalation_reason == "direct target files are sufficient":
        escalation_reason = "direct dependencies or reverse references were required"

knowledge_names = []
if selected_level >= 3:
    knowledge_names = select_knowledge(task_info["text"], requirement_text, sorted(direct_files))
    if knowledge_names and "required" not in escalation_reason and "dependencies" not in escalation_reason:
        escalation_reason = "task required focused domain knowledge"

relevant_files = sorted(direct_files if selected_level >= 2 else target_files)
if selected_level >= 4 and not relevant_files:
    relevant_files = sorted(focused_target_files)
if selected_level >= 5 and task_size == "LARGE":
    broad_seed = set(focused_target_files) | set(reference_files) | set(changed_files)
    relevant_files = sorted(traverse_codegraph(broad_seed, files_by_rel, min(configured_codegraph_depth, 3)))
    search_budget_used = min(search_budget, max(search_budget_used, 3))
    escalation_reason = "broad search enabled for large cross-module context"

minimal_runtime_files = []
runtime_is_concise = runtime_context.exists() and len(safe_read(runtime_context, 8000).splitlines()) <= 80
if runtime_is_concise:
    minimal_runtime_files.append(".ai-agent/generated/runtime/runtime-context.md")
if repair_mode and reviewer_summary.exists():
    minimal_runtime_files.append(".ai-agent/generated/runtime/reviewer-summary.md")

search_allowlist = []
if requirement_path.exists():
    search_allowlist.append(".agent/requirement.md")
if task:
    search_allowlist.append(rel(task))
search_allowlist.extend([
    ".ai-agent/generated/runtime/task-size.txt",
    ".ai-agent/generated/runtime/task-type.txt",
])
search_allowlist.extend(minimal_runtime_files)
if selected_level >= 1:
    search_allowlist.extend(sorted(target_files))
if selected_level >= 2:
    search_allowlist.extend(sorted(relevant_files))
if role in {"reviewer", "reviewer-final"} or repair_mode:
    for value in [
        ".ai-agent/generated/runtime/reviewer-summary.md",
        ".ai-agent/generated/runtime/reviewer-files.txt",
        ".ai-agent/generated/runtime/reviewer-scope.txt",
        ".ai-agent/generated/runtime/reviewer-diff.patch",
    ]:
        search_allowlist.append(value)
seen = set()
search_allowlist = [x for x in search_allowlist if x and not (x in seen or seen.add(x))]

excluded_items = [
    ".ai-agent/generated/runtime/*.log",
    ".ai-agent/generated/runtime/coder-round-*.log",
    ".ai-agent/generated/runtime/reviewer-round-*.log",
    ".ai-agent/generated/runtime/token-usage*",
    ".ai-agent/generated/runtime/*.jsonl",
    ".ai-agent/generated/cache/**",
    ".ai-agent/generated/tmp/**",
    ".ai-agent/generated/history/**",
    ".ai-agent/generated/token*/**",
]


def build_sections(src_lines, knowledge_lines, codegraph_lines, diff_lines, include_sources=True, include_knowledge=True, include_codegraph=True):
    sections = []
    metadata = {
        "role": role,
        "generated": generated_at(),
        "task_size": task_size,
        "task_type": task_type,
        "context_level": selected_level,
        "context_level_name": CONTEXT_LEVEL_LABELS[selected_level],
        "planning_isolation": planning_isolation,
        "context_mode": context_mode,
        "context_escalation": context_escalation,
        "token_guard": token_guard,
        "target_context_tokens": target_context_tokens,
        "max_context_tokens": max_context_tokens,
        "search_budget": search_budget,
        "search_budget_used": search_budget_used,
        "codegraph_depth": codegraph_depth_used,
        "lazy_knowledge": lazy_knowledge,
        "compact_review": compact_review,
        "repair_mode": repair_mode,
        "allow_runtime_log_read": allow_runtime_log_read,
        "allow_history_search": allow_history_search,
        "max_tasks_from_planner": os.environ.get("MAX_TASKS_FROM_PLANNER", ""),
    }
    sections.append(("metadata", "# Compact Context Package\n\n" + "\n".join(f"- {k}: {v}" for k, v in metadata.items()) + "\n"))
    sections.append(("policy", "\n".join([
        "## Runtime Artifact Access Policy",
        "",
        f"- Use the current context level only: Level {selected_level} ({CONTEXT_LEVEL_LABELS[selected_level]}).",
        "- Do not search outside the allowlist unless the script explicitly escalated context or a validation/reviewer failure requires it.",
        "- Do not automatically read or search runtime logs, token files, jsonl streams, generated cache, tmp, history, or token archives.",
        f"- Runtime log reads allowed: {'yes' if allow_runtime_log_read else 'no'}; when allowed, prefer reviewer-summary.md before raw logs.",
        f"- History search allowed: {'yes' if allow_history_search else 'no'}.",
        "- Blocked normal-mode runtime patterns:",
        compact_lines(RUNTIME_BLOCKED_PATTERNS),
        "- Ignored automatic-search roots:",
        compact_lines(AUTO_SEARCH_IGNORES),
        f"- Search budget: {search_budget} unique search operations; used by this package: {search_budget_used}.",
    ]) + "\n"))
    sections.append(("priority", "\n".join([
        "## File Reading Priority",
        "",
        "1. Current task",
        "2. Allowed edit area",
        "3. Runtime context",
        "4. reviewer-summary.md",
        "5. Codegraph",
        "6. Relevant source files",
        "",
        "Only read documentation, README files, historical logs, or archived context when the current task explicitly requires them.",
    ]) + "\n"))
    sections.append(("allowlist", "## Search Allowlist\n\n" + compact_lines(search_allowlist) + "\n"))
    if inferred_allowed:
        sections.append(("inferred", "## Inferred Allowed Edit Area\n\n" + compact_lines(inferred_allowed) + "\n"))
    if role == "planner" and planning_isolation == "changed":
        sections.append(("plan-isolation", "## Old Plan Isolation\n\nThe current requirement changed from the previous planning run. Treat old plan/task files as historical context only. Do not deeply read or reuse old tasks unless the new requirement explicitly says to continue the old plan.\n"))
    if task:
        sections.append(("task", f"## Current Task\n\nPath: `{rel(task)}`\n\n```md\n{first_lines(task, 260)}```\n"))
    if requirement_text:
        req_lines = "\n".join(requirement_text.splitlines()[:140])
        if len(requirement_text.splitlines()) > 140:
            req_lines += "\n...[truncated]"
        sections.append(("requirement", f"## Requirement\n\n```md\n{req_lines}\n```\n"))
    sections.append(("allowed", "\n".join([
        "## Allowed Edit Area",
        "",
        compact_lines(task_info["allowed"]),
        "",
        "## Task Reference Files",
        "",
        compact_lines(task_info["references"]),
        "",
        "## Acceptance Criteria",
        "",
        compact_lines(task_info["acceptance"]),
        "",
        "## Validation Commands",
        "",
        compact_lines(task_info["validation"]),
    ]) + "\n"))
    runtime_excerpt = first_lines(runtime_context, runtime_max_lines) if runtime_context.exists() else ""
    if runtime_excerpt and (selected_level >= 1 or runtime_is_concise):
        sections.append(("runtime", f"## Runtime Context Excerpt\n\nPath: `.ai-agent/generated/runtime/runtime-context.md`\n\n```md\n{runtime_excerpt}```\n"))
    if compact_review and (repair_mode or role in {"reviewer", "reviewer-final", "reviewer-final-fix"}) and reviewer_summary.exists():
        sections.append(("review-summary", f"## Reviewer Summary\n\nPath: `.ai-agent/generated/runtime/reviewer-summary.md`\n\n```md\n{first_lines(reviewer_summary, 180)}```\n"))
    if role in {"reviewer", "reviewer-final"} or repair_mode:
        if reviewer_scope.exists():
            sections.append(("review-scope", f"## Reviewer Scope\n\n```text\n{first_lines(reviewer_scope, 180)}```\n"))
        if reviewer_files_path.exists():
            sections.append(("review-files", f"## Reviewer Files\n\n```text\n{first_lines(reviewer_files_path, 180)}```\n"))
        if reviewer_diff.exists():
            sections.append(("review-diff", f"## Reviewer Diff Excerpt\n\nPath: `.ai-agent/generated/runtime/reviewer-diff.patch`\n\n```diff\n{first_lines(reviewer_diff, diff_lines)}```\n"))
    if role in {"planner-final-fix", "reviewer-final-fix", "reviewer-final"} and final_verdict.exists():
        sections.append(("final-verdict", f"## Final Reviewer Verdict\n\n```text\n{first_lines(final_verdict, 180)}```\n"))
    if include_codegraph and selected_level >= 2 and relevant_files:
        codegraph_parts = []
        wanted = set(relevant_files)
        for graph in [cache / "codegraph-lite.md", cache / "codegraph-project.md", cache / "symbol-index.md", cache / "api-index.md", cache / "schema-index.md", cache / "frontend-index.md", cache / "dependency-index.md"]:
            excerpt = extract_heading_sections(graph, wanted, codegraph_lines)
            if excerpt:
                codegraph_parts.append(f"### {graph.relative_to(ai_dir).as_posix()}\n\n{excerpt}")
        if codegraph_parts:
            sections.append(("codegraph", "## Relevant Codegraph\n\n" + "\n\n".join(codegraph_parts) + "\n"))
    if include_knowledge and selected_level >= 3 and knowledge_names:
        knowledge_parts = []
        for name in knowledge_names:
            path = knowledge / f"{name}.md"
            excerpt = first_lines(path, knowledge_lines)
            if excerpt:
                knowledge_parts.append(f"### {name}.md\n\n```md\n{excerpt}```")
        if knowledge_parts:
            sections.append(("knowledge", "## Relevant Knowledge\n\n" + "\n\n".join(knowledge_parts) + "\n"))
    if include_sources and selected_level >= 1:
        snippet_parts = []
        snippet_files = relevant_files if relevant_files else sorted(target_files)
        for file_rel in snippet_files[:40]:
            snippet = source_snippet(file_rel, src_lines, files_by_rel)
            if snippet:
                snippet_parts.append(f"### {file_rel}\n\n```text\n{snippet}\n```")
        if snippet_parts:
            sections.append(("sources", "## Relevant Source Snippets\n\n" + "\n\n".join(snippet_parts) + "\n"))
    return sections


def render(sections):
    return "\n".join(text.rstrip() for _, text in sections if text.strip()).rstrip() + "\n"


src_lines = source_max_lines
knowledge_lines = knowledge_max_lines
codegraph_lines = 420
diff_lines = review_diff_max_lines
include_sources = True
include_knowledge = True
include_codegraph = True
sections = build_sections(src_lines, knowledge_lines, codegraph_lines, diff_lines, include_sources, include_knowledge, include_codegraph)
package = render(sections)
trim_steps = []
if token_guard:
    while estimate_tokens(package) > max_context_tokens:
        before = estimate_tokens(package)
        if knowledge_lines > 80:
            knowledge_lines = 80
            trim_steps.append("trimmed knowledge excerpts to 80 lines")
        elif codegraph_lines > 160:
            codegraph_lines = 160
            trim_steps.append("trimmed codegraph excerpts to 160 lines")
        elif src_lines > 80:
            src_lines = 80
            trim_steps.append("trimmed source snippets to 80 lines")
        elif diff_lines > 260:
            diff_lines = 260
            trim_steps.append("trimmed reviewer diff excerpt to 260 lines")
        elif include_sources:
            include_sources = False
            trim_steps.append("removed source snippets; targeted files remain in allowlist")
        elif include_knowledge and selected_level >= 3:
            include_knowledge = False
            trim_steps.append("removed knowledge excerpts; knowledge files remain listed in manifest")
        elif include_codegraph and selected_level >= 2:
            include_codegraph = False
            trim_steps.append("removed codegraph excerpts; relevant files remain in allowlist")
        elif runtime_max_lines > 60:
            runtime_max_lines = 60
            trim_steps.append("trimmed runtime context excerpt to 60 lines")
        else:
            trim_steps.append(f"context remains above budget after required sections; estimate was {before}")
            break
        sections = build_sections(src_lines, knowledge_lines, codegraph_lines, diff_lines, include_sources, include_knowledge, include_codegraph)
        package = render(sections)

if trim_steps:
    package = package.replace("## Runtime Artifact Access Policy", "## Token Guard Trimming\n\n" + compact_lines(trim_steps) + "\n\n## Runtime Artifact Access Policy", 1)

estimated_tokens = estimate_tokens(package)
included_files = [".agent/requirement.md"] if requirement_path.exists() else []
if task:
    included_files.append(rel(task))
included_files.extend(minimal_runtime_files)
included_files.extend(sorted(target_files))
if selected_level >= 2:
    included_files.extend(sorted(relevant_files))
included_files.extend([f".ai-agent/generated/knowledge/{name}.md" for name in knowledge_names])
seen = set()
included_files = [x for x in included_files if x and not (x in seen or seen.add(x))]

out_path.write_text(package, encoding="utf-8")
allowlist_path.write_text("\n".join(search_allowlist).rstrip() + "\n", encoding="utf-8")

manifest_text = "\n".join([
    f"TASK_SIZE={task_size}",
    f"TASK_TYPE={task_type}",
    f"ROLE={role}",
    f"CONTEXT_LEVEL={selected_level}",
    f"CONTEXT_LEVEL_NAME={CONTEXT_LEVEL_LABELS[selected_level]}",
    f"ESCALATED={'true' if selected_level > requested_level or search_budget_used > 0 or bool(failure_signals) else 'false'}",
    f"ESCALATION_REASON={escalation_reason}",
    f"ESTIMATED_TOKENS={estimated_tokens}",
    f"TARGET_CONTEXT_TOKENS={target_context_tokens}",
    f"MAX_CONTEXT_TOKENS={max_context_tokens}",
    f"SEARCH_BUDGET={search_budget}",
    f"SEARCH_BUDGET_USED={search_budget_used}",
    f"CODEGRAPH_DEPTH_USED={codegraph_depth_used}",
    "KNOWLEDGE_FILES_LOADED=" + (",".join(knowledge_names) if knowledge_names else "(none)"),
    "INCLUDED_FILES:",
    compact_lines(included_files),
    "EXCLUDED_FILES:",
    compact_lines(excluded_items),
]) + "\n"
if context_manifest_enabled:
    manifest_path.write_text(manifest_text, encoding="utf-8")

escalation_text = "\n".join([
    f"TASK_SIZE={task_size}",
    f"TASK_TYPE={task_type}",
    f"CONTEXT_LEVEL={selected_level}",
    f"ESCALATED={'true' if selected_level > requested_level or search_budget_used > 0 or bool(failure_signals) else 'false'}",
    f"ESCALATION_REASON={escalation_reason}",
    f"MAX_CONTEXT_TOKENS={max_context_tokens}",
    f"KNOWLEDGE_LOADED={'true' if bool(knowledge_names) else 'false'}",
    f"CODEGRAPH_DEPTH={codegraph_depth_used}",
    f"PROJECT_SEARCH={'true' if selected_level >= 4 else 'false'}",
]) + "\n"
escalation_path.write_text(escalation_text, encoding="utf-8")

meta = {
    "generated_at": generated_at(),
    "role": role,
    "task": rel(task) if task else "",
    "task_size": task_size,
    "task_type": task_type,
    "planning_isolation": planning_isolation,
    "inferred_allowed_area": inferred_allowed,
    "context_escalation": context_escalation,
    "context_level": selected_level,
    "context_level_name": CONTEXT_LEVEL_LABELS[selected_level],
    "escalation_reason": escalation_reason,
    "target_context_tokens": target_context_tokens,
    "estimated_tokens": estimated_tokens,
    "max_context_tokens": max_context_tokens,
    "token_guard": token_guard,
    "search_budget": search_budget,
    "search_budget_used": search_budget_used,
    "codegraph_depth": codegraph_depth_used,
    "lazy_knowledge": lazy_knowledge,
    "compact_review": compact_review,
    "repair_mode": repair_mode,
    "allow_runtime_log_read": allow_runtime_log_read,
    "allow_history_search": allow_history_search,
    "relevant_files": relevant_files,
    "knowledge": knowledge_names,
    "trim_steps": trim_steps,
    "included_files": included_files,
    "excluded_patterns": excluded_items,
}
meta_path.write_text(json.dumps(meta, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
print(out_path)
PY
