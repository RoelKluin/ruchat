#!/bin/bash

git ls-files | grep -v '^ruchat$' | while read -r f; do
    id="Contents of file: $f"
    echo "Embedding $id.." 1>&2
    ./ruchat embed --model "all-minilm:l6-v2" "$id:\n\`\`\`\n$(cat "$f")\n\`\`\`"
done

# Embed git commit messages for the src directory
#git log --pretty=oneline -- src | while read -r line; do
#    commit_hash=$(echo "$line" | awk '{print $1}')
# ./ruchat embed -m all-minilm:l6-v2 "$line:\n$(git log -n 1 -p "$commit_hash")"
#done

# Embed contents of all files in the src directory
#ctags -R src
#f=tags
#id="Contents of file: $f"
#./ruchat embed -m all-minilm:l6-v2 "$id:\n\`\`\`\n$(cat "$f")\n\`\`\`"

