#!/bin/bash

model="all-minilm:l6-v2"
collection="repo_cargo_build-${model//:/_}"

./ruchat chroma-ls 2>&1 | grep -q "$collection" ||
  ./ruchat chroma-create --collection "$collection" --metadata "{\"model\": \"$model\"}" || exit 1

# Run cargo and filter for compiler messages
cargo build --message-format=json "$@" | jq -c 'select(.reason == "compiler-message")' | while read -r row; do

    # 1. Extract the full "rendered" message for the embedding content
    content=$(echo "$row" | jq -r '.message.rendered')

    # 2. Extract structured metadata
    # We grab the first primary span to get the exact file/line/col
    metadata=$(echo "$row" | jq -c '
        (.message.spans | map(select(.is_primary)) | .[0]) as $span
        | {
            code: (.message.code.code // "diagnostic"),
            level: .message.level,
            file: ($span.file_name // "unknown"),
            line: ($span.line_start // 0),
            column: ($span.column_start // 0),
            target: .target.name,
            package: .package_id
        }
    ')

    echo "Embedding $(echo "$metadata" | jq -r '.code') in $(echo "$metadata" | jq -r '.file')..."

    # 3. Send to ruchat
    ./ruchat embed \
        --collection "$collection" \
        --model "$model" \
        "$content" \
        --metadata "[$metadata]" || echo "Failed to embed entry" >&2

done
