#!/bin/bash
declare -A ext

while read ext lang; do
    [[ "${ext:0:1}" == "#" ]] && continue
    [[ -z "$ext" || -z "$lang" ]] && continue
    ext["$ext"]="$lang"
done < <(cat etc/file_extensions.txt)

git ls-files | grep -v '^ruchat$' | xargs ctags
declare -A tags
s="[:space:]"
S="[^$s]"
s="[$s]"
#ctags --list-kinds-full | sed -n -E 's~^(Rust|Sh|Markdown|TOML)$s+([a-zA-Z])$s+($S+)($s+$S+){4}$s+(.*)$~\2 \1:\3:\4~p' |
while read -r k v; do
  tags[$k]="$v"
done < <(ctags --list-kinds-full -R src | sed -n -E "s~^($S+)$s+([a-zA-Z])$s+($S+)($s+$S+){4}$s+(.*)$~\1:\2 \3~p")

#Rust:C constant
#Rust:M macro
#Rust:P method
#Rust:c implementation
#Rust:e enumerator
#Rust:f function
#Rust:g enum
#Rust:i interface
#Rust:m field
#Rust:n module
#Rust:s struct
#Rust:t typedef
#Rust:v variable
#Markdown:S subsection
#Markdown:T l4subsection
#Markdown:c chapter
#Markdown:h hashtag
#Markdown:n footnote
#Markdown:s section
#Markdown:t subsubsection
#Markdown:u l5subsection
#Sh:a alias
#Sh:f function
#Sh:h heredoc
#Sh:s script
#TOML:K qkey
#TOML:a arraytable
#TOML:k key
#TOML:t table

if [ -n "$1" ]; then
  files=("$@")
else
  declare -a files
  while read -r f; do
      for extension in "${!ext[@]}"; do
          if [[ $f == *"$extension" ]]; then
              files+=("$f")
              break
          fi
      done
  done < <(git ls-files | grep -v '^ruchat$')
fi


