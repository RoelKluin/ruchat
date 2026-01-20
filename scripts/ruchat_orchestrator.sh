#!/bin/bash
# Principal Software Engineer | Linux Automation Expert
# Dynamic AI Agent Orchestrator for Ruchat/Ollama

# example:
# ./scripts/ruchat_orchestrator.sh meta-gen "Generate a commandline to improve scripts/ruchat_orchestrator.sh"

# Summary of Session JSON structure:
#{
#  "history": ["./orchestrator.sh rust-analysis --file main.rs"],
#  "last_task": "rust-analysis",
#  "total_tokens": 14520,
#  "total_cost": 0.029,
#  "debate_logs": [
#    {
#      "file": "main.rs",
#      "safety_issue": "Potential use-after-free in FFI block",
#      "perf_issue": "APPROVED",
#      "timestamp": "14:20:05"
#    }
#  ]
#}

set -euo pipefail

SCRIPT_DIR=$( cd -- "$( dirname -- "${BASH_SOURCE[0]}" )" &> /dev/null && pwd )

# --- Configuration & Defaults ---
ITERATIONS=4
RUCHAT_BIN="target/release/ruchat"
HISTORY_FILE="/tmp/ruchat_full_history.log"
STRIP_CHATTER=true
DRY_RUN=false
STRICT_FORMAT="Requirement: Output raw data or code blocks only. No polite fillers. No conversational intros/outros. Use strictly technical language."
# --- Colors ---
C_ARCH='\033[1;32m' C_WORK='\033[1;34m' C_VALI='\033[1;33m'
C_CRIT='\033[1;31m' C_SUMM='\033[1;35m' NC='\033[0m'
C_PERF='\033[1;94m'

AGENT=("architect" "worker" "validator" "critic" "summarizer" "critic_perf" "chaos")
SAVE_SESSION="auto_save_$(date +%s)}"
RESUME_SESSION="$(ls -1t ~/.ruchat/sessions | head -n 1 2>/dev/null || echo "")"
TERSE_MODE=false

# Define default role-to-model mapping
declare -A OPTS=(
    [architect_model]="qwen2.5:7b"
    [worker_model]="deepseek-coder-v2"
    [validator_model]="qwen2.5:7b"
    [critic_model]="qwen2.5:7b"
    [summarizer_model]="mistral-nemo"
 
    [architect_temp]=0.0
    [worker_temp]=0.7
    [validator_temp]=0.0
    [critic_temp]=0.0
    [summarizer_temp]=0.0

    [architect_init]="System: You are the TECHNICAL ARCHITECT AGENT. Your role is to devise a comprehensive technical plan for the WORKER agent to implement. Focus on clarity, structure, and feasibility."
    [worker_init]="You are the IMPLEMENTATION AGENT. Follow the Architect's plan precisely."
    [validator_init]="You are the SMART VALIDATOR AGENT. Your role is to validate the WORKER's output against the provided specifications and ensure correctness."
    [critic_init]="You are the SAFETY CRITIC AGENT. Your role is to review the WORKER's output for safety, security, and best practices."
    [summarizer_init]="You are the SESSION SUMMARIZER AGENT. Your role is to condense the session history into a concise summary, focusing on key decisions and changes."
)
CRITIC_SAFETY_INIT="Safety Critic. Focus: Memory safety, race conditions, edge-case handling, and error propagation. Be pedantic."
CRITIC_PERF_INIT="Performance Critic. Focus: Zero-cost abstractions, avoiding heap allocations, Big-O complexity, and cache locality."

# Function to set argument
parse_opt_arg() {
    local what="$1"
    local opt="$2"
    local val="$3"
    case opt in
        --*arch*) role="architect" ;;
        --*work*) role="worker" ;;
        --*vali*) role="validator" ;;
        --*crit*) role="critic" ;;
        --*summ*) role="summarizer" ;;
        --*perf*) role="critic_perf" ;;
        --*chaos*) role="chaos" ;;
        *) role="worker" ;;
    esac
    OPTS[${role}_${what}]="$val"
}

options() {
    cat <<EOF 1>&2
Agent Configuration Options:
    -m, --model "{agent}:{model}"     Specify the model for Agent [worker] (e.g., "${AGENT[1]}:${OPTS[worker_model]}").
    -T, --temp "{agent}:<value>"      Temperature setting for Agent [worker] model (default: ${AGENT[1]}:${OPTS[worker_temp]}).
    -I, --init "{agent}:<prompt>"     Custom initialization prompt for specified Agent.
    --terse                           Enable terse mode for all agents (minimalist responses, adapts defaults).

Context Discovery Options:
    --file <path>                     Target file to operate on.
    --commit <hash>                   Target git commit hash to operate on.
    --subject <keyword>               Subject keyword to guide context discovery.
    --crate <name>                    Target crate to lookup

Session Management Options:
    -S, --save <session_name>         Save the session state under the given name [${SAVE_SESSION}].
    -R, --resume <session_name>       Resume a saved session by name [${RESUME_SESSION}].

Orchestration Loop Options:
    -i, --iterations <num>            Number of iterations for the orchestration loop (default: ${ITERATIONS}).
    --val-cmd <command>               Command to validate changes (e.g., "cargo build").
    --strip-chatter <true|false>      Enable or disable stripping of AI chatter from responses (default: ${STRIP_CHATTER}).

General Options:
    -h, --help [help-subject]         Show this help message. Optionally for a specific subject.
    -j, --dry-run <true|false>            Estimate costs and tokens without executing (default: ${DRY_RUN}).
    -D, --debug                           Enable debug mode with verbose logging.

EOF
}

models() {
    ech0 "Available Models:" 1>&2
    ollama list 1>&2
}

agents() {
    cat <<EOF 1>&2
Available Agents:
    architect       - Technical Architect Agent
    worker          - Implementation Agent
    validator       - Smart Validator Agent
    critic          - Safety Critic Agent
    summarizer      - Session Summarizer Agent
    critic_perf     - Performance Critic Agent
    chaos           - Chaos Injection Agent
EOF
}

goal() {
    cat <<EOF 1>&2
Goal:
    A concise description of the user's objective for the orchestrator.

EOF
}
usage() {
    case "$1" in
        option*) options;;
        agent*) agents;;
        model*) echo "Available Models (ollama list)" 1>&2;
            models;;
        task*) "$SCRIPT_DIR/task_manager.sh" --help tasks;;
        all)
            options
            agents
            goal
            models
            exit 0;;
        *)  [[ "$1" != "continue" ]] && echo -e "Usage: $0 [options] <task> \"<goal>\"\n\n" 1>&2
            options
            agents
            goal
            [[ -n "$1" ]] && echo -e "Error: $1\n\n" 1>&2
            exit "${2-0}";;
    esac
    exit 0
}

