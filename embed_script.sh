#!/bin/bash
declare -A ext

# A
ext[".inp"]="Abaqus"
ext[".abc"]="Abc"
ext[".adb"]="Ada"
ext[".ads"]="Ada"
ext[".yml"]="Yaml"
ext[".yaml"]="Yaml"
ext[".yml:AnsiblePlaybook"]="AnsiblePlaybook"
ext[".yaml:AnsiblePlaybook"]="AnsiblePlaybook"
ext[".xml:Ant"]="Ant"
ext[".adoc"]="Asciidoc"
ext[".asc"]="Asciidoc"
ext[".asm"]="Asm"
ext[".asp"]="Asp"
ext[".au3"]="AutoIt"
ext[".ac"]="Autoconf"
ext[".am"]="Automake"
ext[".awk"]="Awk"

# B
ext[".beta"]="BETA"
ext[".bas"]="Basic"
ext[".bats"]="Bats"
ext[".bib"]="BibTeX"
ext[".bib:BibLaTeX"]="BibLaTeX"

# C
ext[".c"]="C"
ext[".h"]="C++"
ext[".cs"]="C#"
ext[".cpp"]="C++"
ext[".cxx"]="C++"
ext[".cmake"]="CMake"
ext[".css"]="CSS"
ext[".cu"]="CUDA"
ext[".toml:Cargo"]="Cargo"
ext[".clj"]="Clojure"
ext[".cbl"]="Cobol"
ext[".cob"]="Cobol"
ext[".cbl:CobolFree"]="CobolFree"
ext[".tags"]="Ctags"

# D
ext[".d"]="D"
ext[".xml:DBusIntrospect"]="DBusIntrospect"
ext[".dtd"]="DTD"
ext[".dts"]="DTS"
ext[".diff"]="Diff"
ext[".patch"]="Diff"
ext[".bat"]="DosBatch"

# E
ext[".e"]="Eiffel"
ext[".ex"]="Elixir"
ext[".elm"]="Elm"
ext[".el"]="EmacsLisp"
ext[".erl"]="Erlang"

# F
ext[".fal"]="Falcon"
ext[".l"]="LEX"
ext[".l:Flex"]="Flex"
ext[".fs"]="Forth"
ext[".f90"]="Fortran"
ext[".fypp"]="Fypp"

# G
ext[".gd"]="GDScript"
ext[".gperf"]="GPerf"
ext[".gdbinit"]="Gdbinit"
ext[".gemspec"]="GemSpec"
ext[".glade"]="Glade"
ext[".go"]="Go"

# H
ext[".html"]="HTML"
ext[".hs"]="Haskell"
ext[".hx"]="Haxe"

# I
ext[".rb:I18nRubyGem"]="I18nRubyGem"
ext[".ipynb"]="IPythonCell"
ext[".itcl"]="ITcl"
ext[".ini"]="Iniconf"
ext[".inko"]="Inko"
ext[".jni"]="JNI"

# J
ext[".json"]="JSON"
ext[".java"]="Java"
ext[".properties"]="JavaProperties"
ext[".js"]="JavaScript"
ext[".jl"]="Julia"

# K
ext[".kconfig"]="Kconfig"
ext[".kt"]="Kotlin"

# L
ext[".ld"]="LdScript"
ext[".lisp"]="Lisp"
ext[".lhs"]="LiterateHaskell"
ext[".lua"]="Lua"

# M
ext[".m"]="ObjectiveC"
ext[".m:MatLab"]="MatLab"
ext[".m4"]="M4"
ext[".mk"]="Make"
ext[".man"]="Man"
ext[".md"]="Markdown"
ext[".xml:Maven2"]="Maven2"
ext[".meson"]="Meson"
ext[".meson.options"]="MesonOptions"
ext[".pm:Moose"]="Moose"
ext[".myr"]="Myrddin"

# N
ext[".nsi"]="NSIS"

# O
ext[".ml"]="OCaml"
ext[".yaml:OpenAPI"]="OpenAPI"
ext[".org"]="Org"

# P
ext[".php"]="PHP"
ext[".pas"]="Pascal"
ext[".passwd"]="Passwd"
ext[".pl"]="Perl"
ext[".pod"]="Pod"
ext[".ps1"]="PowerShell"
ext[".proto"]="Protobuf"
ext[".pp"]="PuppetManifest"
ext[".py"]="Python"
ext[".cfg:PythonLoggingConfig"]="PythonLoggingConfig"

# Q
ext[".moc"]="QtMoc"
ext[".qmd"]="Quarto"

# R
ext[".r"]="R"
ext[".r6"]="R6Class"
ext[".rd"]="RDoc"
ext[".rex"]="REXX"
ext[".rmd"]="RMarkdown"
ext[".rb"]="Ruby"
ext[".rs"]="Rust"

# S
ext[".s4"]="S4Class"
ext[".scss"]="SCSS"
ext[".if"]="SELinuxInterface"
ext[".te"]="SELinuxTypeEnforcement"
ext[".sl"]="SLang"
ext[".sml"]="SML"
ext[".sql"]="SQL"
ext[".svg"]="SVG"
ext[".scd"]="Scdoc"
ext[".scm"]="Scheme"
ext[".sh"]="Sh"
ext[".stp"]="SystemTap"
ext[".sv"]="SystemVerilog"
ext[".service"]="SystemdUnit"

