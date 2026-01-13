#!/bin/bash

# 1. Setup Colors
C1=$(printf '\033[0;32m') # Green
C2=$(printf '\033[0;34m') # Blue
NC=$(printf '\033[0m')

PIPE1=/tmp/ruchat_p1
PIPE2=/tmp/ruchat_p2
LOG1=/tmp/ruchat_log1.txt
LOG2=/tmp/ruchat_log2.txt

rm -f $PIPE1 $PIPE2
mkfifo $PIPE1
mkfifo $PIPE2

# Open persistent File Descriptors
exec 3<>$PIPE1
exec 4<>$PIPE2

# 2. Define the Relay Function
# This reads from an AI, colors it for you, and adds the trigger for the next AI.
relay() {
    local color="$1"
    local name="$2"
    local target_fd="$3"
    
    # Read line by line from the AI's output
    while IFS= read -r line; do
        # Print to your screen with color
        printf "${color}${name}: ${NC}%s\n" "$line" >&2
        printf "%s: %s\n" "$name" "$line" >> "/tmp/ruchat_log_${name// /_}.txt"
        
        # Send the line to the other AI
        echo "$line" >&$target_fd
        
        # If the AI is done (outputs an empty line or end of stream)
        # we append the trigger. Most LLMs output a blank line at the end.
        if [[ -z "$line" ]]; then
            echo "---" >&$target_fd
        fi
    done
}

echo "Starting AI instances..."

# 3. Start Instance 1 (Reads FD 3, Writes to FD 4 via relay)
stdbuf -oL target/release/ruchat <&3 | relay "$C1" "AI 1" 4 &
pid1=$!

# 4. Start Instance 2 (Reads FD 4, Writes to FD 3 via relay)
stdbuf -oL target/release/ruchat <&4 | relay "$C2" "AI 2" 3 &
pid2=$!

sleep 1

# 5. Initialization (Send model commands)
# We send the delimiter manually here to set the models
printf "!model: qwen2.5vl:latest\n---\n" >&3
printf "!model: qwen2.5vl:latest\n---\n" >&4

sleep 1

# 6. Kickstart the conversation
echo "You are an AI instructor. Briefly delegate one small task to your assistant." >&3
echo "---" >&3

echo "Conversation active. Press Ctrl+C to exit."
wait
