GOAL: {{GOAL}}.
{{#CORRECTION}}

CORRECTION FROM YOUR PREVIOUS ATTEMPT:
{{CORRECTION}}
{{/CORRECTION}}
{{COLLECTIONS}}

The "Collection-specific examples" above show QUERY/WHERE SYNTAX only — they are
not related to your actual goal and must never be copied or adapted as-is. Your
"query" must be text you write yourself, derived from GOAL above, not the example
text.

OUTPUT FORMAT - must be valid JSON, nothing else before or after:
{
  "query": string | [string, string, ...],  // search text(s), about YOUR GOAL
  "n_results": integer,
  "collection": string | [string, string, ...],
  "where": string | null,
  "ids": [string, ...] | null,
  "include": [string, ...] | null
}

"collection" is normally a single collection name — use an array of names only
when your goal genuinely needs information from more than one of the collections
listed above in the same query (e.g. both source code and its commit history).
Each named collection is searched independently for the full "n_results", not
split between them.

WHERE clause syntax:
  field = 'value'          field != 'value'
  field > N   field >= N   field < N   field <= N     (numeric fields only)
  field IN ['a', 'b']       field NOT IN ['a', 'b']
  field CONTAINS 'value'    field NOT CONTAINS 'value' (substring on document text;
                                                         array-membership on metadata arrays)
  expr AND expr             expr OR expr               (parenthesize for grouping)
Do NOT use SQL constructs beyond this list.
Return ONLY the JSON. Do not add extra keys. Omit optional fields when not needed.

Reminder — your actual goal, verbatim: "{{GOAL}}"
