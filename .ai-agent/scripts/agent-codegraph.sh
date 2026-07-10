#!/usr/bin/env bash
set -euo pipefail

SELF="${BASH_SOURCE[0]}"
AI_DIR="${AI_DIR:-$(cd "$(dirname "$SELF")/.." && pwd)}"
PROJECT_ROOT="${PROJECT_ROOT:-$(cd "$AI_DIR/.." && pwd)}"
CMD="${1:-status}"

if [[ -f "$AI_DIR/scripts/agent-load-env.sh" ]]; then
  # shellcheck disable=SC1090
  source "$AI_DIR/scripts/agent-load-env.sh"
  load_agent_env "$AI_DIR"
fi

python3 - "$PROJECT_ROOT" "$AI_DIR" "$CMD" "${CODEGRAPH_MODE:-smart}" "${CURRENT_TASK:-}" <<'PY'
import datetime
import hashlib
import json
import os
import re
import sys
from pathlib import Path

root = Path(sys.argv[1]).resolve()
ai_dir = Path(sys.argv[2]).resolve()
cmd = sys.argv[3]
mode = sys.argv[4] or "smart"
current_task_arg = sys.argv[5] if len(sys.argv) > 5 else ""

cache_dir = ai_dir / "generated" / "cache"
runtime_dir = ai_dir / "generated" / "runtime"
state_path = cache_dir / "codegraph.state"

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
EXCLUDE_PREFIXES = (
    ".ai-agent/generated/",
    ".ai-agent/runtime/",
    ".ai-agent/cache/",
    ".ai-agent/logs/",
    ".ai-agent/tmp/",
    ".ai-agent/state/",
    ".ai-agent/backups/",
    ".code-agent-uninstall-archive/",
)
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
}
SPECIAL_FILES = {
    "Cargo.toml",
    "package.json",
    "wrangler.toml",
    "schema.sql",
    "README.md",
    ".agent/requirement.md",
}

GENERATED_FILES = {
    "codegraph-project.md",
    "codegraph-lite.md",
    "symbol-index.md",
    "api-index.md",
    "schema-index.md",
    "frontend-index.md",
    "dependency-index.md",
    "docs-index.md",
    "project-summary.json",
    "file-fingerprints.tsv",
}

try:
    AGENT_GENERATED_PREFIX = (ai_dir / "generated").resolve().relative_to(root).as_posix() + "/"
except Exception:
    AGENT_GENERATED_PREFIX = ".ai-agent/generated/"

def rel(path: Path) -> str:
    return path.resolve().relative_to(root).as_posix()

def should_skip_dir(path: Path) -> bool:
    name = path.name
    if name in EXCLUDE_DIRS:
        return True
    try:
        r = rel(path)
    except Exception:
        return True
    if r.startswith(".ai-agent/") and not r.startswith(".ai-agent/ai-plan/"):
        return True
    return (r + "/").startswith(AGENT_GENERATED_PREFIX) or any((r + "/").startswith(prefix) for prefix in EXCLUDE_PREFIXES)

def is_candidate(path: Path) -> bool:
    try:
        r = rel(path)
    except Exception as exc:
        warn(f"skip path outside project or invalid path {path!s}: {exc}")
        return False
    if any(r.startswith(prefix) for prefix in EXCLUDE_PREFIXES):
        return False
    if r.startswith(AGENT_GENERATED_PREFIX):
        return False
    if r == "AGENTS.md":
        return False
    if r.startswith(".ai-agent/") and not r.startswith(".ai-agent/ai-plan/"):
        return False
    if path.name in SPECIAL_FILES:
        return True
    if path.suffix.lower() not in TEXT_EXTENSIONS:
        return False
    try:
        if path.stat().st_size > int(os.environ.get("CODEGRAPH_MAX_FILE_BYTES", "1200000")):
            return False
    except (OSError, ValueError, UnicodeError) as exc:
        warn(f"skip path with unreadable stat {path!s}: {exc}")
        return False
    return True

