#!/bin/bash

find src -name "*.rs" | while read -r f; do
    id="Contents of file: $f"
    ./ruchat embed "$id:\n\`\`\`\n$(cat "$f")\n\`\`\`"
done
