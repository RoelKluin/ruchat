#!/bin/bash

# Configuration
model="all-minilm:l6-v2"
collection="repo_hist-${model//:/_}"

# Map of supported extensions (example set)

if [ -n "$1" ]; then
  hashes=("$@")
else
  ./ruchat chroma-delete --collection "$collection" 2>/dev/null || true
  ./ruchat chroma-create --collection "$collection" --metadata "{\"model\": \"$model\"}" || exit 1
  
  declare -a hashes
  while read -r h; do
    hashes+=("$h")
  done < <(git log --pretty=oneline -- src | awk '{print $1}')
fi

embed() {
    local content="$1"
    [ -z "$content" ] && return

    if [ ${#metadata_array[@]} -gt 0 ]; then
      metadata_json=$(printf '%s\n' "${metadata_array[@]}" | jq -s '.')
    else
      metadata_json='[]'
    fi

    embed_args=("--collection" "$collection" "--model" "$model" "$content")
    if [ ${#metadata_array[@]} -gt 0 ]; then
        embed_args+=("--metadata" "$(jq -c . <<< "$metadata_json")")
    else
        echo "No metadata for content" >&2
    fi

    ./ruchat embed "${embed_args[@]}" || echo -e "Error embedding content?\n" "${embed_args[@]}" 1>&2
}

for commit_hash in "${hashes[@]}"; do
    # State variables
    commit=""
    author=""
    date=""
    msg=""
    old_file=""
    new_file=""
    hunk=""
    in_msg=false
    in_hunk=false

    # We use -U0 to keep hunks concise, or standard -p for context
    while IFS= read -r line; do
        
        # 1. Extract Header Info
        if [[ "$line" =~ ^commit\  ]]; then
            commit="${line#commit }"
            continue
        elif [[ "$line" =~ ^Author:\  ]]; then
            author="${line#Author: }"
            continue
        elif [[ "$line" =~ ^Date:\  ]]; then
            date="${line#Date:   }"
            in_msg=true # Message follows Date
            continue
        fi

        # 2. Extract Message (indented lines after header)
        if $in_msg && [[ "$line" =~ ^\ \ \ \  || "$line" == "" ]]; then
            if [[ "$line" =~ ^diff\ --git ]]; then
                in_msg=false
            else
                msg+="$line"$'\n'
                continue
            fi
        fi

        # 3. Process Diff / Hunks
        if [[ "$line" =~ ^diff\ --git ]]; then
            # If we were in a hunk, embed the last one before moving to new file
            if [ -n "$hunk" ]; then embed "$hunk"; hunk=""; fi
            
            # Extract diff line
            diff_line="$line"
            in_hunk=false
            continue
        elif [[ "$line" =~ ^index\  ]]; then
            index_line="${line#index }"
            continue
        elif [[ "$line" =~ ^---\  ]]; then
            old_file="${line#--- a/}"
            continue
        elif [[ "$line" =~ ^\+\+\+\  ]]; then
            new_file="${line#+++ b/}"
            
            # Once we have the file info and message, we can build the base info
            # and embed the commit message itself first
            if [ -n "$msg" ]; then
                info=$(jq -n --arg c "$commit" --arg a "$author" --arg d "$date" \
                    '{ commit: $c, author: $a, date: $d, type: "message" }')
                metadata_array=("$info")
                embed "$msg"
                msg="" # Clear so we don't re-embed for every file in the commit
            fi
            continue
        fi

        # 4. Extract Hunks
        if [[ "$line" =~ ^@@ ]]; then
            # Embed previous hunk if exists
            if [ -n "$hunk" ]; then embed "$hunk"; fi
            
            # Extract hunk header: @@ -old_s,old_ct +new_s,new_ct @@
            # Use sed to pull the numbers
            read old_s old_ct new_s new_ct < <(echo "$line" | sed -n 's/^@@ -\([0-9]*\),\?\([0-9]*\) +\([0-9]*\),\?\([0-9]*\) @@.*/\1 \2 \3 \4/p')
            
            # Prepare metadata for this specific hunk
            info=$(jq -n \
                --arg commit "$commit" \
                --arg author "$author" \
                --arg date "$date" \
                --arg file "$new_file" \
                --arg os "$old_s" --arg oc "$old_ct" \
                --arg ns "$new_s" --arg nc "$new_ct" \
                '{ commit: $commit, author: $author, date: $date, file: $file, old_start: $os, old_ct: $oc, new_start: $ns, new_ct: $nc, type: "hunk" }')
            
            metadata_array=("$info")
            hunk="$line"$'\n'
            in_hunk=true
        elif $in_hunk; then
            hunk+="$line"$'\n'
        fi

    done < <(git log -n 1 -p "$commit_hash")
    
    # Final hunk in the file
    if [ -n "$hunk" ]; then embed "$hunk"; fi
done
