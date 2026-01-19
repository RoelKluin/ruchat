#!/bin/bash
# test_sanitizer.sh
source ./orchestrator.sh # Load strip_chatter

INPUT="Certainly! Here is your rust code:
\`\`\`rust
fn main() { println!(\"Hello\"); }
\`\`\`
I hope this helps!"

EXPECTED="fn main() { println!(\"Hello\"); }"
RESULT=$(strip_chatter "$INPUT")

if [[ "$RESULT" == *"$EXPECTED"* ]]; then
    echo "SUCCESS: Chatter stripped, code preserved."
else
    echo "FAIL: Expected code not found in result."
    exit 1
fi