# New Global State
TARGET_FILES=()
TARGET_CRATE=""
TARGET_COMMIT=""
SUBJECT=""
TASK_TYPE=""
USER_GOAL=""
DEBUG=false

# Main Argument Loop Update
while [[ $# -gt 0 ]]; do
    case "$1" in
        -m|--mod*) parse_opt_arg "model" "$1" "$2"; shift 2 ;;
        -I|--init*) parse_opt_arg "init" "$1" "$2"; shift 2 ;;
        -T|--temp*) parse_opt_arg "temp" "$1" "$2"; shift 2 ;;
        --strip-chatter) STRIP_CHATTER="$2"; shift 2 ;;
        -i|--iter*) ITERATIONS="$2"; shift 2 ;;
        -S|--save) SAVE_SESSION="$2"; shift 2 ;;
        -R|--resume) RESUME_SESSION="$2"; shift 2 ;;
        --val-cmd) VAL_CMD="$2"; shift 2 ;;
        --terse)
            TERSE_MODE=true
            OPTS[architect_init]="System: You are the TECHNICAL ARCHITECT AGENT. Provide concise, minimalist plans with no extra explanation."
            OPTS[worker_init]="You are the IMPLEMENTATION AGENT. Provide concise code changes only."
            OPTS[validator_init]="You are the SMART VALIDATOR AGENT. Provide brief validation results without extra commentary."
            OPTS[critic_init]=""
            OPTS[summarizer_init]="You are the SESSION SUMMARIZER AGENT. Provide a brief summary of key decisions and changes."
            shift ;;

        --file) TARGET_FILES+=("$2"); shift 2 ;;
        --commit) TARGET_COMMIT="$2"; shift 2 ;;
        --subject) SUBJECT="$2"; shift 2 ;;
        --crate) TARGET_CRATE="$2"; shift 2 ;;

        -h|--help) usage "${2:-}";;
        -j|--dry-run) DRY_RUN="$2"; shift 2 ;;
        -D|--debug) DEBUG=true; set -Tx; shift ;;
        *) 
            if [[ -z "${TASK_TYPE}" ]]; then
                TASK_TYPE="$1"
                shift
            elif [[ -z "${USER_GOAL}" ]]; then
                USER_GOAL="${1%.}."
                shift
            else
                usage "Unknown argument: $1" 1
            fi
    esac
done

[[ -z "$USER_GOAL" ]] && usage "No user goal provided." 1

# --- Dynamic Role Engine ---

case "$TASK_TYPE" in
    git-ops|git-pr-apply|git-bisect-autofix)
        # Force these tasks to use the filtered context
        if [[ -n "$TARGET_COMMIT" ]]; then
            # Replace the heavy 'git show' in query_agent with this:
            GIT_PAYLOAD=$(query_git_context "$TARGET_COMMIT")
        fi
        ;;
    chaos-drill|rust-high-stakes)
        OPTS[critic_perf_model]="qwen2.5:7b"
        OPTS[critic_perf_temp]=0.0
        if [[ "${TERSE_MODE}" = true ]]; then
            OPTS[critic_perf_init]="System: You are the PERFORMANCE CRITIC AGENT. Provide concise performance reviews focusing on major optimizations."
        else
            OPTS[critic_perf_init]="System: You are the PERFORMANCE CRITIC AGENT. Your role is to evaluate the WORKER's output for performance optimizations and efficiency improvements."
        fi
        ;;
esac

if [[ "${TASK_TYPE}" == "chaos-drill" ]]; then
    OPTS[chaos_model]="mistral"
    OPTS[chaos_temp]=1.0
    if [[ "${TERSE_MODE}" = true ]]; then
        OPTS[chaos_init]="System: You are the CHAOS INJECTION AGENT. Introduce brief, impactful challenges without elaboration."
    else
        OPTS[chaos_init]="System: You are to inject random system constraints to test resilience."
    fi
fi

# Append to all _INIT variables
# Append to all system prompts to enforce high-signal communication
WORKER_INIT+=" $STRICT_FORMAT"


DENSE_SIGNAL="Instruction: Use Delimiters (###) for sections. Use 'Chain of Thought' (First, analyze... Then, implement...). Avoid all pleasantries. If providing code, provide ONLY the code."

ARCHITECT_INIT+=" $DENSE_SIGNAL Task: Create a structured prompt for the Worker that includes: 1. Input Context 2. Constraints 3. Expected Output Format."


CHAOS_REINIT="System: Continue chaos injection."
ARCHITECT_REINIT="System: Continue technical plan. Keep logic dense."

WORKER_REINIT="You are the IMPLEMENTATION AGENT. 
CONSTRAINTS:
1. Output RAW CODE BLOCKS or VIM SCRIPTS only.
2. API STABILITY: Your changes will be tested against 'Associated Files' (temporally coupled modules). 
3. If you modify a public Struct, Trait, or Function signature, you MUST ensure all coupled files provided in the context are updated or that the change is backward-compatible.
4. Minimize 'Diff Noise' to keep the validation rounds fast."

CRITIC_SAFETY_REINIT="System: Continue safety audit."
CRITIC_PERF_REINIT="System: Continue performance audit."
CRITIC_REINIT="Output: Use concise technical language. Provide raw code blocks only."

CRITIC_REJECTION="Critic rejected previous attempt. Feedback: "


# Cleanup function to close pipes and remove FIFOs
KEEP_ALIVE="${KEEP_ALIVE:-}"
cleanup() {
    if [[ -n "$KEEP_ALIVE" ]]; then
        return
    fi
    for a in "${AGENT[@]}"; do
        # 1. Use indirect expansion to get the FD value safely
        local varname="FD_${a^^}_IN"
        local fd="${!varname:-}"

        # 2. Check if the FD is set and refers to an open file descriptor
        if [[ -n "$fd" ]]; then
            # We use >&"$fd" to specify the exact file descriptor
            # and move the 2>/dev/null to the end of the command
            printf "\n---\n" >&"$fd" || true
        fi

        # 3. Remove the named pipes
        rm -f "/tmp/ruchat_${a}_in" "/tmp/ruchat_${a}_out" 2>/dev/null || true
    done
}

# --- Infrastructure Setup ---
SESSION_FILE="/tmp/ruchat_session_state.json"
PERSIST_DIR="${HOME}/.ruchat/sessions"
mkdir -p "$PERSIST_DIR"

