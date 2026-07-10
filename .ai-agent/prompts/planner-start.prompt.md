Create or repair the implementation plan under `.ai-agent/ai-plan/`. Do not implement code.

Inputs to use:
- Read `.agent/requirement.md` if it exists.
- Read `.ai-agent/generated/runtime/context-package.md` first if it exists, then `.ai-agent/generated/runtime/runtime-context.md` if needed.
- Use only the current context level and allowlist prepared by the script. Do not broaden project search unless the script explicitly escalated context or the requirement clearly demands it.
- Use the compact package's relevant codegraph/knowledge excerpts as the first project map before opening broad source files.
- Read existing `.ai-agent/ai-plan/overview.md`, `.ai-agent/ai-plan/context.md`, and current task files if they exist.
- If the compact package says old plan isolation is active, summarize old plan files briefly only; do not let unrelated old tasks dominate the new plan unless the requirement explicitly says to continue the old plan.
- If one source is missing, continue with the remaining sources instead of failing.

Required outputs:
- Update or create `.ai-agent/ai-plan/overview.md`
- Update or create `.ai-agent/ai-plan/context.md`
- Update or create `.ai-agent/ai-plan/tasks/task-*.md`

Planning rules:
- Break work into small tasks with clear outcomes, validation, and explicit implementation scope.
- If the compact context package says `task_size: SMALL`, create only one task unless there is a clear technical reason to split.
- For SMALL frontend/content tasks, do not include backend, database, migrations, scripts, auth, or admin files unless the requirement explicitly mentions them.
- If the compact context package includes `Inferred Allowed Edit Area`, use it as the starting scope and do not leave Allowed Files empty when the requirement clearly points to files or modules.
- Include a `## Reference Map` section in every task. List the generated knowledge/cache files and exact source files the Coder and Reviewer should inspect for that task.
- Prefer task references from generated knowledge/codegraph first, then targeted source files. Do not instruct Coder or Reviewer to read the whole project.
- Load generated knowledge lazily: include only frontend/backend/database/documentation knowledge relevant to the task.
- Follow the configured planner task detail level appended below this prompt. The configured level overrides generic instincts about how coarse or fine the plan should be.
- Scope must be complete enough for the coder to finish the task without being forced into out-of-scope edits.
- Include every implementation file that is directly implied by the requirement, context, routing surface, auth flow, frontend entrypoint, shared helper, test, or config touched by the task.
- Validation commands must prove each acceptance criterion directly. Avoid broad combined searches that can pass when only one required file changed; prefer file-specific checks for schema, migration, route, UI, and test expectations.
- Do not create artificially narrow scopes just to minimize file count. If safe completion needs multiple files, include them.
- For the most detailed planning levels, optimize for coder success, not task count minimization. It is acceptable to create many small tasks when that reduces ambiguity.
- If a requirement spans multiple layers such as backend route wiring, auth handling, notification text, and SPA deep-link consumption, either split them into separate tasks or give one task the full cross-file scope it needs.
- Keep scope tight to relevant implementation files only. Do not include agent framework/runtime/task-state paths such as `.ai-agent/**`, root `AGENTS.md`, `.gitignore`, or `.agent/loop-verdict.txt` in implementation scope.
- Prefer revising existing task files when repairing a bad plan instead of creating duplicate tasks.
