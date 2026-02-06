#!/bin/bash
 [ -n "$DEBUG" ] && set -x

stty -echoctl # Disable ^C echo to keep output clean

# 1. Colors & Agent Definitions
C_ARCH=$(printf '\033[1;32m') # Green
C_WORK=$(printf '\033[1;34m') # Blue
C_VALI=$(printf '\033[1;33m') # Yellow (Validator)
C_CRIT=$(printf '\033[1;31m') # Red
C_SUMM=$(printf '\033[1;35m') # Magenta
NC=$(printf '\033[0m')

AGENTS=("architect" "worker" "validator" "critic" "summarizer")

cleanup() {
    query_agent 3 4 "$C_ARCH" "ARCHITECT" '!done'
    query_agent 5 6 "$C_WORK" "WORKER" '!done'
    query_agent 7 8 "$C_VALI" "VALIDATOR" '!done'
    query_agent 9 10 "$C_CRIT" "CRITIC" '!done'
    query_agent 11 12 "$C_SUMM" "SUMMARIZER" '!done'
    rm -f "/tmp/ruchat_architect_in" "/tmp/ruchat_architect_out"
    rm -f "/tmp/ruchat_worker_in" "/tmp/ruchat_worker_out"
    rm -f "/tmp/ruchat_validator_in" "/tmp/ruchat_validator_out"
    rm -f "/tmp/ruchat_critic_in" "/tmp/ruchat_critic_out"
    rm -f "/tmp/ruchat_summarizer_in" "/tmp/ruchat_summarizer_out"
}
trap cleanup EXIT

HISTORY_FILE="/tmp/ruchat_full_history.log"
> "$HISTORY_FILE"

# Setup Pipes and Start Instances (Standard setup)
for agent in "${AGENTS[@]}"; do
    rm -f "/tmp/ruchat_${agent}_in" "/tmp/ruchat_${agent}_out"
    mkfifo "/tmp/ruchat_${agent}_in" "/tmp/ruchat_${agent}_out"
done

MODELS=("qwen3:4b" "deepseek-coder:latest" "qwen3:4b" "qwen3:4b" "mistral-nemo:latest")

# Start Instances
target/release/ruchat pipe -m ${MODELS[0]} -o '{"temperature": 0.0}'  < "/tmp/ruchat_architect_in" > "/tmp/ruchat_architect_out" &
target/release/ruchat pipe -m ${MODELS[1]} -o '{"temperature": 0.7}'  < "/tmp/ruchat_worker_in"    > "/tmp/ruchat_worker_out" &
target/release/ruchat pipe -m ${MODELS[2]} -o '{"temperature": 0.0}'  < "/tmp/ruchat_validator_in"    > "/tmp/ruchat_validator_out" &
target/release/ruchat pipe -m ${MODELS[3]} -o '{"temperature": 0.0}'  < "/tmp/ruchat_critic_in"    > "/tmp/ruchat_critic_out" &

# Open FDs
exec 3>"/tmp/ruchat_architect_in"  4<"/tmp/ruchat_architect_out"
exec 5>"/tmp/ruchat_worker_in"     6<"/tmp/ruchat_worker_out"
exec 7>"/tmp/ruchat_validator_in"     8<"/tmp/ruchat_validator_out"
exec 9>"/tmp/ruchat_critic_in"     10<"/tmp/ruchat_critic_out"

# --- HELPER: Code Extraction & Validation ---
validate_code() {
    local text="$1"
    # Extract content between triple backticks
    local code=$(echo "$text" | sed -n '/^```\(bash\|sh\)\?$/,/^```$/p' | sed '1d;$d')
    
    if [[ -z "$code" ]]; then
        echo "VALIDATION_FAILED: No code block found."
        return 1
    fi

    # Run shellcheck on the extracted code
    echo "$code" > /tmp/test_script.sh
    local lint_output=$(shellcheck /tmp/test_script.sh 2>&1)
    
    if [[ -z "$lint_output" ]]; then
        echo "VALIDATION_SUCCESS"
    else
        echo "VALIDATION_FAILED: Shellcheck found errors:"
        echo "$lint_output"
    fi
}