def iter_files():
    files = []
    for dirpath, dirnames, filenames in os.walk(root):
        dpath = Path(dirpath)
        dirnames[:] = [d for d in dirnames if not should_skip_dir(dpath / d)]
        for name in filenames:
            path = dpath / name
            if is_candidate(path):
                files.append(path)
    return sorted(files, key=lambda p: rel(p))

def read_text(path: Path) -> str:
    try:
        return path.read_text(encoding="utf-8", errors="replace")
    except (OSError, ValueError, UnicodeError) as exc:
        warn(f"skip unreadable file {path}: {exc}")
        return ""

def warn(message: str):
    try:
        runtime_dir.mkdir(parents=True, exist_ok=True)
        with (runtime_dir / "codegraph-warnings.log").open("a", encoding="utf-8") as f:
            f.write(f"{generated_at()} [CodeGraph] {message}\n")
    except Exception:
        pass

def safe_exists(path: Path) -> bool:
    try:
        return path.exists()
    except (OSError, ValueError, UnicodeError) as exc:
        warn(f"invalid path skipped during exists check: {path!s} ({exc})")
        return False

def safe_is_file(path: Path) -> bool:
    try:
        return path.is_file()
    except (OSError, ValueError, UnicodeError) as exc:
        warn(f"invalid path skipped during file check: {path!s} ({exc})")
        return False

def safe_resolve_under_root(value: str):
    valid = valid_repo_reference(value)
    if not valid:
        return None
    try:
        path = root / valid
        resolved = path.resolve()
        resolved.relative_to(root)
        return resolved
    except (OSError, ValueError, UnicodeError) as exc:
        warn(f"invalid reference path skipped: {value!r} ({exc})")
        return None

def valid_repo_reference(raw: str):
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
    if not any(ch in value for ch in ["/", "."]):
        return None
    parts = [part for part in value.split("/") if part]
    if any(part == ".." for part in parts):
        return None
    return value.lstrip("./")

def fingerprint(files):
    h = hashlib.sha256()
    rows = []
    for path in files:
        data = path.read_bytes()
        digest = hashlib.sha256(data).hexdigest()
        r = rel(path)
        rows.append((r, digest, len(data)))
        h.update(r.encode())
        h.update(b"\0")
        h.update(digest.encode())
        h.update(b"\0")
    return h.hexdigest(), rows

def generated_at():
    return datetime.datetime.now(datetime.timezone.utc).astimezone().isoformat(timespec="seconds")

def load_state():
    data = {}
    if state_path.exists():
        for line in state_path.read_text(encoding="utf-8", errors="replace").splitlines():
            if "=" in line:
                k, v = line.split("=", 1)
                data[k.strip()] = v.strip()
    return data

def write_state(fp):
    state_path.write_text(
        "\n".join(
            [
                f"fingerprint={fp}",
                f"generated_at={generated_at()}",
                f"mode={mode}",
            ]
        )
        + "\n",
        encoding="utf-8",
    )

SYMBOL_PATTERNS = [
    re.compile(r"^\s*(?:pub\s+)?(?:async\s+)?fn\s+([A-Za-z_][A-Za-z0-9_]*)"),
    re.compile(r"^\s*(?:pub\s+)?(?:struct|enum|trait|impl|mod)\s+([A-Za-z_][A-Za-z0-9_]*)?"),
    re.compile(r"^\s*(?:export\s+)?(?:async\s+)?function\s+([A-Za-z_$][A-Za-z0-9_$]*)"),
    re.compile(r"^\s*(?:export\s+)?(?:const|let|var)\s+([A-Za-z_$][A-Za-z0-9_$]*)\s*=\s*(?:async\s*)?(?:\([^)]*\)|[A-Za-z_$][A-Za-z0-9_$]*)\s*=>"),
    re.compile(r"^\s*(?:export\s+)?class\s+([A-Za-z_$][A-Za-z0-9_$]*)"),
    re.compile(r"^\s*CREATE\s+(?:TABLE|INDEX|VIEW|TRIGGER)\b", re.I),
    re.compile(r"^\s*ALTER\s+TABLE\b", re.I),
]
API_RE = re.compile(r"(/api/[A-Za-z0-9_./:{}?-]+|fetch\s*\(|router|route|endpoint|METHOD|GET|POST|PUT|PATCH|DELETE)", re.I)
SCHEMA_RE = re.compile(r"\b(CREATE\s+TABLE|ALTER\s+TABLE|CREATE\s+INDEX|REFERENCES|FOREIGN\s+KEY|PRIMARY\s+KEY)\b", re.I)
FRONTEND_RE = re.compile(r"(querySelector|addEventListener|render|localStorage|fetch\s*\(|#[A-Za-z0-9_-]+|\.[A-Za-z0-9_-]+\s*\{)")
DEPENDENCY_RE = re.compile(r"^\s*(use\s+|import\s+|from\s+['\"]|dependencies|dev-dependencies|\[[A-Za-z0-9_.-]*dependencies|\[package\]|\[lib\]|\[bin\])")