save_session() {
    local session_name="${1:-session_$(date +%Y%m%d_%H%M%S)}"
    local target_path="${PERSIST_DIR}/${session_name}"
    mkdir -p "$target_path"

    cp "$HISTORY_FILE" "${target_path}/history.log"
    cp "$SESSION_FILE" "${target_path}/state.json"
    
    # Save target files to track drift
    [[ -f "${TARGET_FILES[0]}" ]] && cp "${TARGET_FILES[0]}" "${target_path}/last_known_file"
    
    echo -e "${C_SUMM}SESSION SAVED: ${session_name}${NC}"
}

resume_session() {
    local session_name="$1"
    local target_path="${PERSIST_DIR}/${session_name}"

    if [[ -d "$target_path" ]]; then
        cp "${target_path}/history.log" "$HISTORY_FILE"
        cp "${target_path}/state.json" "$SESSION_FILE"
        echo -e "${C_VALI}SESSION RESUMED: ${session_name}${NC}"
    else
        echo -e "${C_CRIT}ERROR: Session '${session_name}' not found.${NC}"
        exit 1
    fi
}
# Initialize session if not exists
# Initialize extended session if not exists
if [[ ! -f "$SESSION_FILE" ]]; then
    echo '{"history": [], "last_task": "", "total_tokens": 0, "total_cost": 0.0, "debate_logs": []}' > "$SESSION_FILE"
fi
estimate_costs() {
    echo -e "${C_SUMM}--- PRE-FLIGHT TOKEN ESTIMATE ---${NC}"
    
    # Calculate potential payload size
    local payload_size=0
    if [[ -n "$TARGET_COMMIT" ]]; then
        # Simulate query_git_context
        payload_size=$(git show --abbrev-commit --stat -p -U1 "$TARGET_COMMIT" | wc -c)
    elif [[ -f "${TARGET_FILEs[0]}" ]]; then
        for f in "${TARGET_FILES[@]}"; do
            payload_size=$(( payload_size + $(wc -c < "$f") ))
        done
    fi

    local est_input_tokens=$(( payload_size / 4 ))
    local est_per_round=$(( (est_input_tokens + 500) * 4 )) # (Payload + Prompt) * Agents
    local total_est=$(( est_per_round * ITERATIONS ))

    echo "Mode: $TASK_TYPE"
    echo "Estimated Session Tokens: $total_est"
    echo "Estimated Cost: \$$(awk "BEGIN {print $total_est / 1000 * 0.002}")"
    
    read -p "Begin Orchestration? (y/n) " -n 1 -r; echo
    echo -e "The replay was '$REPLY'"
    if [[ ! $REPLY =~ ^[Yy]$ ]]; then
        echo -e "${C_CRIT}Orchestration aborted by user.${NC}"
        exit 0
    fi
}
if [[ ! -f "tags" ]] && command -v ctags &> /dev/null; then
    echo -e "${C_SUMM}SYSTEM: Generating symbol map (ctags)...${NC}"
    # Generate tags for Rust, ignoring target and hidden dirs
    ctags -R --languages=Rust,sh,-TOML,-Cargo --exclude=target --exclude=.git . 2>/dev/null &
fi

# --- Initialization Phase ---
if [[ -z "${TARGET_FILES[0]}" && "$TASK_TYPE" != "meta-gen" ]]; then
    echo -e "${C_SUMM}SYSTEM: No target file provided. Discovering context...${NC}"
    DISCOVERED_FILES=$(discover_relevant_files "$SUBJECT")
    
    if [[ -z "$DISCOVERED_FILES" ]]; then
        echo -e "${C_CRIT}WARNING: No relevant files found in repository.${NC}"
    else
        echo -e "${C_VALI}FOUND CONTEXT:${NC}\n$DISCOVERED_FILES"
    fi
fi

# estimate tokens and cost in dry-run mode, optionally exit before execution
if [[ "$DRY_RUN" == "true" ]]; then
    estimate_costs
fi

trap cleanup EXIT

cleanup # Ensure no stale pipes exist

for agent in "${AGENT[@]}"; do
    mkfifo "/tmp/ruchat_${agent}_in" "/tmp/ruchat_${agent}_out"
done

# Start Ruchat Pipes
$RUCHAT_BIN pipe -m "${OPTS[architect_model]}" -o "{\"temperature\": ${OPTS[architect_temp]}}" < /tmp/ruchat_architect_in > /tmp/ruchat_architect_out &
$RUCHAT_BIN pipe -m "${OPTS[worker_model]}" -o "{\"temperature\": ${OPTS[worker_temp]}}" < /tmp/ruchat_worker_in > /tmp/ruchat_worker_out &
[[ -n "$VAL_CMD" ]] && \
  $RUCHAT_BIN pipe -m "${OPTS[validator_model]}" -o "{\"temperature\": ${OPTS[validator_temp]}}" < /tmp/ruchat_validator_in > /tmp/ruchat_validator_out &
$RUCHAT_BIN pipe -m "${OPTS[critic_model]}" -o "{\"temperature\": ${OPTS[critic_temp]}}" < /tmp/ruchat_critic_in > /tmp/ruchat_critic_out &
[[ "$STRIP_CHATTER" = true ]] && \
  $RUCHAT_BIN pipe -m "${OPTS[summarizer_model]}" -o "{\"temperature\": ${OPTS[summarizer_temp]}}" < /tmp/ruchat_summarizer_in > /tmp/ruchat_summarizer_out &
[[ -n "${OPTS[critic_perf_init]:-}" ]] && \
  $RUCHAT_BIN pipe -m "${OPTS[critic_perf_model]}" -o "{\"temperature\": ${OPTS[critic_perf_temp]}}" < /tmp/ruchat_critic_perf_in > /tmp/ruchat_critic_perf_out &
[[ -n "${OPTS[chaos_init]:-}" ]] && \
  $RUCHAT_BIN pipe -m "${OPTS[chaos_model]:-mistral}" -o "{\"temperature\": ${OPTS[chaos_temp]}}" < /tmp/ruchat_chaos_in > /tmp/ruchat_chaos_out &

# Infrastructure: Add the second Critic pipe

# Role Definitions
exec {FD_ARCH_IN}>/tmp/ruchat_architect_in {FD_ARCH_OUT}</tmp/ruchat_architect_out
exec {FD_WORK_IN}>/tmp/ruchat_worker_in {FD_WORK_OUT}</tmp/ruchat_worker_out
exec {FD_CRIT_IN}>/tmp/ruchat_critic_in {FD_CRIT_OUT}</tmp/ruchat_critic_out

# Conditional opens: Only open if the background process was actually started
if [[ -n "$VAL_CMD" ]]; then
    exec {FD_VALI_IN}>/tmp/ruchat_validator_in {FD_VALI_OUT}</tmp/ruchat_validator_out
fi

