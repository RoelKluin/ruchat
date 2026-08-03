GOAL: {{GOAL}}.
RAW HISTORY TO COMPRESS: {{HISTORY}}

This summary REPLACES the raw history above entirely — nothing not captured here will be
visible to future rounds. Compress it into a single dense summary that preserves everything
a future round needs to keep making progress toward GOAL without repeating past mistakes.

Always keep, verbatim where possible:
- Every concrete file path, function/struct/symbol name, and line number mentioned anywhere
  in the history.
- Every approach that was already tried and rejected, and the SPECIFIC reason it was
  rejected (a Validator/Tester/Critic reason, or a patch that failed to apply and why) — a
  future round must not waste a turn retrying something already known not to work.
- The current state of the implementation: what has actually been changed so far, and in
  which file(s) — distinct from what was only proposed or attempted and failed.
- Any concrete fact established by a lookup (ripgrep/read_file/git_*/cargo_check/
  cargo_clippy result) that would otherwise have to be rediscovered at the cost of another
  round's one lookup budget.

Discard: pleasantries, restated instructions, narrative commentary, and anything that
doesn't carry one of the facts above — a shorter summary that keeps every fact above is
better than a longer one that also keeps filler.

Write plain prose, no markdown headers or bullet formatting. Dense, not padded — but never
so short that a fact listed above is lost; a longer summary that's actually complete beats a
shorter one that forces a future round to re-discover something it already knew.

Reminder — your actual goal, verbatim: "{{GOAL}}"