def compact(line: str, limit=220) -> str:
    line = line.strip()
    return line if len(line) <= limit else line[: limit - 3] + "..."

def markers_for(path: Path, kind: str):
    try:
        lines = read_text(path).splitlines()
    except OSError:
        return []
    out = []
    r = rel(path)
    for idx, line in enumerate(lines, 1):
        text = line.rstrip()
        hit = False
        if kind == "symbol":
            hit = any(p.search(text) for p in SYMBOL_PATTERNS)
        elif kind == "api":
            hit = bool(API_RE.search(text))
        elif kind == "schema":
            hit = bool(SCHEMA_RE.search(text))
        elif kind == "frontend":
            hit = r.startswith("public/") and bool(FRONTEND_RE.search(text))
        elif kind == "dependency":
            hit = bool(DEPENDENCY_RE.search(text))
        elif kind == "docs":
            hit = path.suffix.lower() == ".md" and text.lstrip().startswith("#")
        if hit:
            out.append(f"{idx}:{compact(text)}")
    return out

def write_index(name: str, title: str, grouped):
    lines = [f"# {title}", "", f"Generated: {generated_at()}", ""]
    for file_name, entries in grouped:
        if not entries:
            continue
        lines.append(f"## {file_name}")
        lines.extend(entries)
        lines.append("")
    (cache_dir / name).write_text("\n".join(lines).rstrip() + "\n", encoding="utf-8")

def group_markers(files, kind: str, max_entries_per_file: int):
    grouped = []
    for path in files:
        entries = markers_for(path, kind)
        if entries:
            grouped.append((rel(path), entries[:max_entries_per_file]))
    return grouped

def task_reference_files(files):
    selected = set()
    task_path = None
    try:
        task_path = Path(current_task_arg) if current_task_arg else None
        if task_path and not task_path.is_absolute():
            task_path = root / task_path
    except (OSError, ValueError, UnicodeError):
        task_path = None
    candidates = []
    if task_path and safe_exists(task_path):
        candidates.append(task_path)
    status_path = ai_dir / "generated" / "status.json"
    if safe_exists(status_path):
        try:
            task_value = json.loads(status_path.read_text(encoding="utf-8")).get("task")
            if task_value:
                p = Path(task_value)
                candidate = p if p.is_absolute() else root / p
                if safe_exists(candidate):
                    candidates.append(candidate)
        except (OSError, ValueError, UnicodeError, json.JSONDecodeError) as exc:
            warn(f"status task reference skipped: {exc}")

    reference_headers = {
        "reference files",
        "reference map",
        "allowed files",
        "allowed scope",
        "scope",
        "target files",
        "files to edit",
        "implementation files",
        "required files",
    }

    for task in candidates:
        if not safe_exists(task):
            continue
        in_section = False
        in_fence = False
        for raw in read_text(task).splitlines():
            if raw.lstrip().startswith("```"):
                in_fence = not in_fence
                continue
            if in_fence:
                continue
            header = re.match(r"^#{1,6}\s+(.+?)\s*$", raw)
            if header:
                in_section = header.group(1).strip().lower() in reference_headers
                continue
            if not in_section:
                continue
            stripped = raw.strip()
            if not stripped or stripped.startswith(">"):
                continue
            bullet = re.match(r"^\s*(?:[-*+]|\d+[.)])\s+(?:\[[ xX]\]\s*)?(.+?)\s*$", raw)
            if not bullet:
                continue
            body = bullet.group(1).strip()
            tokens = re.findall(r"`([^`\n\r]+)`", body) or [body.split()[0] if body.split() else ""]
            for value in tokens:
                if value.startswith(".ai-agent/generated/"):
                    continue
                p = safe_resolve_under_root(value)
                if p and safe_exists(p) and safe_is_file(p) and is_candidate(p):
                    selected.add(rel(p))
    for path in files:
        r = rel(path)
        if r in selected:
            continue
        if r in {"Cargo.toml", "wrangler.toml", "schema.sql", "README.md"}:
            selected.add(r)
    return sorted(selected)

