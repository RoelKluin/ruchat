#!/usr/bin/env bash
#
# AGENT ORCHESTRATOR
# Principal Software Engineer / Linux Automation Expert
#
# Usage: ./orchestrator.sh [TASK] [FLAGS]
#
[ -n "$DEBUG" ] && set -x

set -euo pipefail
stty -echoctl # Disable ^C echo to keep output clean

# --- Globals & Constants ---
TERM_WIDTH=$(tput cols)
C_ARCH="\033[1;34m"  # Blue
C_WORK="\033[1;32m"  # Green
C_CRIT="\033[1;31m"  # Red
C_VALI="\033[1;33m"  # Yellow
C_SYS="\033[1;37m"   # White
NC="\033[0m"         # No Color

# Defaults
MODEL="gpt-4-turbo"
TEMP="0.7"
ITERATIONS=3
TASK_CONTEXT=""
LOOP_TYPE="STANDARD" # STANDARD | VALIDATED
VALIDATOR_ENGINE=""  # SHELLCHECK | RUSTC | CARGO_AUDIT

# --- Helper Functions ---

usage() {
    echo -e "${C_SYS}Usage: $0 <task_context> [options]${NC}"
    echo -e "\nTasks:"
    echo -e "  rust-analysis   : Ownership/Borrow checking, unsafe audit"
    echo -e "  git-ops         : Semantic commits, squashing, bisect"
    echo -e "  docs            : Rustdoc, READMEs, Changelogs"
    echo -e "  qa              : Unit tests, cargo audit, flamegraph"
    echo -e "  shell           : Bash generation with Shellcheck"
    echo -e "\nOptions:"
    echo -e "  -m, --model     : LLM Model (default: $MODEL)"
    echo -e "  -t, --temp      : Temperature (default: $TEMP)"
    echo -e "  -i, --iter      : Max iterations (default: $ITERATIONS)"
    echo -e "  -h, --help      : Show this help"
    exit 1
}

# Mocking the AI Query function for standalone execution
# In prod, this wraps curl/ollama/openai-cli
query_agent() {
    local color_in="$1" # Unused in mock
    local color_out="$2" # Unused in mock
    local color_name="$3"
    local role="$4"
    local prompt="$5"

    echo -e "${color_name}[$role] Generating response...${NC}" >&2
    
    # Simulation Logic for Demo Purposes
    if [[ "$role" == "CRITIC" ]]; then
        # Randomly approve or reject to simulate loop
        if (( RANDOM % 2 )); then echo "APPROVED"; else echo "Issues found in logic."; fi
    else
        echo "Response from $role based on: ${prompt:0:50}..."
    fi
}

extract_code() {
    # Extracts content between markdown code blocks
    sed -n '/^```/,/^```/ p' | sed '/^```/d'
}

validate_code() {
    local input_text="$1"
    local code_content
    code_content=$(echo "$input_text" | extract_code)

    if [[ -z "$code_content" ]]; then
        echo "FAILED: No code block found."
        return
    fi

    local temp_file
    temp_file=$(mktemp)
    trap 'rm -f "$temp_file"' RETURN

    echo "$code_content" > "$temp_file"
    local output=""
    local status=0

    case "$VALIDATOR_ENGINE" in
        SHELLCHECK)
            if ! command -v shellcheck &> /dev/null; then
                echo "FAILED: shellcheck not installed."
                return
            fi
            output=$(shellcheck "$temp_file" 2>&1) || status=$?
            ;;
        RUSTC)
            if ! command -v rustc &> /dev/null; then
                echo "FAILED: rustc not installed."
                return
            fi
            # Check syntax as library to avoid main function req
            output=$(rustc --crate-type lib --emit=metadata -o /dev/null "$temp_file" 2>&1) || status=$?
            ;;
        CARGO_AUDIT)
            # Simulation: requires full crate structure usually
            echo "Running simulated cargo audit..."
            status=0 
            ;;
        *)
            echo "FAILED: Unknown validator engine."
            return
            ;;
    esac

    if [[ $status -eq 0 ]]; then
        echo "PASSED"
    else
        echo "FAILED: $output"
    fi
}

# --- Argument Parsing ---

while [[ $# -gt 0 ]]; do
    case "$1" in
        rust-analysis|git-ops|docs|qa|shell)
            TASK_CONTEXT="$1"
            shift
            ;;
        -m|--model)
            MODEL="$2"
            shift 2
            ;;
        -t|--temp)
            TEMP="$2"
            shift 2
            ;;
        -i|--iter)
            ITERATIONS="$2"
            shift 2
            ;;
        -h|--help)
            usage
            ;;
        *)
            echo -e "${C_CRIT}Error: Unknown argument or invalid task '$1'${NC}"
            usage
            ;;
    esac
done

if [[ -z "$TASK_CONTEXT" ]]; then
    echo -e "${C_CRIT}Error: Task argument is mandatory.${NC}"
    usage
fi

# --- Dynamic Configuration ---

echo -e "${C_SYS}Initializing Context: ${TASK_CONTEXT^^}${NC}"

