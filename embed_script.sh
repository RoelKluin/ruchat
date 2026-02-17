#!/bin/bash
declare -A ext

[ -n "$DEBUG" ] && set -x

while read -r lang extension comment_re; do
    [[ "${lang:0:1}" == "#" ]] && continue
    [[ -z "$extension" || -z "$lang" ]] && continue
    ext["$extension"]="$lang"
done < <(cat etc/language_specifics.txt)

git ls-files | grep -v '^ruchat$' | ctags -L -
declare -A tags
s="[:space:]"
S="[^$s]"
s="[$s]"

# Parse the kinds safely
while read -r k v; do
  tags[$k]="$v"
done < <(ctags --list-kinds-full | awk '{print $3 ":" $2 " " $4}')


model="all-minilm:l6-v2"
collection="repo_src-${model//:/_}"

if [ -n "$1" ]; then
  files=("$@")
else
  ./ruchat chroma-delete --collection "$collection" 2>/dev/null || true
  ./ruchat chroma-create --collection "$collection" --metadata "{\"model\": \"$model\"}" || exit 1
  declare -a files
  while read -r f; do
    # Only process files with known extensions
    [[ -n "${ext[".${f##*.}"]}" ]] && files+=("$f")
  done < <(git ls-files | grep -v '^ruchat$' | tac)
fi


for f in "${files[@]}"; do
    metadata_array=()

    for extension in "${!ext[@]}"; do
        if [[ $f == *"$extension" ]]; then
            lang="${ext[$extension]}"

            # Initial metadata object
            base_info="$(jq -n \
                --arg f "$f" \
                --arg lang "$lang" \
                '{ file: $f, language: $lang }' 2>/dev/null)"

            if [ $? -ne 0 ] || [ -z "$base_info" ]; then
                 echo "Error building base metadata JSON for $f" >&2
                 continue # Skip file if base metadata fails
            fi

            # --- KEY FIX: Use awk for robust parsing instead of sed ---
            # 1. grep -F "$f" pre-filters lines containing the filename (fast)
            # 2. awk -F'\t' strictly splits by tabs, avoiding regex fragility
            while IFS=$'\a' read -r var ex_cmd kind; do
                echo "Processing tag: var='$var', ex_cmd='$ex_cmd', kind='$kind' for file '$f'" >&2

                # Trim & normalize
                var="${var//[[:space:]]/}"
                [[ -z "$var" ]] && continue

                kind="${kind##[[:space:]]}"
                kind="${kind##kind:}"
                tag_fields=""

                # Extract trailing fields if present
                if [[ "$kind" =~ [[:space:]] ]]; then
                    tag_fields="${kind#*[[:space:]]}"
                    kind="${kind%%[[:space:]]*}"
                fi

                lang_kind="${tags[$lang:${kind}]:-$kind}"

                # Safely update JSON
                new_json="$(jq -n \
                    --argjson prev "$base_info" \
                    --arg name "$var" \
                    --arg kind "$lang_kind" \
                    '$prev + { name: $name, kind: $kind }' 2>/dev/null)"

                if [ $? -eq 0 ] && [ -n "$new_json" ]; then
                    metadata_json="$new_json"
                fi

                # Build tags object
                if [[ -n "$tag_fields" ]]; then
                    IFS=$' \t' read -ra tag_array <<< "$tag_fields"
                    for tag_field in "${tag_array[@]}"; do
                        IFS=':' read -r tkey tval <<< "$tag_field"
                        tkey="${tkey//[[:space:]]/}"
                        tval="${tval//[[:space:]]/}"
                        [[ -z $tkey || -z $tval ]] && continue

                        new_json="$(jq -n \
                            --argjson prev "$metadata_json" \
                            --arg k "$tkey" \
                            --arg v "$tval" \
                            '$prev + {($k): $v}' 2>/dev/null)"

                        if [ $? -eq 0 ] && [ -n "$new_json" ]; then
                             metadata_json="$new_json"
                        fi
                    done
                fi

                start=""
                end=""

                # Handle regex search pattern vs line number
                if [[ "$ex_cmd" =~ ^/ ]]; then
                    # Convert ctags regex to sed address
                    # Note: We use # delimiter to avoid conflict with / in pattern
                    sed_re=$(printf '%s\n' "$ex_cmd" | sed -E 's#^([/?])(.*)\1;?$#/\2/#')

                    # Run sed on source file to find line number
                    start="$(sed -n "${sed_re}{=;q}" "$f" 2>/dev/null)"
                    end="$start"

                    # (Simplified end-finding logic for brevity - keeping your logic structure)
                    # You can add back the block specific logic here if needed

                elif [[ "$ex_cmd" =~ ^[0-9]+$ ]]; then
                    # It's already a line number
                    start="$ex_cmd"
                    end="$start"
                fi

                # Find references (call sites)
                if [[ -n "$var" ]]; then
                    # Escape special chars in var for regex search
                    safe_var=$(printf '%s\n' "$var" | sed 's/[][*^$.]/\\&/g')
                    references="$(rg --color=never --no-heading -n "(^|\W)${safe_var}(\W|$)" . | cut -d: -f1,2 | \
                        grep -v "^$f:$start$" | tr '\n' ',' | sed 's/,$//')"

                    if [[ -n "$references" ]]; then
                        new_json="$(jq -n \
                            --argjson prev "$metadata_json" \
                            --arg call "$references" \
                            '$prev + { references: $call }' 2>/dev/null)"
                         if [ $? -eq 0 ] && [ -n "$new_json" ]; then
                             metadata_json="$new_json"
                         fi
                    fi
                fi

                if [[ -z "$start" ]]; then
                    #echo "Warning: Could not determine start line for $var in $f" >&2
                    continue
                fi

                # Add start/end line numbers
                new_json="$(jq -n \
                    --argjson prev "$metadata_json" \
                    --arg start "$start" \
                    --arg end   "$end" \
                    '$prev + {
                        start: ($start | tonumber),
                        end:   ($end   | tonumber)
                    }' 2>/dev/null)"

                if [ $? -eq 0 ] && [ -n "$new_json" ]; then
                    metadata_array+=("$new_json")
                fi

            done < <(grep -F "	$f	" tags | awk -F'\t' -v f="$f" '
            $2 == f {
                # Find the separator ;" which marks the end of the Ex command
                line = $0
                idx = index(line, ";\"")
                if (idx > 0) {
                   tag = $1
                   # Remainder contains fields (everything after ;")
                   fields = substr(line, idx+2)

                   # ExCmd is between File<tab> and ;"
                   # We calculate position: length(tag) + 1 (tab) + length(file) + 1 (tab) + 1 (start)
                   cmd_start = length($1) + length($2) + 3
                   cmd_len = idx - cmd_start
                   cmd = substr(line, cmd_start, cmd_len)

                   # Print delimited by \a (Alert)
                   printf "%s\a%s\a%s\n", tag, cmd, fields
                }
            }')
        fi
    done

    if [ ${#metadata_array[@]} -gt 0 ]; then
      # Join array elements with comma
      metadata_json=$(printf '%s\n' "${metadata_array[@]}" | jq -s .)
    else
      metadata_json='[]'
    fi
    # Final Check and Embed
    embed_args=("--collection" "$collection" "--model" "$model")
    if [ ${#metadata_array[@]} -gt 0 ]; then
        embed_args+=("--metadata" "$(jq -c . <<< "$metadata_json")")   # already compact array
    else
      echo "No metadata for file $f" >&2
    fi

    ./ruchat embed "${embed_args[@]}" "$(cat "$f")" || echo -e "Error embedding $f?\n" "${embed_args[@]}" 1>&2
done
