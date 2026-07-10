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

case "$CMD" in
  update|build|rebuild)
    bash "$AI_DIR/scripts/agent-codegraph.sh" update >/dev/null
    ;;
  ensure|check|status)
    bash "$AI_DIR/scripts/agent-codegraph.sh" ensure >/dev/null || true
    ;;
  *)
    echo "Usage: agent-knowledge-scan.sh status|check|update|ensure" >&2
    exit 2
    ;;
esac

python3 - "$PROJECT_ROOT" "$AI_DIR" "$CMD" "${SCAN_MODE:-smart}" <<'PY'
import datetime
import json
import sys
from pathlib import Path

root = Path(sys.argv[1]).resolve()
ai_dir = Path(sys.argv[2]).resolve()
cmd = sys.argv[3]
mode = sys.argv[4] or "smart"

cache_dir = ai_dir / "generated" / "cache"
knowledge_dir = ai_dir / "generated" / "knowledge"
state_path = cache_dir / "knowledge.state"

OUTPUTS = {
    "README.md",
    "architecture.md",
    "api.md",
    "database.md",
    "frontend.md",
    "documentation.md",
}

def now():
    return datetime.datetime.now(datetime.timezone.utc).astimezone().isoformat(timespec="seconds")

def load_graph_state():
    data = {}
    path = cache_dir / "codegraph.state"
    if path.exists():
        for line in path.read_text(encoding="utf-8", errors="replace").splitlines():
            if "=" in line:
                k, v = line.split("=", 1)
                data[k.strip()] = v.strip()
    return data

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
                f"generated_at={now()}",
                f"mode={mode}",
            ]
        )
        + "\n",
        encoding="utf-8",
    )

def read(path, max_lines=None):
    p = cache_dir / path
    if not p.exists():
        return ""
    lines = p.read_text(encoding="utf-8", errors="replace").splitlines()
    if max_lines is not None:
        lines = lines[:max_lines]
    return "\n".join(lines).rstrip()

def summary_json():
    path = cache_dir / "project-summary.json"
    if not path.exists():
        return {}
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except Exception:
        return {}

def write_doc(name, title, body):
    (knowledge_dir / name).write_text(f"# {title}\n\nGenerated: {now()}\n\n{body.rstrip()}\n", encoding="utf-8")

def generate(force=False):
    graph_state = load_graph_state()
    fp = graph_state.get("fingerprint", "")
    if not fp:
        print("Knowledge scan: missing CodeGraph state", file=sys.stderr)
        return 1
    state = load_state()
    outputs_exist = all((knowledge_dir / name).exists() for name in OUTPUTS)
    fresh = state.get("fingerprint") == fp and state.get("mode") == mode and outputs_exist
    if fresh and not force:
        print(f"Knowledge fresh: {knowledge_dir}")
        return 0

    knowledge_dir.mkdir(parents=True, exist_ok=True)
    summary = summary_json()
    likely = summary.get("likely_entrypoints") or []
    top_counts = summary.get("top_level_counts") or {}

    readme = f"""ไฟล์ในโฟลเดอร์นี้เป็น generated knowledge สำหรับให้ AI อ่านแทนการไล่อ่านทั้งโปรเจ็กต์

- `architecture.md` โครงสร้าง repo, entrypoint, dependency และภาพรวมจาก codegraph
- `api.md` endpoint, fetch, route และ marker ที่เกี่ยวกับ API
- `database.md` schema, migration, table, index และ reference สำคัญ
- `frontend.md` frontend/UI marker จาก `public/**` และไฟล์ UI
- `documentation.md` heading/index ของเอกสารใน repo

Source cache:
- `.ai-agent/generated/cache/project-summary.json`
- `.ai-agent/generated/cache/codegraph-project.md`
- `.ai-agent/generated/cache/codegraph-lite.md`
- `.ai-agent/generated/cache/*-index.md`

Fingerprint: `{fp}`
Mode: `{mode}`
"""
    (knowledge_dir / "README.md").write_text("# Project Knowledge\n\n" + readme.rstrip() + "\n", encoding="utf-8")

    architecture_body = f"""## Project Summary

```json
{json.dumps(summary, ensure_ascii=False, indent=2)}
```

## Top-Level Areas

{chr(10).join(f"- `{k}`: {v} indexed files" for k, v in sorted(top_counts.items())) or "- (none detected)"}

## Likely Entry Points

{chr(10).join(f"- `{item}`" for item in likely) or "- (none detected)"}

## Dependency Markers

{read("dependency-index.md", 320)}

## CodeGraph Excerpt

{read("codegraph-project.md", 260)}
"""
    write_doc("architecture.md", "Architecture Knowledge", architecture_body)

    write_doc("api.md", "API Knowledge", read("api-index.md", 900) or "(no API markers detected)")
    write_doc("database.md", "Database Knowledge", read("schema-index.md", 900) or "(no schema markers detected)")
    write_doc("frontend.md", "Frontend Knowledge", read("frontend-index.md", 1200) or "(no frontend markers detected)")
    documentation_body = f"""## Documentation Index

{read("docs-index.md", 900) or "(no documentation headings detected)"}

## README Build Inputs

- `.ai-agent/generated/cache/project-summary.json`
- `.ai-agent/generated/cache/codegraph-project.md`
- `.ai-agent/generated/knowledge/architecture.md`
- `.ai-agent/generated/knowledge/api.md`
- `.ai-agent/generated/knowledge/database.md`
- `.ai-agent/generated/knowledge/frontend.md`
"""
    write_doc("documentation.md", "Documentation Knowledge", documentation_body)
    write_state(fp)
    print(f"Knowledge updated: {knowledge_dir}")
    return 0

def status():
    state = load_state()
    if not state:
        print("Knowledge status: missing")
        return 1
    print("Knowledge status:")
    print(f"- generated_at: {state.get('generated_at', '(unknown)')}")
    print(f"- mode: {state.get('mode', '(unknown)')}")
    print(f"- fingerprint: {state.get('fingerprint', '(unknown)')}")
    print(f"- directory: {knowledge_dir}")
    return 0

def check():
    graph_state = load_graph_state()
    state = load_state()
    fp = graph_state.get("fingerprint", "")
    outputs_exist = all((knowledge_dir / name).exists() for name in OUTPUTS)
    if fp and state.get("fingerprint") == fp and state.get("mode") == mode and outputs_exist:
        print("Knowledge check: fresh")
        return 0
    print("Knowledge check: stale")
    print(f"- expected fingerprint: {fp or '(missing graph fingerprint)'}")
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
raise SystemExit(2)
PY