# T
ext[".toml"]="TOML"
ext[".ttcn"]="TTCN"
ext[".tcl"]="Tcl"
ext[".tex:TeXBeamer"]="TeXBeamer"
ext[".tf"]="Terraform"
ext[".tex"]="Tex"
ext[".thrift"]="Thrift"
ext[".t2t"]="Txt2tags"
ext[".ts"]="TypeScript"
ext[".tsp"]="TypeSpec"

# V
ext[".v"]="Verilog"
ext[".vhdl"]="VHDL"
ext[".varlink"]="Varlink"
ext[".vr"]="Vera"
ext[".vim"]="Vim"
ext[".rc"]="WindRes"

# X
ext[".xml"]="XML"
ext[".xrc"]="XRC"
ext[".xslt"]="XSLT"

# Y
ext[".y"]="YACC"
ext[".repo"]="YumRepo"

# Z
ext[".zep"]="Zephir"
ext[".zsh"]="Zsh"



git ls-files | grep -v '^ruchat$' | xargs ctags
declare -A tags
s="[:space:]"
S="[^$s]"
s="[$s]"
#ctags --list-kinds-full | sed -n -r 's~^(Rust|Sh|Markdown|TOML)$s+([a-zA-Z])$s+($S+)($s+$S+){4}$s+(.*)$~\2 \1:\3:\4~p' |
while read -r k v; do
  tags[$k]="$v"
done < <(ctags --list-kinds-full -R src | sed -n -r "s~^($S+)$s+([a-zA-Z])$s+($S+)($s+$S+){4}$s+(.*)$~\1:\2 \3~p")



model="all-minilm:l6-v2"

git ls-files | grep -v '^ruchat$' | while read -r f; do
    metadata=""
    lang="Unknown"
    for extension in "${!ext[@]}"; do
        if [[ -z "${f#*"$extension"}" ]]; then
            lang="${ext[$extension]}"
            for m in $(
                while IFS=$'\r' read -r var ex_cmd kind; do
                    var="${var//[[:space:]]/}"  # Trim var (names shouldn't have spaces anyway)
                    kind="${kind##[[:space:]]}"
                    kind="${kind##kind:}"
                    [[ -z "${kind/*[[:space:]]*/}" ]] && tag_fields="${kind#*[[:space:]]}" || tag_fields=""
                    kind="${kind%%[[:space:]]*}"
                    if [[ "$ex_cmd" =~ ^/.*/$ ]]; then
                        ex_cmd="${ex_cmd:1:-1}"
                        ex_type="regex"
                    else
                        ex_type="line"
                    fi
                    if [[ -n "$kind" && -n "$var" ]]; then
                        rust_kind="${tags[$lang:${kind}]:-${kind}}"  # Fallback to kind if no tag
                        jq_args=("-cn" "--arg" "file" "$f" "--arg" "kind" "$rust_kind" "--arg" "name" "$var" "--arg" "type" "$ex_type" "--arg" "value" "$ex_cmd" "--arg" "language" "$lang")
                        # Build tags as jq object string
                        tags_json='{}'
                        if [[ -n "$tag_fields" ]]; then
                            # Split tags by space/tab
                            IFS=$' \t' read -r -a tag_array <<< "$tag_fields"
                            for tag_field in "${tag_array[@]}"; do
                                IFS=':' read -r tag_key tag_value <<< "$tag_field"
                                tag_key="${tag_key//[[:space:]]/}"  # Trim
                                tag_value="${tag_value//[[:space:]]/}"
                                [[ -z "$tag_value" || -z "$tag_key" ]] && continue
                                # Add to tags_json with jq
                                tags_json=$(jq -n "$tags_json" | jq --arg k "$tag_key" --arg v "$tag_value" '.[$k] = $v')
                            done
                        fi
                        jq_filter="{language: \$language, kind: \$kind, name: \$name, file: \$file, type: \$type, value: \$value"
                        if [[ "$tags_json" != "{}" ]]; then
                            jq_args+=("--argjson" "tags_json" "$tags_json")
                            jq_filter+=", tags: $tags_json"
                        fi
                        jq_filter+="}"
                        # Output a JSON fragment for this entry
                        jq -n "${jq_args[@]}" "$jq_filter"
                    fi
                done < <(sed -n -r "s~^($S+)\t${f}\t((\/.*\\$\/|[0-9]+)+)(;\"((\t$S+)*))?$~\1\r\2\r\5~p" tags) |
                    jq -s 'reduce .[] as $item ({}; .file[$item.file][$item.kind][$item.name] = {type: $item.type, value: $item.value, language: $item.language})'); do
                metadata+="$m"
            done
        fi
    done
    embed_args=("--collection" "repo_src:$model" "--model" "$model")
    [[ -n "$metadata" ]] && embed_args+=("--metadata" "${metadata}") || echo -e "No metadata for $f (Lang: $lang)?" 1>&2

    ./ruchat embed "${embed_args[@]}" "Contents of file $f:\n\`\`\`\n$(cat "$f")\n\`\`\`" || echo -e "Error in metadata for $f?\n${metadata}" 1>&2
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

