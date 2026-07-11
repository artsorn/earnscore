# AI Agent Framework

Use the compact context package before broad source exploration:

1. `.ai-agent/generated/runtime/task-size.txt`
2. `.ai-agent/generated/runtime/context-package.md`
3. `.ai-agent/generated/runtime/context-escalation.txt`
4. `.ai-agent/generated/runtime/context-manifest.txt`
5. The current task file under `.ai-agent/ai-plan/tasks/`
6. Allowed edit files from the current task
7. `.ai-agent/generated/runtime/runtime-context.md`
8. `.ai-agent/generated/runtime/reviewer-summary.md`
9. Relevant codegraph/source snippets inside `context-package.md`
10. Targeted source files listed by the task `Reference Map`

Search rules:
- Search only the paths in `.ai-agent/generated/runtime/search-allowlist.txt` unless the current task explicitly requires a broader source file.
- Start from the lowest context level in `.ai-agent/generated/runtime/context-escalation.txt`; broaden only when the script escalates context.
- Respect `SEARCH_BUDGET` and avoid repeated searches for the same keyword.
- Stop searching once the task, allowed files, relevant codegraph, and source snippets provide enough implementation context.
- For `SMALL` tasks, keep search and edits inside the inferred allowed edit area unless the requirement explicitly demands a broader change.

Runtime artifact rules:
- During normal coding, do not automatically read or search `.ai-agent/generated/runtime/*.log`, `coder-round-*.log`, `reviewer-round-*.log`, `token-usage*`, or `*.jsonl`.
- Do not automatically search `.ai-agent/generated/runtime`, `.ai-agent/generated/cache`, `.ai-agent/generated/tmp`, `.ai-agent/generated/history`, or `.ai-agent/generated/token*`.
- Raw runtime logs are allowed only when `REPAIR_MODE=true` or the user explicitly asks to investigate previous runs. Prefer `.ai-agent/generated/runtime/reviewer-summary.md` before raw logs.
- Operational `*.log` files are compact summaries. Exact CLI stdout is retained separately as compressed `*.raw.log.gz` debug artifacts; never inject those raw artifacts into model context.
- Run required checks through `.ai-agent/bin/aia validate -- <command> [args...]` so environment blocks are not mislabeled as passes.
- Load generated knowledge lazily from the compact package; do not preload every file in `.ai-agent/generated/knowledge/`.

Use the current task scope strictly. `.ai-agent/**`, root `AGENTS.md`, `.gitignore`, and `.agent/loop-verdict.txt` are agent framework/runtime/task-state files, not implementation diff, and are not part of task review.