def git_changed_files(files_by_rel):
    changed = []
    try:
        import subprocess

        proc = subprocess.run(
            ["git", "-C", str(root), "status", "--short"],
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            check=False,
        )
        for line in proc.stdout.splitlines():
            if not line:
                continue
            name = line[3:].strip()
            if " -> " in name:
                name = name.split(" -> ", 1)[1]
            if name in files_by_rel:
                changed.append(name)
    except Exception:
        pass
    return sorted(set(changed))

def project_summary(files, fp):
    by_ext = {}
    by_top = {}
    for path in files:
        r = rel(path)
        ext = path.suffix.lower() or path.name
        by_ext[ext] = by_ext.get(ext, 0) + 1
        top = r.split("/", 1)[0]
        by_top[top] = by_top.get(top, 0) + 1
    likely = [
        r
        for r in [rel(p) for p in files]
        if r in {"Cargo.toml", "wrangler.toml", "schema.sql", "src/lib.rs", "src/main.rs", "public/index.html", "public/admin/index.html", "README.md"}
    ]
    return {
        "generated_at": generated_at(),
        "mode": mode,
        "fingerprint": fp,
        "root": str(root),
        "file_count": len(files),
        "top_level_counts": dict(sorted(by_top.items())),
        "extension_counts": dict(sorted(by_ext.items())),
        "likely_entrypoints": likely,
        "cache_files": {
            "codegraph": ".ai-agent/generated/cache/codegraph-project.md",
            "lite": ".ai-agent/generated/cache/codegraph-lite.md",
            "symbols": ".ai-agent/generated/cache/symbol-index.md",
            "api": ".ai-agent/generated/cache/api-index.md",
            "schema": ".ai-agent/generated/cache/schema-index.md",
            "frontend": ".ai-agent/generated/cache/frontend-index.md",
            "dependencies": ".ai-agent/generated/cache/dependency-index.md",
            "docs": ".ai-agent/generated/cache/docs-index.md",
        },
    }

def index_excerpt(path: Path, max_lines: int):
    if not path.exists():
        return []
    return path.read_text(encoding="utf-8", errors="replace").splitlines()[:max_lines]