if [ "$STRIP_CHATTER" = true ]; then
    exec {FD_SUMM_IN}>/tmp/ruchat_summarizer_in {FD_SUMM_OUT}</tmp/ruchat_summarizer_out
fi

if [[ -n "${OPTS[critic_perf_init]:-}" ]]; then
    exec {FD_CRIT_PERF_IN}>/tmp/ruchat_critic_perf_in {FD_CRIT_PERF_OUT}</tmp/ruchat_critic_perf_out
fi

if [[ -n "${OPTS[chaos_init]:-}" ]]; then
    exec {FD_CHAOS_IN}>/tmp/ruchat_chaos_in {FD_CHAOS_OUT}</tmp/ruchat_chaos_out
fi

# --- Crate Helper Functions ---
fetch_crate_context_recursive() {
    local TARGET_CRATE=$1
    local BASE_URL="https://docs.rs/$TARGET_CRATE/latest/$TARGET_CRATE"
    
    # Check for 'pup' (HTML parser) and 'pandoc' (Converter)
    if ! command -v pup &> /dev/null || ! command -v pandoc &> /dev/null; then
        echo "Error: This function requires 'pup' and 'pandoc'."
        echo "Install via: brew install pup pandoc (or your package manager)"
        return 1
    fi

    echo "# Full Documentation Context: $TARGET_CRATE"
    
    # 1. Fetch the main index to find submodules
    local INDEX_HTML=$(curl -sL "$BASE_URL/index.html")
    
    # 2. Extract submodule links (looking for the 'modules' section in docs.rs)
    local MODULES=$(echo "$INDEX_HTML" | pup 'table.item-table td.mod a attr{href}')

    # 3. Process the Main Page
    echo "## Root Module (lib.rs)"
    echo "$INDEX_HTML" | pup '#main-content' | pandoc -f html -t markdown_strict
    echo -e "\n---\n"

    # 4. Iterate through found modules
    for MOD_PATH in $MODULES; do
        # Ensure we don't go out of bounds or hit external links
        if [[ "$MOD_PATH" == index.html ]]; then continue; fi
        
        local MOD_NAME=$(echo "$MOD_PATH" | sed 's/\/index.html//g')
        local FULL_URL="$BASE_URL/$MOD_PATH"
        
        echo "## Module: $MOD_NAME"
        curl -sL "$FULL_URL" | pup '#main-content' | pandoc -f html -t markdown_strict
        echo -e "\n---\n"
    done
}
fetch_crate_docs() {
    local TARGET_CRATE=$1

    if [ -z "$TARGET_CRATE" ]; then
        echo "Error: No crate name provided."
        return 1
    fi

    # Ensure cargo-docs-rs is installed
    if ! command -v cargo-docs-rs &> /dev/null; then
        echo "Installing cargo-docs-rs dependency..."
        cargo install cargo-docs-rs
    fi

    echo "--- START OF DOCUMENTATION FOR $TARGET_CRATE ---"
    
    # 1. Download and convert docs.rs content to Markdown
    # 2. Use 'grep -v' or 'sed' to remove common noise if necessary
    # 3. Output the result
    cargo docs-rs "$TARGET_CRATE" --format markdown
    
    echo "--- END OF DOCUMENTATION FOR $TARGET_CRATE ---"
}
fetch_crate_docs_raw() {
    local TARGET_CRATE=$1
    local URL="https://docs.rs/$TARGET_CRATE/latest/$TARGET_CRATE/"

    echo "Fetching documentation from $URL..."

    # Use curl to get the HTML, then pandoc to turn it into clean markdown
    # We target the 'main' ID which is where the actual content lives on docs.rs
    curl -sL "$URL" | \
        pandoc --from html --to markdown_strict-raw_html-native_divs-native_spans \
        --lua-filter=<(echo "function Div (el) return el.content end") 
}
# --- Git Helper Functions ---
query_git_context() {
    local commit="$1"
    git show --abbrev-commit --stat -p -U1 "$commit"
}
list_commit_hashes_for_file() {
    local file="$1"
    git log --pretty="%H" -- "$file"
}
list_files_in_commit() {
    local commit="$1"
    git diff-tree --no-commit-id --name-only "$commit" -r
}
list_associated_files() {
    local target_file="$1"
    local count="${2:-15}"
    local associated_files=""

    # 1. Files changed in the same commits as the target file
    while read -r commit; do
        associated_files+=" "$(list_files_in_commit "$commit")
    done < <(list_commit_hashes_for_file "$target_file")

    # 2. Deduplicate and return
    echo "$associated_files" | tr ' ' '\n' | sort -u | grep -v "^$target_file$" | head -n "$count"
}
sorted_files_on_change() {
    while read -r FILE; do
        git log --pretty="%ad $FILE" --date=iso8601-strict -1 -- "$FILE"
    done < <( git ls-files ) | sort | cut -f 2 -d " "
}
# reads file paths from stdin and outputs a tree structure
treeify() {
    tree --charset=ascii --fromfile /dev/stdin
}
relevant_crates_for_files() {
    local count="${1:-20}"
    # 1. Extract crate names from 'extern crate' and 'use' statements
    set +e
    grep '\.rs$' | xargs head -n "$count" | sed -r -n '/ crate::/b;s/^(extern crate |use )([a-zA-Z0-9_]+).*/\2/p' | sort -u
    set -e
}
top_symbols_for_files() {
    local count="${1:-50}"
    grep -E '\.(rs|sh)$' | xargs ctags -x --languages=Rust,sh,-TOML,-Cargo --sort=yes | \
    grep -Pv '(test[s_]|[ \t](field|method)[ \t])' | head -n "$count"
}
git_log_summary_for_files() {
    local files=("$@")
    local count="${2:-30}"
    git log -n "$count" --pretty=short -- "${files[@]}" | git shortlog
}
get_target_file_metadata() {
    local f="$0"
    [[ ! -f "$f" ]] && return

    echo -e "\n[METADATA: $f]"
    
    # 0. Temporal Coupling (What else changes with this file?)
    local associated; associated=$(list_associated_files "$f" 4)
    [[ -n "$associated" ]] && echo -e "Temporally Associated Files:\n$associated"

    # 1. Crate Dependencies (What external logic does this file rely on?)
    local deps; deps=$(echo "$f" | relevant_crates_for_files 9)
    [[ -n "$deps" ]] && echo -e "Detected Dependencies: $deps"

    # 2. Structural Symbols (High-level API surface)
    if [[ $f =~ \.md$ ]]; then
        # ctags does not support TOML well; skip for markdown files
        echo -e "Skipping Key Symbols for $f (Markdown file)." 1>&2
    else
        echo -e "Key Symbols in $f:"
        echo "$f" | top_symbols_for_files 14
    fi

    # 3. Git History Summary (Recent churn and context)
    echo -e "Recent History Summary:"
    git_log_summary_for_files "$f" 2
}

