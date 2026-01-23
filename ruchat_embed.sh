#!/bin/bash

find src -name "*.rs" | while read -r f; do
    id="Contents of file: $f"
    ./ruchat embed -m all-minilm:l6-v2 "$id:\n\`\`\`\n$(cat "$f")\n\`\`\`"
done
