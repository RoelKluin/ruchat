#!/bin/bash
 [ -n "$DEBUG" ] && set -x

stty -echoctl # Disable ^C echo to keep output clean

cleanup() {
    query_agent 3 4 "$C_ARCH" "ARCHITECT" '!done'
    query_agent 5 6 "$C_WORK" "WORKER" '!done'
    query_agent 7 8 "$C_CRIT" "CRITIC" '!done'
    rm -f "/tmp/ruchat_architect_in" "/tmp/ruchat_architect_out"
    rm -f "/tmp/ruchat_worker_in" "/tmp/ruchat_worker_out"
    rm -f "/tmp/ruchat_critic_in" "/tmp/ruchat_critic_out"
}
trap cleanup EXIT

# 1. Configuration & Colors
C_ARCH=$(printf '\033[1;32m') # Bold Green
C_WORK=$(printf '\033[1;34m') # Bold Blue
C_CRIT=$(printf '\033[1;31m') # Bold Red
NC=$(printf '\033[0m')

# Create pipes for 3 agents
AGENTS=("architect" "worker" "critic")
for agent in "${AGENTS[@]}"; do
    rm -f "/tmp/ruchat_${agent}_in" "/tmp/ruchat_${agent}_out"
    mkfifo "/tmp/ruchat_${agent}_in" "/tmp/ruchat_${agent}_out"
done

MODELS=("qwen2.5:latest" "deepseek-coder-v2:latest" "mistral-nemo:latest")

# 2. Start Instances in the background
echo -e "${C_ARCH}SYSTEM: Starting agent instances...${NC}"
# Each instance reads from its 'in' pipe and writes to its 'out' pipe
target/release/ruchat pipe -m ${MODELS[0]} -o '{"temperature": 0.0}' < "/tmp/ruchat_${AGENTS[0]}_in" > "/tmp/ruchat_${AGENTS[0]}_out" &  # Architect
target/release/ruchat pipe -m ${MODELS[1]} -o '{"temperature": 0.7}' < "/tmp/ruchat_${AGENTS[1]}_in" > "/tmp/ruchat_${AGENTS[1]}_out" &  # Worker
target/release/ruchat pipe -m ${MODELS[2]} -o '{"temperature": 0.0}' < "/tmp/ruchat_${AGENTS[2]}_in" > "/tmp/ruchat_${AGENTS[2]}_out" &  # Critic

# Open File Descriptors for the Orchestrator to control them
exec 3>"/tmp/ruchat_architect_in"  4<"/tmp/ruchat_architect_out"
exec 5>"/tmp/ruchat_worker_in"     6<"/tmp/ruchat_worker_out"
exec 7>"/tmp/ruchat_critic_in"     8<"/tmp/ruchat_critic_out"

# Function to query an agent and wait for the "---" response
query_agent() {
    local in_fd=$1   # FD to send prompt TO the agent
    local out_fd=$2  # FD to read response FROM the agent
    local color=$3
    local name=$4
    local prompt=$5

    # Send prompt to agent
    printf "%s\n---\n" "$prompt" >&"$in_fd"

    # Read response until we see the "---" delimiter from ruchat
    local line
    local full_response=""
    while IFS= read -u "$out_fd" -r line; do
        if [[ "$line" == "---" ]]; then break; fi
        printf "${color}${name}:${NC} %s\n" "$line" 1>&2
        full_response+="$line"$'\n'
    done
    echo "$full_response"
}

# 4. The Orchestration Loop
USER_GOAL="Create a plan for a 3-course vegetarian dinner and provide the recipe for the main course."
CURRENT_PROMPT="User Goal: $USER_GOAL\nArchitect, please break this down into a specific task for the Worker.
---"

while true; do
    # STEP 1: ARCHITECT PLANS
    ARCH_PLAN=$(query_agent 3 4 "$C_ARCH" "ARCHITECT" "$CURRENT_PROMPT")

    # STEP 2: WORKER EXECUTES
    WORKER_OUTPUT=$(query_agent 5 6 "$C_WORK" "WORKER" "The Architect has assigned you this task: $ARCH_PLAN
---")

    # STEP 3: CRITIC REVIEWS
    CRITIC_FEEDBACK=$(query_agent 7 8 "$C_CRIT" "CRITIC" "Review the Worker's output for quality and accuracy. If it is good, say 'APPROVED'. If not, provide feedback: $WORKER_OUTPUT
---")

    # STEP 4: DECISION LOGIC
    if [[ "$CRITIC_FEEDBACK" == *"APPROVED"* ]]; then
        echo -e "\n${C_ARCH}SYSTEM: Task successfully completed and approved.${NC}"
        break
    else
        echo -e "\n${C_ARCH}SYSTEM: Critic rejected work. Routing back for revision...${NC}"
        CURRENT_PROMPT="The Critic rejected the previous attempt with this feedback: $CRITIC_FEEDBACK. Architect, please revise the instructions.
---"
    fi
done