# --- Helper Functions ---
discover_relevant_files() {
    local subject="${1:-}"
    local more_files=""

    # 1. Base: Modified & Recent files
    local base_files
    base_files=$(git status --porcelain | awk '{print $2}')
    base_files+=" "$(git log -n 2 --pretty=format: --name-only)

    # 2. Structural: Use ctags to find where symbols in base_files are defined
    if [[ -f "tags" ]]; then
        for f in $base_files; do
            # Extract high-signal keywords (Structs, Enums, Traits) from the base file
            local symbols
            symbols=$(grep -E "(struct|enum|trait) " "$f" 2>/dev/null | awk '{print $2}' | tr -d '[:punct:]')
            
            for sym in $symbols; do
                # Find the file where this symbol is defined
                local def_file
                def_file=$(awk -v s="$sym" '$1 == s {print $2}' tags | head -n 1)
                [[ -n "$def_file" && -f "$def_file" ]] && more_files+=" $def_file"
            done
        done
    fi

    # 3. Keyword: ripgrep for the subject
    [[ -n "$subject" ]] && more_files+=" "$(rg -l --max-count 1 "$subject" | head -n 3)

    # Deduplicate and return
    echo "$base_files $more_files" | tr ' ' '\n' | sort -u | grep -v '^$' | head -n 8
}
maintain_context_efficiency() {
    local history_size
    history_size=$(wc -c < "$HISTORY_FILE")
    
    # 1. Deduplicate File Snapshots (Aggressive Pruning)
    # This removes intermediate [FILE: ...] blocks that haven't changed
    # to stop the "echo effect" in the context window.
    if [[ $history_size -gt 6000 ]]; then
        sed -i -E '/\[FILE: .*\]/,/\[\/FILE\]/ { N; /\[FILE: (.*)\].*\n.*\1/D; }' "$HISTORY_FILE"
    fi

    # 2. Semantic Compression (The Summarizer)
    if [[ $history_size -gt 12000 ]]; then
        echo -e "${C_SUMM}SYSTEM: Context limit approaching. Compressing logic...${NC}"
        local summary
        summary=$(query_agent "$FD_SUMM_IN" "$FD_SUMM_OUT" "$C_SUMM" "SUMMARIZER" \
            "Task: Distill this interaction. 
             Keep: Decisions, specific code fixes, and remaining errors. 
             Discard: General discussion and redundant snapshots. 
             History: $(cat "$HISTORY_FILE")")
        
        echo -e "### SESSION RESUME (Compressed) ###\n$summary" > "$HISTORY_FILE"
    fi
}
strip_chatter() {
    local input="$1"
    # Check if input contains triple backticks
    if [[ "$input" == *"\`\`\`"* ]]; then
        # Extract everything between first and last backticks, remove the markers themselves
        echo "$input" | sed -n '/^```/,/^```/p' | sed '/^```/d'
    else
        # Remove common "AI chatter" prefixes and suffixes
        echo "$input" | sed -E '1,3s/^(Certainly|Sure|Okay|Here is|Based on).*[.:]//gI' | \
                        sed -E '/(If you need|Hope this|Let me know)/d'
    fi
}
run_ripple_validation() {
    local primary_file="$1"
    local val_cmd="$2"
    local report=""

    # 1. Primary Check
    echo -ne "${C_VALI}VALIDATING PRIMARY: $primary_file... "
    set +e
    KEEP_ALIVE=true
    V_RES=$(eval "$val_cmd" 2>&1)
    V_EXIT=$?
    KEEP_ALIVE=false
    set -e
    if [[ $V_EXIT -eq 0 ]]; then
        echo -e "PASSED${NC}" 1>&2
        report+="[Primary: $primary_file Passed]\n"
    else
        # --- POST-MORTEM BRANCH ---
        # Detect Segfaults or Panics (Exit codes 139, 134, or specific strings)
        if [[ $V_EXIT -eq 139 || "$V_LOG" == *"panic"* || "$V_LOG" == *"segfault"* ]]; then
             echo -e "${C_CRIT}CRASH DETECTED. Extracting Post-Mortem...${NC}" 1>&2
             
             # Automatic GDB backtrace for the last binary run
             # (Assumes we are debugging 'target/debug/app')
             gdb -ex "set logging on gdb.log" -ex "run" -ex "bt" -ex "quit" ./target/debug/app &>/dev/null
             
             # Use Smart Validator to explain the crash
             CURRENT_PROMPT=$(query_agent "$FD_VALI_IN" "$FD_VALI_OUT" "$C_VALI" "VALIDATOR" \
                "Binary crashed (Code $V_EXIT). Analysis of log and GDB backtrace: \n$V_LOG\n$(cat gdb.log 2>/dev/null)")
        else
            # Standard compilation error
            CURRENT_PROMPT="Validation Failed ($VAL_CMD):\n$V_LOG"
        fi
        echo "### VALIDATION FAILURE ###"$'\n'"$CURRENT_PROMPT" >> "$HISTORY_FILE"

        echo -e "FAILED${NC}"
        echo "$V_RES"
        return 1
    fi

    # 2. Coupled Check (Check the 'Neighborhood')
    local coupled; coupled=$(list_associated_files "$primary_file" 3)
    if [[ -z "$coupled" ]]; then 
        report+="[Check Passed: $val_cmd]\n"
        return 0; 
    fi

    echo -e "${C_SUMM}CHECKING RIPPLE EFFECTS: $coupled${NC}" 1>&2
    local passed=true
    for f in $coupled; do
        # Check if the coupled file still compiles/tests correctly
        # We target specific tests if it's a Rust project
        if [[ "$val_cmd" == *"cargo"* ]]; then
             local test_cmd="cargo test --file $f"
             if ! eval "$test_cmd" &>/dev/null; then
                 report+="[RIPPLE FAILURE: $f might be broken by changes in $primary_file]\n"
                 echo -e "${C_CRIT}WARNING: Ripple failure in $f${NC}" 1>&2
                 passed=false

             fi
        fi
    done
    if [[ "$passed" = true ]]; then
        report+="[Ripple Check Passed: $val_cmd]\n"
    else
        local broken_file; broken_file=$(echo "$VAL_REPORT" | grep "RIPPLE FAILURE" | cut -d' ' -f3 | tr -d ']')
        
        # Get symbols from the primary and the broken file to find the mismatch
        local context_update="Regression detected in $broken_file.\n"
        for f in ${TARGET_FILES[@]}; do
            [[ $f =~ \.md$ ]] && continue
            context_update+="Symbols in $f:\n$(echo "$f" | top_symbols_for_files 10)\n"
        done
        [[ $broken_file =~ \.md$ ]] ||
            context_update+="Symbols in $broken_file:\n$(echo "$broken_file" | top_symbols_for_files 10)\n"

        
        CURRENT_PROMPT+="\n$context_update\nFix the interface mismatch."
    fi
    
    echo -e "$report"
    return 0
}

update_usage_stats() {
    local tokens="$1"
    local cost_per_1k="0.002" # Adjust based on your local/API pricing
    local added_cost
    added_cost=$(awk "BEGIN {print $tokens / 1000 * $cost_per_1k}")

    local updated
    updated=$(jq --argjson t "$tokens" --argjson c "$added_cost" \
        '.total_tokens += $t | .total_cost += $c' "$SESSION_FILE")
    echo "$updated" > "$SESSION_FILE"
}
update_session_debate() {
    local file="$1" s_crit="$2" p_crit="$3"
    local entry
    entry=$(jq -n --arg f "$file" --arg s "$s_crit" --arg p "$p_crit" \
        '{file: $f, safety_issue: $s, perf_issue: $p, timestamp: (now | strftime("%H:%M:%S"))}')
    jq ".debate_logs += [$entry]" "$SESSION_FILE" > "${SESSION_FILE}.tmp" && mv "${SESSION_FILE}.tmp" "$SESSION_FILE"
}
update_session() {
    local task="$1" cmd="$2"
    # Use jq to append the latest command and task to history
    local updated
    updated=$(jq --arg t "$task" --arg c "$cmd" \
        '.last_task = $t | .history += [$c]' "$SESSION_FILE")
    echo "$updated" > "$SESSION_FILE"
}

get_session_context() {
    jq -r '"[SESSION HISTORY]\nLast Task: " + .last_task + 
           "\nTokens Used: " + (.total_tokens|tostring) + 
           "\nEst. Cost: $" + (.total_cost|tostring) + 
           "\nCommand History: " + (.history | join(", "))' "$SESSION_FILE"
}
query_git_context() {
    local target="$1"
    # -w ignores whitespace, -U1 limits context lines to save tokens
    if git rev-parse "$target" >/dev/null 2>&1; then
        echo "--- SKINNY COMMIT ($target) ---"
        git show --abbrev-commit --stat -w -p -U1 "$target" | sed 's/^[[:space:]]*//'
    else
        echo "--- SKINNY DIFF ---"
        git diff -w -U1 "$target"
    fi
}
generate_file_manifest() {
    echo "[FILE MANIFEST]"
    # Provides: filename | short_hash | line_count
    find . -maxdepth 2 -name "*.rs" -exec du -b {} + | awk '{print $2 " | " $1 " bytes"}'
}
compress_context() {
    local long_context="$1"
    if [[ ${#long_context} -gt 4000 ]]; then
        echo -e "${C_VALI}SYSTEM: Context too large. Compressing...${NC}"
        query_agent "$FD_SUMM_IN" "$FD_SUMM_OUT" "$C_SUMM" "SUMMARIZER" \
            "Compress the following technical history into a dense, bulleted list of facts, decisions, and errors. Remove all prose: $long_context"
    else
        echo "$long_context"
    fi
}
run_bisect_session() {
    local good_rev="$1"
    local bad_rev="$2"
    
    echo -e "${C_SYS}SYSTEM: Starting Automated Bisect from $good_rev to $bad_rev${NC}"
    git bisect start "$bad_rev" "$good_rev"

    # Use 'git bisect run' to automate the logic engine as the decider
    local TARGETS=""
    for f in "${TARGET_FILES[@]}"; do
        TARGETS+=" --file $f"
    done
    git bisect run bash -c "
        # Run the orchestrator in a non-interactive mode for a single iteration
        # If orchestrator returns 0 (APPROVED), git treats commit as 'good'
        ./orchestrator.sh debug-test --iter 1 --file $TARGETS
    "
    
    echo -e "${C_SUMM}SYSTEM: Bisect Complete. Culprit identified.${NC}"
    git bisect reset
}
query_agent() {
    local in_fd=$1 out_fd=$2 color=$3 name=$4 prompt=$5
    local context=""

    # 1. Identity & Meta-Stats
    if [[ "$TASK_TYPE" == "meta-gen" ]]; then
        context+="\n$(get_session_context)\n"
        context+="\n[CAPABILITIES]\n$(usage)\n"
    fi

    # 2. Source Ground-Truth (Mutual Exclusivity Logic)
    if [[ -n "${GIT_PAYLOAD:-}" ]]; then
        context+="\n[SKINNY DIFF]\n$GIT_PAYLOAD\n"
    elif [[ -n "$TARGET_COMMIT" ]]; then
        context+="\n[COMMIT]\n$(git show --abbrev-commit --stat -p -U1 "$TARGET_COMMIT")\n"
    fi

    # --- Target File Enrichment ---
    for f in "${TARGET_FILES[@]}"; do
        if [[ -f "$f" ]]; then
            # Inject the specialized Git/Symbol metadata
            [[ "$f" != *".md" ]] ||
                context+="$(get_target_file_metadata "$f")\n"
            
            # Inject full content of the primary file
            context+="\n[SOURCE: $f]\n$(cat "$f")\n"
        fi
    done

    # Handle Multi-File Context
    if [[ -n "${DISCOVERED_FILES:-}" ]]; then
        context+="\n[LOGICAL NEIGHBORHOOD TREE]\n"
        context+="$(echo "$DISCOVERED_FILES" | treeify)\n"

        for f in $DISCOVERED_FILES; do
            local found=false
            for tf in "${TARGET_FILES[@]}"; do
                if [[ "$f" == "$tf" ]]; then
                    context+="\n[PRIMARY FILE: $f]\n$(cat "$f")\n"
                    found=true
                    break
                fi
            done
            [[ "$found" = false ]] && \
                context+="\n[REFERENCE FILE: $f]\n$(head -n 50 "$f")\n... (truncated)\n"
        done
    elif [[ -f "${TARGET_FILES[0]}" ]]; then
        for f in "${TARGET_FILES[@]}"; do
            [[ -f "$f" ]] && context+="\n[FILE: $f]\n$(cat "$f")\n"
        done
    fi

    # 3. Dynamic Tooling Context
    case "$TASK_TYPE" in
        editor-nav)
            if [[ -f "tags" ]]; then
                for f in "${TARGET_FILES[@]}"; do
                    context+="\n[CTAGS]\n$(grep "${f:-.*}" tags | head -n 30)\n" 
                done
            fi;;
        rust-meta-opt)
            context+="\n[BLOAT]\n$(cargo bloat --release -n 5 2>/dev/null)\n" ;;
        debug-core)
            # Post-Mortem data from the GDB branch in the main loop
            [[ -f "gdb.log" ]] && context+="\n[GDB BACKTRACE]\n$(cat gdb.log)\n" ;;
    esac

    # 4. Transmission & Reception
    printf "%b\n%b\n---\n" "$context" "$prompt" >&"$in_fd"
    
    local line full_resp=""
    while IFS= read -u "$out_fd" -r line; do
        [[ "$line" == "---" ]] && break
        if [[ "$line" == "USAGE:"* ]]; then
            update_usage_stats "$(echo "$line" | cut -d: -f2 | jq -r .tokens)"
            continue
        fi
        full_resp+="$line"$'\n'
    done

    # 5. Sanitization & Persistence
    local clean_resp
    clean_resp=$(strip_chatter "$full_resp")
    echo "### ${name} ###"$'\n'"${clean_resp}" >> "$HISTORY_FILE"
    
    # Visual Output
    printf "${color}${name}:${NC}\n%s\n" "$clean_resp" >&2
    echo "$clean_resp"
}

validate_output() {
    local text="$1" engine="$2"
    local total_errors=""
    local block_count=0

    # Extract all code blocks using a while-read loop and sed
    while IFS= read -r block; do
        [[ -z "$block" ]] && continue
        ((block_count++))
        
        local tmp_file="/tmp/ruchat_eval_block_${block_count}_$(date +%s)"
        echo "$block" > "$tmp_file"
        
        local err=""
        case "$engine" in
            shellcheck) err=$(shellcheck "$tmp_file" 2>&1 || true) ;;
            rustc)      err=$(rustc --crate-type lib "$tmp_file" -o /dev/null 2>&1 || true) ;;
            clippy)     err=$(cargo clippy --message-format=short 2>&1 || true) ;; # Assumes context is a crate
            awk|sed)    err=$(echo "test" | "$engine" -f "$tmp_file" 2>&1 >/dev/null || true) ;;
            "cargo test")
                echo -e "${C_VALI}SYSTEM: Executing tests...${NC}"
                # Capture stdout/stderr, including colored output for the LLM to see "Expected vs Actual"
                local test_log
                test_log=$(cargo test -- --nocapture 2>&1 || true)
                
                if [[ "$test_log" == *"test result: ok"* ]]; then
                    echo "SUCCESS"
                else
                    # Extract only the failing test failures to save context window tokens
                    local failure_report
                    failure_report=$(echo "$test_log" | sed -n '/----/,/test result:/p')
                    echo -e "FAIL: Test suite failed.\n$failure_report"
                fi
                ;;
        esac

        if [[ -n "$err" && "$err" == *"error"* ]]; then
            total_errors+="Block #$block_count Error: $err\n"
        fi
        rm -f "$tmp_file"
    done < <(echo "$text" | sed -n '/^```/,/^```/ { /^```/d; p; }')

    if [[ $block_count -eq 0 ]]; then
        echo "FAIL: No code blocks found to validate."
    elif [[ -z "$total_errors" ]]; then
        echo "SUCCESS"
    else
        echo -e "FAIL:\n$total_errors"
    fi
}

