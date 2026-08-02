GOAL: {{GOAL}}.
{{#CORRECTION}}

CORRECTION FROM YOUR PREVIOUS ATTEMPT:
{{CORRECTION}}
{{/CORRECTION}}
{{COLLECTIONS}}

OUTPUT FORMAT - must be valid JSON, nothing else before or after:
{
  "query": string | [string, string, ...],
  "n_results": integer,
  "collection": string,
  "where": string | null,
  "ids": [string, ...] | null,
  "include": [string, ...] | null
}

WHERE clause syntax:
  field = 'value'          field != 'value'
  field > N   field >= N   field < N   field <= N     (numeric fields only)
  field IN ['a', 'b']       field NOT IN ['a', 'b']
  field CONTAINS 'value'    field NOT CONTAINS 'value' (substring on document text;
                                                         array-membership on metadata arrays)
  expr AND expr             expr OR expr               (parenthesize for grouping)
Do NOT use SQL constructs beyond this list.
Return ONLY the JSON. Do not add extra keys. Omit optional fields when not needed.
