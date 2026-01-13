#!/bin/bash
 [ -n "$DEBUG" ] && set -x

stty -echoctl # Disable ^C echo to keep output clean

cleanup() {
    query_agent 3 4 "$C_ARCH" "ARCHITECT" '!done'
    query_agent 5 6 "$C_WORK" "WORKER" '!done'
    query_agent 7 8 "$C_CRIT" "CRITIC" '!done'
    query_agent 9 10 "$C_SUMM" "SUMMARIZER" '!done'
    rm -f "/tmp/ruchat_architect_in" "/tmp/ruchat_architect_out"
    rm -f "/tmp/ruchat_worker_in" "/tmp/ruchat_worker_out"
    rm -f "/tmp/ruchat_critic_in" "/tmp/ruchat_critic_out"
    rm -f "/tmp/ruchat_summarizer_in" "/tmp/ruchat_summarizer_out"
}
trap cleanup EXIT

# 1. Colors & Agent Definitions
C_ARCH=$(printf '\033[1;32m') # Green (Architect)
C_WORK=$(printf '\033[1;34m') # Blue (Worker)
C_CRIT=$(printf '\033[1;31m') # Red (Critic)
C_SUMM=$(printf '\033[1;35m') # Magenta (Summarizer)
NC=$(printf '\033[0m')

AGENTS=("architect" "worker" "critic" "summarizer")
HISTORY_FILE="/tmp/ruchat_full_history.log"
> "$HISTORY_FILE" # Clear history

# Setup Pipes
for agent in "${AGENTS[@]}"; do
    rm -f "/tmp/ruchat_${agent}_in" "/tmp/ruchat_${agent}_out"
    mkfifo "/tmp/ruchat_${agent}_in" "/tmp/ruchat_${agent}_out"
done

MODELS=("qwen2.5:latest" "deepseek-coder-v2:latest" "mistral-nemo:latest")

# Start Instances
target/release/ruchat pipe -m ${MODELS[0]} -o '{"temperature": 0.0}'  < "/tmp/ruchat_architect_in" > "/tmp/ruchat_architect_out" &
target/release/ruchat pipe -m ${MODELS[0]} -o '{"temperature": 0.7}'  < "/tmp/ruchat_worker_in"    > "/tmp/ruchat_worker_out" &
target/release/ruchat pipe -m ${MODELS[0]} -o '{"temperature": 0.0}'  < "/tmp/ruchat_critic_in"    > "/tmp/ruchat_critic_out" &
target/release/ruchat pipe -m ${MODELS[0]} -o '{"temperature": 0.0}'  < "/tmp/ruchat_summarizer_in"> "/tmp/ruchat_summarizer_out" &

# Open FDs
exec 3>"/tmp/ruchat_architect_in"  4<"/tmp/ruchat_architect_out"
exec 5>"/tmp/ruchat_worker_in"     6<"/tmp/ruchat_worker_out"
exec 7>"/tmp/ruchat_critic_in"     8<"/tmp/ruchat_critic_out"
exec 9>"/tmp/ruchat_summarizer_in" 10<"/tmp/ruchat_summarizer_out"

# Updated Query Function with History Logging
query_agent() {
    local in_fd=$1; local out_fd=$2; local color=$3; local name=$4; local prompt=$5
    
    printf "%s\n---\n" "$prompt" >&"$in_fd"
    
    local line; local full_response=""
    while IFS= read -u "$out_fd" -r line; do
        [[ "$line" == "---" ]] && break
        printf "${color}${name}:${NC} %s\n" "$line" 1>&2
        full_response+="$line"$'\n'
    done
    
    # Log to global history
    echo -e "### ${name} ###\n${full_response}\n" >> "$HISTORY_FILE"
    echo "$full_response"
}

# send initial system prompts to each agent
query_agent 3 4 "$C_ARCH" "ARCHITECT" "You are a Senior Software Architect. Your job is to turn vague goals into technical specifications. Be concise and use bullet points.
---"
query_agent 5 6 "$C_WORK" "WORKER" "You are an expert Linux Developer. Provide only clean, commented code without excessive conversational filler.
---"
query_agent 7 8 "$C_CRIT" "CRITIC" "You are a pedantic QA Engineer. Look for security flaws, edge cases, and syntax errors. Do not be polite; be accurate.
---"
query_agent 9 10 "$C_SUMM" "SUMMARIZER" "You are a Technical Writer. Your job is to create clean, professional documentation based on a chat log. Focus on the final result.
---"


# 3. Execution
USER_GOAL="Design a simple Bash script that monitors CPU usage and alerts if it exceeds 90%."
echo "USER GOAL: $USER_GOAL" >> "$HISTORY_FILE"
CURRENT_PROMPT="User Goal: $USER_GOAL. Architect, create the technical requirements."

for i in {1..3}; do # Limit to 3 attempts for safety
    ARCH_PLAN=$(query_agent 3 4 "$C_ARCH" "ARCHITECT" "$CURRENT_PROMPT
---")
    WORKER_OUT=$(query_agent 5 6 "$C_WORK" "WORKER" "Requirements: $ARCH_PLAN
---")
    CRITIC_OUT=$(query_agent 7 8 "$C_CRIT" "CRITIC" "Review this code. If perfect, say 'APPROVED'. Otherwise, list issues: $WORKER_OUT
---")

    if [[ "$CRITIC_OUT" == *"APPROVED"* ]]; then
        echo -e "\n${C_ARCH}SYSTEM: Target achieved.${NC}"
        break
    else
        CURRENT_PROMPT="The Critic found issues: $CRITIC_OUT. Architect, refine the requirements."
    fi
done

# 4. Final Summarization
echo -e "\n${C_SUMM}SYSTEM: Generating final report...${NC}\n"
FINAL_HISTORY=$(cat "$HISTORY_FILE")
query_agent 9 10 "$C_SUMM" "SUMMARIZER" "Summarize the entire interaction below into a final technical report. Include the final code and a 'Lessons Learned' section: $FINAL_HISTORY"

echo "Process Complete."
