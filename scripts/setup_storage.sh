# Prepare RUCHAT workspace
mkdir -p ${HOME}/.ruchat/{sessions,logs,bin}
export PERSIST_DIR="${HOME}/.ruchat/sessions"
export SESSION_FILE="/tmp/ruchat_session_state.json"
export HISTORY_FILE="/tmp/ruchat_history.log"

# Initialize Session Schema
if [[ ! -f "$SESSION_FILE" ]]; then
    echo '{
      "history": [], 
      "last_task": "", 
      "total_tokens": 0, 
      "total_cost": 0.0, 
      "debate_logs": []
    }' > "$SESSION_FILE"
fi