# --- Main Logic Engine ---
echo -e "${C_SUMM}TASK:${NC} Using provided goal for $TASK_TYPE."

# --- Meta-Agent Dispatcher (exists early and recurses) ---
if [[ "$TASK_TYPE" == "meta-gen" ]]; then
    echo -e "${C_SUMM}SYSTEM: Meta-Agent analyzing session and request...${NC}"
    for i in $(seq 1 1000); do
        echo -e "${C_SUMM}--- DISPATCHER ITERATION $i ---${NC}" 
        CMD_TO_RUN=$(query_agent "$FD_WORK_IN" "$FD_WORK_OUT" "$C_WORK" "DISPATCHER" \
            "Context: $(get_session_context). User Request: '$USER_GOAL'. Generate the next orchestrator command.")

        CMD_CLEAN=$(echo "$CMD_TO_RUN" | sed -n '/^```/,/^```/ { /^```/d; p; }' | head -n 1)
        [[ -z "$CMD_CLEAN" ]] && CMD_CLEAN="$CMD_TO_RUN"
        [[ -n "$CMD_CLEAN" ]] && {
            [[ "$DEBUG" == "true" ]] && CMD_CLEAN="$CMD_CLEAN --debug"
        }
    
        echo -e "${C_VALI}DISPATCHER SUGGESTS:${NC} $CMD_CLEAN"
        
        read -p "Execute? (y/n/q): " -n 1 -r; echo
        if [[ $REPLY =~ ^[Yy]$ ]]; then
            update_session "$TASK_TYPE" "$CMD_CLEAN"
            cleanup
            eval "$CMD_CLEAN"
            exit $?
        fi
        if [[ $REPLY =~ ^[Qq]$ ]]; then
            echo "Exiting Meta-Agent Dispatcher."
            break
        fi
    done
    exit 0