query_agent() {
    local in_fd=$1; local out_fd=$2; local color=$3; local name=$4; local prompt=$5
    printf "%s\n---\n" "$prompt" >&"$in_fd"
    local line; local full_response=""
    while IFS= read -u "$out_fd" -r line; do
        [[ "$line" == "---" ]] && break
        printf "${color}${name}:${NC} %s\n" "$line" 1>&2
        full_response+="$line"$'\n'
    done
    echo -e "### ${name} ###\n${full_response}\n" >> "$HISTORY_FILE"
    echo "$full_response"
}

# send initial system prompts to each agent
ARCHITECT_INIT="You are a Senior Software Architect. Your job is to turn vague goals into technical specifications. Be concise and use bullet points. "
WORKER_INIT="You are an expert Linux Developer. Provide only clean, commented code without excessive conversational filler. "
VALIDATOR_INIT="You are a shellcheck expert. Explain the errors and valid warnings. Do not be polite; be accurate. "
CRITIC_INIT="You are a pedantic QA Engineer. Look for security flaws, edge cases, and syntax errors. Do not be polite; be accurate. "

# 3. The Orchestration Loop
USER_GOAL="Write a bash script that finds all .log files in /var/log larger than 100MB and lists them."
CURRENT_PROMPT="Goal: $USER_GOAL. Architect, create the technical requirements."

for i in {1..3}; do
    echo -e "\n${C_VALI}--- ROUND $i ---${NC}"
    
    ARCH_PLAN=$(query_agent 3 4 "$C_ARCH" "ARCHITECT" "${ARCHITECT_INIT} $CURRENT_PROMPT")
    ARCHITECT_INIT=
    WORKER_OUT=$(query_agent 5 6 "$C_WORK" "WORKER" "${WORKER_INIT} Requirements: $ARCH_PLAN. Output code in \` \` \` blocks.")
    WORKER_INIT=

    # VALIDATION STEP (Linter)
    echo -e "${C_VALI}VALIDATOR: Running Shellcheck...${NC}"
    VAL_RESULT=$(validate_code "$WORKER_OUT")
    
    if [[ "$VAL_RESULT" == *"FAILED"* ]]; then
        echo -e "${C_VALI}VALIDATOR: Code failed linting. Sending back to Worker.${NC}"
        CURRENT_PROMPT="The Validator found these issues in your script. Please fix them:\n$VAL_RESULT"
        continue
    fi

    # CRITIC STEP (Human-like Review)
    CRITIC_OUT=$(query_agent 7 8 "$C_CRIT" "CRITIC" "${CRITIC_INIT} The code passed the linter. Review it for logic and safety: $WORKER_OUT")
    CRITIC_INIT=

    if [[ "$CRITIC_OUT" == *"APPROVED"* ]]; then
        echo -e "\n${C_ARCH}SYSTEM: Target achieved.${NC}"
        break
    else
        CURRENT_PROMPT="Critic feedback: $CRITIC_OUT. Architect, adjust requirements."
    fi
done

query_agent 3 4 "$C_ARCH" "ARCHITECT" '!done'
query_agent 5 6 "$C_WORK" "WORKER" '!done'
query_agent 7 8 "$C_CRIT" "CRITIC" '!done'
rm -f "/tmp/ruchat_architect_in" "/tmp/ruchat_architect_out"
rm -f "/tmp/ruchat_worker_in" "/tmp/ruchat_worker_out"
rm -f "/tmp/ruchat_critic_in" "/tmp/ruchat_critic_out"

target/release/ruchat pipe -m "${MODELS[4]}" -o '{"temperature": 0.0}'  < "/tmp/ruchat_summarizer_in"> "/tmp/ruchat_summarizer_out" &
exec 11>"/tmp/ruchat_summarizer_in" 12<"/tmp/ruchat_summarizer_out"

# 4. Final Summarization
echo -e "\n${C_SUMM}SYSTEM: Generating final documentation...${NC}"
query_agent 11 12 "$C_SUMM" "SUMMARIZER" "You are a Technical Writer. Your job is to create clean, professional documentation based on a chat log. Focus on the final result. Summarize this into a README.md: $(cat "$HISTORY_FILE")"
query_agent 11 12 "$C_SUMM" "SUMMARIZER" '!done'
rm -f "/tmp/ruchat_summarizer_in" "/tmp/ruchat_summarizer_out"
