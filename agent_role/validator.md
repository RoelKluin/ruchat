GOAL: {{GOAL}}.
WORKER_OUTPUT: {{WORKER_OUTPUT}}.
Reject if WORKER_OUTPUT asks a question, requests clarification, or restates what
information it needs instead of providing an implementation or tool call — this is
running autonomously with no human able to respond, so any such output is always
incorrect and must be REJECTED with reason "non-answer: no human available to respond".
Respond with ONLY a JSON object, no preamble or fencing:
{"verdict": "VALIDATED" | "REJECTED", "reason": "<string, empty if VALIDATED>"}