def write_codegraphs(files, fp, symbol_grouped):
    summary = project_summary(files, fp)
    (cache_dir / "project-summary.json").write_text(json.dumps(summary, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    files_by_rel = {rel(p): p for p in files}
    selected = set(task_reference_files(files))
    selected.update(git_changed_files(files_by_rel))
    if not selected:
        selected.update(summary["likely_entrypoints"][:8])

    lite_lines = ["# CodeGraph (lite)", "", f"Generated: {generated_at()}", f"Mode: {mode}", ""]
    for file_name in sorted(selected):
        path = files_by_rel.get(file_name)
        if not path:
            continue
        entries = markers_for(path, "symbol") or markers_for(path, "api") or markers_for(path, "frontend")
        lite_lines.append(f"## {file_name}")
        lite_lines.extend(entries[:80] or ["(no compact markers found)"])
        lite_lines.append("")
    (cache_dir / "codegraph-lite.md").write_text("\n".join(lite_lines).rstrip() + "\n", encoding="utf-8")

    project_max = int(os.environ.get("CODEGRAPH_PROJECT_MAX_LINES", "1800"))
    lines = [
        "# Project CodeGraph",
        "",
        f"Generated: {summary['generated_at']}",
        f"Mode: {mode}",
        f"Fingerprint: {fp}",
        "",
        "## Summary",
        f"- Root: {summary['root']}",
        f"- Files indexed: {summary['file_count']}",
        f"- Entry points: {', '.join(summary['likely_entrypoints']) or '(none detected)'}",
        "",
        "## Files",
    ]
    for r in [rel(p) for p in files]:
        lines.append(f"- {r}")
    lines.extend(["", "## Symbols"])
    for file_name, entries in symbol_grouped:
        lines.append("")
        lines.append(f"### {file_name}")
        lines.extend(entries)
        if len(lines) >= project_max and mode not in {"strict", "full"}:
            lines.append("")
            lines.append(f"_Truncated at {project_max} lines. Use CODEGRAPH_MODE=strict for a larger index._")
            break
    (cache_dir / "codegraph-project.md").write_text("\n".join(lines).rstrip() + "\n", encoding="utf-8")

def generate(force=False):
    cache_dir.mkdir(parents=True, exist_ok=True)
    runtime_dir.mkdir(parents=True, exist_ok=True)
    files = iter_files()
    fp, rows = fingerprint(files)
    state = load_state()
    outputs_exist = all((cache_dir / name).exists() for name in GENERATED_FILES)
    fresh = state.get("fingerprint") == fp and state.get("mode") == mode and outputs_exist
    if fresh and not force:
        print(f"CodeGraph fresh: {cache_dir / 'codegraph-project.md'}")
        return 0

    (cache_dir / "file-fingerprints.tsv").write_text(
        "".join(f"{digest}\t{size}\t{name}\n" for name, digest, size in rows),
        encoding="utf-8",
    )
    entry_limit = 180 if mode in {"strict", "full"} else 80
    symbol_grouped = group_markers(files, "symbol", entry_limit)
    write_index("symbol-index.md", "Symbol Index", symbol_grouped)
    write_index("api-index.md", "API Index", group_markers(files, "api", 120))
    write_index("schema-index.md", "Schema Index", group_markers(files, "schema", 160))
    write_index("frontend-index.md", "Frontend Index", group_markers(files, "frontend", 140))
    write_index("dependency-index.md", "Dependency Index", group_markers(files, "dependency", 120))
    write_index("docs-index.md", "Documentation Index", group_markers(files, "docs", 180))
    write_codegraphs(files, fp, symbol_grouped)
    write_state(fp)
    print(f"CodeGraph updated: {cache_dir / 'codegraph-project.md'}")
    print(f"Lite graph updated: {cache_dir / 'codegraph-lite.md'}")
    return 0

def status():
    state = load_state()
    if not state:
        print("CodeGraph status: missing")
        return 1
    print("CodeGraph status:")
    print(f"- generated_at: {state.get('generated_at', '(unknown)')}")
    print(f"- mode: {state.get('mode', '(unknown)')}")
    print(f"- fingerprint: {state.get('fingerprint', '(unknown)')}")
    print(f"- project: {cache_dir / 'codegraph-project.md'}")
    print(f"- lite: {cache_dir / 'codegraph-lite.md'}")
    return 0

def check():
    files = iter_files()
    fp, _ = fingerprint(files)
    state = load_state()
    outputs_exist = all((cache_dir / name).exists() for name in GENERATED_FILES)
    if state.get("fingerprint") == fp and state.get("mode") == mode and outputs_exist:
        print("CodeGraph check: fresh")
        return 0
    print("CodeGraph check: stale")
    print(f"- expected fingerprint: {fp}")
    print(f"- current fingerprint: {state.get('fingerprint', '(missing)')}")
    print(f"- mode: {mode}")
    return 1

if cmd in {"update", "build", "rebuild"}:
    raise SystemExit(generate(force=True))
if cmd == "ensure":
    raise SystemExit(generate(force=False))
if cmd == "status":
    raise SystemExit(status())
if cmd == "check":
    raise SystemExit(check())
print("Usage: agent-codegraph.sh status|check|update|ensure", file=sys.stderr)
raise SystemExit(2)
PY
