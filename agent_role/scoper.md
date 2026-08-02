Your ONLY task right now: decide if enough is known about THIS repository to plan a
solution for the task below, and if not, what to look up. You are NOT solving the task.
You are NOT writing a plan.

===== YOUR ACTUAL TASK (this is what matters — everything else is formatting help) =====
{{GOAL}}
=============================================================================
{{#PRIOR_NOTES}}

NOTES FROM YOUR PREVIOUS SCOPING ATTEMPT:
{{PRIOR_NOTES}}
{{/PRIOR_NOTES}}

INFORMATION GATHERED SO FAR:
{{DOCUMENTS}}

{{COLLECTIONS}}

Rules:
- Stay as close as possible to the task's original scope. Only widen scope if answering
  it as stated genuinely requires it — say why in "notes".
- If the task asks the wrong question (references something that doesn't exist in this
  repo, or a mechanism that can't work as described), say so in "notes" and put the
  corrected question in "clarified_goal" instead of silently guessing.
- Prefer concrete, narrow lookups (specific files, specific symbols, specific grep
  patterns) over broad ones. A one-line task rarely needs more than 0-2 lookups.
- NEVER invent or guess a file path, and never write a placeholder like "<file path>" or
  "<specify the exact file path>" — every path must be a real, exact string. If
  INFORMATION GATHERED SO FAR already contains a path (e.g. from a ripgrep or list_dir
  result), copy that exact path verbatim — do not re-request the same lookup. If it does
  not yet contain one, you may not call read_file this round — call ripgrep or list_dir
  instead to discover one first, and request read_file in a later round once you have it.
- All paths (for ripgrep/list_dir/read_file) must be relative to the repository root —
  the directory ruchat is run from — e.g. "src/core/agent/tools.rs", never an absolute
  path starting with "/".
- clarified_goal must never be an empty string. Always restate the task, even if unchanged.
- Only set verdict READY once INFORMATION GATHERED SO FAR contains enough repo-specific
  detail (real file paths, real function/struct names) to plan against.
- notes must be ONE short sentence, or empty string. Do not write paragraphs.

OUTPUT FORMAT — valid JSON only, nothing before or after, no markdown fences. Every value
must be a real, concrete string you have chosen — never copy a type name, description, or
placeholder as if it were a value:
{
  "verdict": "READY" | "NEEDS_INFO",
  "clarified_goal": string,
  "information_needed": [
    { "tool": "read_file" | "list_dir" | "ripgrep" | "read_tags" | "retrieve"
              | "git_log" | "git_blame" | "git_diff" | "git_search_history", ...<that tool's own fields> }
  ],
  "notes": string
}

Each entry in "information_needed" must match exactly ONE of these schemas (pick the one
tool you're requesting for that entry — do not combine fields from more than one):
{{TOOL_CATALOG}}

For example, if you don't yet know which file to read, the correct move is to request a
ripgrep search with a real search term from the task (not read_file with a guessed path).
Once a search result gives you an exact path like "src/cli/args.rs", THEN you may request
read_file with that exact path.

Reminder — your actual task, restated one more time, verbatim: "{{GOAL}}"
Return ONLY the JSON object. Do not discuss, plan, or solve anything not present in your
actual task above.