model="all-minilm:l6-v2"
for f in "${files[@]}"; do
    metadata_json="{}"

    for extension in "${!ext[@]}"; do
        if [[ $f == *"$extension" ]]; then
            lang="${ext[$extension]}"
            metadata_json=$(if ! jq -n \
                --arg f "$f" \
                --arg lang "$lang" \
                '{
                    file: $f,
                    language: $lang
                }'; then
                echo "Error building base metadata JSON for $f" >&2
            fi)
            [ -z "$metadata_json" ] && continue

            # Process each matching line from tags file
            while IFS=$'\r' read -r var ex_cmd kind; do
                # Trim & normalize
                var="${var//[[:space:]]/}"
                [[ -z "$var" ]] && continue

                kind="${kind##[[:space:]]}"
                kind="${kind##kind:}"
                tag_fields=""
                [[ "$kind" =~ [[:space:]] ]] && {
                    tag_fields="${kind#*[[:space:]]}"
                    kind="${kind%%[[:space:]]*}"
                }

                lang_kind="${tags[$lang:${kind}]:-$kind}"
                metadata_json="$(if ! jq -n \
                    --argjson prev "$metadata_json" \
                    --arg kindKey "${var}.kind" \
                    --arg kind "$lang_kind" \
                    '$prev + {
                        ($kindKey): $kind
                    }'; then
                    echo "Error adding kind to metadata JSON for $f" >&2
                fi)"
                [ -z "$metadata_json" ] && continue

                # Build tags object
                if [[ -n "$tag_fields" ]]; then
                    IFS=$' \t' read -ra tag_array <<< "$tag_fields"
                    for tag_field in "${tag_array[@]}"; do
                        IFS=':' read -r tkey tval <<< "$tag_field"
                        tkey="${tkey//[[:space:]]/}"
                        tval="${tval//[[:space:]]/}"
                        [[ -z $tkey || -z $tval ]] && continue
                        metadata_json=$(if ! jq -n \
                            --argjson prev "$metadata_json" \
                            --arg k "${var}.$tkey" \
                            --arg v "$tval" \
                            '$prev + {($k): $v}'; then
                            echo "Error adding tag $tkey to metadata JSON for $f" >&2
                        fi)
                        [ -z "$metadata_json" ] && continue
                    done
                fi
                start=""
                end=""
                # Determine type & clean value
                if [[ $ex_cmd =~ ^/.*/$ ]]; then
                    # XXX: still Uncertain about these: '.?|' certain about excluding '+{}()\\'
                    ex_cmd="/^$(echo "${ex_cmd:2:-2}" | sed -r 's/([][*^$])/\\\1/g')$/"
                    start="$(sed -n "${ex_cmd}{=;q}" "$f")"
                    end="$start"
                    # FIXME: improver per lang/kind handling here:
                    if [[ "$lang:$lang_kind" =~ ^Rust:(function|method|implementation|macro|module|struct|enum)$ ]] || \
                            [[ "$lang:$lang_kind" =~ ^Sh:(function|script|heredoc)$ ]] || \
                            [[ "$lang:$lang_kind" =~ ^TOML:(arraytable|table|key)$ ]] || \
                            [[ "$lang:$lang_kind" == "Markdown:(chapter|section|subsection|subsubsection|l4subsection|l5subsection|footnote|hashtag)" ]]; then
                        # For these kinds, try to find the closing brace
                        # For these kinds, try to find the closing brace
                        ex_end_cmd=$(echo "$ex_cmd" | sed -E -n 's~(/\^[ \t]*).*$~\1[^[:space:]].*$/~p')
                        if [[ -n "$ex_end_cmd" ]]; then
                            end=$(sed -n "${ex_cmd},${ex_end_cmd}{${ex_cmd}b;${ex_end_cmd}{=;q}}" "$f")
                            [[ -z "$end" ]] && end="$start"
                        fi
                    fi
                elif [[ $ex_cmd =~ ^[0-9]+$ ]]; then
                    start="$ex_cmd"
                    end="$start"
                    # FIXME: improve per lang/kind handling here:
                    if [[ "$lang:$lang_kind" =~ ^Rust:(function|method|implementation|macro|module|struct|enum)$ ]] || \
                            [[ "$lang:$lang_kind" =~ ^Sh:function$ ]]; then
                        # For these kinds, try to find the closing brace
                        total_lines=$(wc -l < "$f")
                        brace_count=0
                        for (( line_num=start; line_num<=total_lines; line_num++ )); do
                            line_content=$(sed -n "${line_num}p" "$f")
                            # Count opening and closing braces
                            open_braces=$(grep -o '{' <<< "$line_content" | wc -l)
                            close_braces=$(grep -o '}' <<< "$line_content" | wc -l)
                            brace_count=$((brace_count + open_braces - close_braces))
                            if (( brace_count <= 0 )); then
                                end=$line_num
                                break
                            fi
                        done
                    fi
                fi
                
                if [[ -z "$start" ]]; then
                    echo "Warning: Could not determine start line for $var in $f with ex_cmd: $ex_cmd" >&2
                    continue
                fi
                #    --argjson tags "$tags_json" \
                #        tags:     $tags
                #    --arg lang    "$lang" \
                #        language: $lang,

                # Build one metadata object per item
                metadata_json=$(if ! jq -n \
                    --argjson prev "$metadata_json" \
                    --arg var     "$var" \
                    --arg start   "$start" \
                    --arg end     "$end" \
                    '{
                        ($var + ".start"): ($start | tonumber),
                        ($var + ".end"):   ($end   | tonumber)
                    } + $prev'; then
                    echo "Error adding line numbers to metadata JSON for $f" >&2
                fi)
                [ -z "$metadata_json" ] && continue
            done < <(sed -n -E "s~^($S+)\t$f\t((/.*\\\$/|[0-9]+)+)(;\"((\t$S+)*))?$~\1\r\2\r\5~p" tags)
        fi
    done

    # Now embed_args
    embed_args=("--collection" "repo_src-${model//:/_}" "--model" "$model")
    echo "Embedding file $f with metadata" >&2
    if jq -e 'length > 0' <<< "$metadata_json" >/dev/null; then
        embed_args+=("--metadata" "$(jq -c . <<< "$metadata_json")")
    else
        echo "No metadata extracted for $f (lang: $lang)" >&2
    fi

    ./ruchat embed "${embed_args[@]}" "$f contents:\n\`\`\`${lang}\n$(cat "$f")\n\`\`\`" || echo -e "Error in metadata for $f?\n" "${embed_args[@]}" 1>&2
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