case "$TASK_CONTEXT" in
    rust-analysis)
        ARCHITECT_INIT="Analyze the provided Rust code structure. Focus on memory safety, ownership patterns, and borrow checker constraints."
        WORKER_INIT="Refactor the code to eliminate 'unsafe' blocks where possible and optimize lifetimes."
        CRITIC_INIT="Review for edge cases in concurrency and memory leaks. Ensure idiomatic Rust."
        LOOP_TYPE="VALIDATED"
        VALIDATOR_ENGINE="RUSTC"
        TEMP="0.2" # Low temp for precision
        ;;
    git-ops)
        ARCHITECT_INIT="Design a git workflow strategy. Focus on clean history, atomic commits, and bisect capability."
        WORKER_INIT="Generate the git commands or alias scripts."
        CRITIC_INIT="Ensure commands are non-destructive (safe) and adhere to semantic versioning."
        LOOP_TYPE="STANDARD"
        TEMP="0.5"
        ;;
    docs)
        ARCHITECT_INIT="Analyze code for missing documentation. Plan a README structure and Rustdoc comments."
        WORKER_INIT="Write clear, concise technical documentation in Markdown."
        CRITIC_INIT="Check for spelling, clarity, and missing parameter descriptions."
        LOOP_TYPE="STANDARD"
        TEMP="0.7"
        ;;
    qa)
        ARCHITECT_INIT="Identify critical paths requiring test coverage."
        WORKER_INIT="Write unit and integration tests using standard frameworks."
        CRITIC_INIT="Review test coverage and assert validity."
        LOOP_TYPE="VALIDATED"
        VALIDATOR_ENGINE="RUSTC" # Validating test syntax
        TEMP="0.3"
        ;;
    shell)
        ARCHITECT_INIT="Design a POSIX-compliant shell script logic flow."
        WORKER_INIT="Write the Bash script. Use 'set -euo pipefail'. Avoid bashisms if requesting sh compatibility."
        CRITIC_INIT="Check for logic errors and unquoted variables."
        LOOP_TYPE="VALIDATED"
        VALIDATOR_ENGINE="SHELLCHECK"
        TEMP="0.2"
        ;;
esac

# --- Execution Engine ---

CURRENT_PROMPT="Begin task: $TASK_CONTEXT"

echo -e "${C_SYS}Mode: $LOOP_TYPE | Model: $MODEL | Validator: ${VALIDATOR_ENGINE:-NONE}${NC}"
echo -e "${C_SYS}--------------------------------------------------${NC}"

for (( i=1; i<=ITERATIONS; i++ )); do
    echo -e "\n${C_SYS}--- ROUND $i ---${NC}"

    # 1. Architect Step
    ARCH_PLAN=$(query_agent "" "" "$C_ARCH" "ARCHITECT" "${ARCHITECT_INIT} Context: $CURRENT_PROMPT")
    # clear init after first run to save tokens
    ARCHITECT_INIT="" 

    # 2. Worker Step
    WORKER_OUT=$(query_agent "" "" "$C_WORK" "WORKER" "${WORKER_INIT} Execute Plan: $ARCH_PLAN")
    WORKER_INIT=""

    # 3. Validation Logic (Branching)
    if [[ "$LOOP_TYPE" == "VALIDATED" ]]; then
        echo -e "${C_VALI}VALIDATOR: Analyzing code safety ($VALIDATOR_ENGINE)...${NC}"
        
        VAL_RESULT=$(validate_code "$WORKER_OUT")

        if [[ "$VAL_RESULT" == *"FAILED"* ]]; then
            echo -e "${C_VALI}VALIDATOR: Check failed.${NC}"
            # Extract error log
            ERR_LOG="${VAL_RESULT#FAILED: }"
            echo -e "${C_CRIT}$ERR_LOG${NC}"
            
            CURRENT_PROMPT="The Validator found critical syntax/safety issues:\n$ERR_LOG\nFix the code."
            # Short-circuit loop to force Worker retry in next iteration 
            # (In a real agent system, we might loop Worker->Validator immediately, 
            # but here we pass back to Architect to re-evaluate requirements if needed)
            continue 
        else
            echo -e "${C_VALI}VALIDATOR: Check passed.${NC}"
            CURRENT_PROMPT="$WORKER_OUT" # Pass valid code to critic
        fi
    else
        # Standard loop, pass output directly
        CURRENT_PROMPT="$WORKER_OUT"
    fi

    # 4. Critic Step
    CRITIC_OUT=$(query_agent "" "" "$C_CRIT" "CRITIC" "${CRITIC_INIT} Review: $CURRENT_PROMPT")
    CRITIC_INIT=""

    if [[ "$CRITIC_OUT" == *"APPROVED"* ]]; then
        echo -e "\n${C_SYS}SYSTEM: Target achieved at Round $i.${NC}"
        echo -e "Final Output:\n$WORKER_OUT"
        exit 0
    else
        CURRENT_PROMPT="Critic Feedback: $CRITIC_OUT. Refine the solution."
    fi
done

echo -e "\n${C_CRIT}SYSTEM: Max iterations ($ITERATIONS) reached without approval.${NC}"
exit 1