fi
# --- Consolidated Main Logic Engine ---
if [[ -n "$RESUME_SESSION" ]]; then
    resume_session "$RESUME_SESSION"
    # This function copies persistent logs into /tmp/ruchat_history.log
    # so the Architect sees the previous context in Round 1.
else
    # Standard fresh start
    echo > "$HISTORY_FILE"
    echo '{"history": [], "last_task": "", "total_tokens": 0, "total_cost": 0.0, "debate_logs": []}' > "$SESSION_FILE"
fi
CURRENT_PROMPT="Goal: $USER_GOAL"

# --- Final Main Execution Logic ---
for i in $(seq 1 "$ITERATIONS"); do
    echo -e "\n${C_VALI}--- ROUND $i ---${NC}"

    # 1. HOUSEKEEPING
    # Ensure context is lean before the Architect sees it
    maintain_context_efficiency 

    # 2. CHAOS INJECTION (Environmental constraints)
    if [[ -n "${OPTS[chaos_init]:-}" ]]; then
        TARGETS=""
        for f in "${TARGET_FILES[@]}"; do
            TARGETS+=" $f"
        done
        CHAOS_EVENT=$(query_agent "$FD_CHAOS_IN" "$FD_CHAOS_OUT" "$C_SUMM" "CHAOS" \
            "${OPTS[chaos_init]} Inject hazard based on files: $TARGETS.")
        OPTS[chaos_init]="${CHAOS_REINIT}"
        # Crucial: Prepend to the current goal so the Architect prioritizes the hazard
        CURRENT_PROMPT="[ENVIRONMENTAL HAZARD: $CHAOS_EVENT] Context: $CURRENT_PROMPT"
    fi

    # 3. ARCHITECT: Strategy & Payload Preparation
    # Optimization: Set Git context globally for this round
    if [[ "$TASK_TYPE" == "git-ops" || "$TASK_TYPE" == "git-pr-apply" ]]; then
        export GIT_PAYLOAD=$(query_git_context "$TARGET_COMMIT")
    fi

    # Branch logic for Prompt Optimization vs Standard
    if [[ "$TASK_TYPE" == "prompt-opt" ]]; then
        PLAN=$(query_agent "$FD_ARCH_IN" "$FD_ARCH_OUT" "$C_ARCH" "ARCHITECT" \
            "${OPTS[architect_init]}\nContext:\n$(cat "$HISTORY_FILE")\n\nTask: $CURRENT_PROMPT")
    else
        # Standard: Compress only the immediate prompt, not the system instructions
        CLEAN_PROMPT=$(compress_context "$CURRENT_PROMPT")
        PLAN=$(query_agent "$FD_ARCH_IN" "$FD_ARCH_OUT" "$C_ARCH" "ARCHITECT" \
            "${OPTS[architect_init]}\nHistory:\n$(cat "$HISTORY_FILE")\n\nTask: $CLEAN_PROMPT")
    fi
    # Clear Architect system prompt for Round 2+
    OPTS[architect_init]="${ARCHITECT_REINIT}"

    # 4. WORKER: Implementation
    WORKER_OUT=$(query_agent "$FD_WORK_IN" "$FD_WORK_OUT" "$C_WORK" "WORKER" "${OPTS[worker_init]}\nPlan: $PLAN")
    [[ -n "$STRIP_CHATTER" ]] && WORKER_OUT=$(strip_chatter "$WORKER_OUT")
    OPTS[worker_init]="${WORKER_REINIT}"

    # 5. ACTION & VALIDATION (The "Real World" result)
    VAL_REPORT=""
    
    # Apply Vim Edits if generated
    if [[ "$WORKER_OUT" == *":%s/"* || "$WORKER_OUT" == *'```vim'* ]]; then
        VIM_SCRIPT=$(echo "$WORKER_OUT" | sed -n '/^```\(vim\)\?$/,/^```$/p' | sed '1d;$d')
        [[ -z "$VIM_SCRIPT" ]] && VIM_SCRIPT=$(echo "$WORKER_OUT" | grep -E '^(:|%|s/)')
        if [[ -n "$VIM_SCRIPT" && -f "${TARGET_FILES[0]}" ]]; then
            TMP_VIM="/tmp/apply_fix.vim"
            { echo "set backup"; echo "$VIM_SCRIPT"; echo "wq"; } > "$TMP_VIM"
            if vim -u NONE -es -S "$TMP_VIM" "${TARGET_FILES[0]}"; then
                VAL_REPORT+="[Vim changes applied to ${TARGET_FILES[0]}] "
            else
                VAL_REPORT+="[Vim execution FAILED] "
            fi
        fi
    fi

    # Smart Validation (Compiler/Check) with Post-Mortem Analysis
    if [[ -n "$VAL_CMD" ]]; then
        # Pass the primary file and the validation command
        if ! VAL_REPORT=$(run_ripple_validation "${TARGET_FILES[0]}" "$VAL_CMD"); then
            # If primary fails, extract log and loop back
            CURRENT_PROMPT="Validation Failed:\n$VAL_REPORT\nAdjust implementation."
            continue
        fi
        
        # If primary passed but ripple failed, the Critic gets the report
        [[ "$VAL_REPORT" == *"RIPPLE FAILURE"* ]] &&
            echo -e "${C_CRIT}SYSTEM: Primary passed, but regressions detected in coupled files.${NC}" 1>&2
    fi
    # 6. CRITICISM (Multi-Agent or Single)
    if [[ -n "${OPTS[critic_perf_init]:-}" ]]; then
        echo -e "${C_CRIT}SYSTEM: Initiating Multi-Agent Debate...${NC}"
        
        REVIEW_SAFETY=$(query_agent "$FD_CRIT_IN" "$FD_CRIT_OUT" "$C_CRIT" "SAFETY" "${OPTS[critic_safety_init]} Review: $WORKER_OUT")
        OPTS[critic_safety_init]="${CRITIC_SAFETY_REINIT}"
        
        REVIEW_PERF=$(query_agent "$FD_CRIT_PERF_IN" "$FD_CRIT_PERF_OUT" "$C_PERF" "PERFORMANCE" "${OPTS[critic_perf_init]} Review: $WORKER_OUT")
        OPTS[critic_perf_init]="${CRITIC_PERF_REINIT}"

        if [[ "$REVIEW_SAFETY" == *"APPROVED"* && "$REVIEW_PERF" == *"APPROVED"* ]]; then
            update_session_debate "${TARGET_FILES[0]}" "APPROVED" "APPROVED"
            break
        else
            update_session_debate "${TARGET_FILES[0]}" "$REVIEW_SAFETY" "$REVIEW_PERF"
            CURRENT_PROMPT="DEBATE CONFLICT:\nSafety: $REVIEW_SAFETY\nPerformance: $REVIEW_PERF"
        fi
    elif [[ -n "${OPTS[critic_init]:-}" ]]; then
        # Single Critic
        CRITIC_PROMPT="Goal: $USER_GOAL\nAction Result: $VAL_REPORT\nWorker Output: $WORKER_OUT"
        REVIEW=$(query_agent "$FD_CRIT_IN" "$FD_CRIT_OUT" "$C_CRIT" "CRITIC" "${OPTS[critic_init]}\n$CRITIC_PROMPT")
        OPTS[critic_init]="${CRITIC_REINIT}"
        
        [[ "$REVIEW" == *"APPROVED"* ]] && break
        CURRENT_PROMPT="${CRITIC_REJECTION}$REVIEW"
    else
        # No Critic, assume success
        break
    fi
done
# --- Session Persistence ---
if [[ -n "$SAVE_SESSION" ]]; then
    # If no name was provided, the function handles timestamping
    save_session "$SAVE_SESSION"
fi

# --- Final Summarization ---
$RUCHAT_BIN pipe -m "${OPTS[summarizer_model]}" -o "{\"temperature\": ${OPTS[summarizer_temp]}}" < /tmp/ruchat_summarizer_in > /tmp/ruchat_summarizer_out &
exec {FD_SUMM_IN}>/tmp/ruchat_summarizer_in {FD_SUMM_OUT}</tmp/ruchat_summarizer_out

echo -e "\n${C_SUMM}SYSTEM: Finalizing documentation...${NC}"
query_agent "$FD_SUMM_IN" "$FD_SUMM_OUT" "$C_SUMM" "SUMMARY" "Create README.md from history: $(cat "$HISTORY_FILE")"
cleanup
