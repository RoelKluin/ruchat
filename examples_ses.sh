# Search for context and pipe it into the LLM
ruchat chroma-search -c documentation -q "How do I use the manager?" --limit 3 -j | \
ruchat pipe "Based on the provided JSON context from our database, explain how to use the manager."


ruchat chroma-get -c legal_docs -i "doc_001,doc_042" -j | \
ruchat ask "Summarize these specific documents for a lawyer."

ruchat chroma-get -c legal_docs -i "doc_001,doc_042" -j | \
ruchat ask "Summarize these specific documents for a lawyer."

#--
# 1. Ask the AI what it needs to look up
SEARCH_QUERY=$(ruchat ask "I want to know about rust memory safety. What should I search in the DB?" -o text)

# 2. Use that output to query Chroma, then pipe back to the AI for the final answer
ruchat chroma-query -c rust_notes -q "$SEARCH_QUERY" -n 2 -j | \
#--
ruchat pipe "Here is the data found for '$SEARCH_QUERY'. Please synthesize an answer."

#Subcommand	                Status	        Assessment/Suggestion
#chroma-query	            Critical	    This is your primary RAG driver. Ensure it can output plain text (just the documents) as well as JSON. Piping raw JSON into an LLM works, but it consumes more tokens.
#pipe	                    Great	        Very useful for agent chaining. It seems to handle multi-part markdown which is excellent for keeping "History" and "New Context" separate.
#manager	                Needs Work?	    Currently, it seems to rely on a config file. To make it "agentic" in the sense of running its own pipeline, consider adding a way to pass a "Task" via CLI that the manager then decomposes.
#func / func-struct	High    Potential	    These are the keys to a true agent. If an agent can call func, it can decide to call chroma-search itself.

#--
# Ask AI to generate a search command based on a goal
CMD=$(ruchat ask "Generate a ruchat chroma-search command to find info about 'lifetimes' in the 'rust' collection. Output ONLY the command." -o text)

#--
# Execute the AI's suggested command and pipe it back for a summary
eval "$CMD" | ruchat pipe "Summarize these findings."

# 1. Fetch relevant docs from Chroma (text only)
# 2. Pipe them into 'ask' with an agentic configuration
ruchat chroma-query -c rust_docs -q "ownership and borrowing" -n 5 --fields doc | \
ruchat ask --agentic '{
    "iterations": 2,
    "Architect": {
        "model": "qwen2.5:latest",
        "task": "Based on the provided documentation context, create a lesson plan."
    },
    "Worker": {
        "model": "qwen2.5:latest",
        "task": "Write the technical examples for the lesson plan."
    },
    "Critic": {
        "model": "qwen2.5:latest",
        "task": "Verify if the code examples follow Rust best practices. Reply APPROVED or feedback.",
        "approval_signal": "APPROVED"
    }
}' "Create a Rust tutorial using the piped context."

#--
# Ask an agent to generate the search command
SEARCH_CMD=$(ruchat ask "I need to know about Chroma integration. Write the ruchat chroma-query command for this." -o text)

# Execute and feed back to the agentic workflow
eval "$SEARCH_CMD" | ruchat ask --agentic "$(cat my_team_config.json)" "Final Analysis"

# Proposed shortcut for RAG
ruchat ask --rag-collection "my_docs" --rag-top-k 3 "How do I do X?"

# Get context and pipe it as the goal/initial prompt
ruchat chroma-query -c codebase -q "error handling" --fields doc | \
ruchat ask --agentic '{
    "iterations": 2,
    "history_limit": 1500,
    "Architect": {
        "role": "ARCHITECT",
        "model": "qwen2.5:latest",
        "task": "Extract the core logic from the provided documentation and plan a fix."
    },
    "Worker": {
        "role": "WORKER",
        "model": "qwen2.5:latest",
        "task": "Write the Rust code implementing the fix."
    },
    "Critic": {
        "role": "CRITIC",
        "model": "qwen2.5:latest",
        "approval_signal": "APPROVED"
    },
    "Summarizer": {
        "role": "SUMMARIZER",
        "model": "qwen2.5:latest",
        "task": "Compress the history while preserving the code implementation and the rejection reasons."
    }
}'

#--
# Get the context manually and pass it to the team
ruchat chroma-search -c dev_docs -q "auth flow" --fields doc | \
ruchat ask --agentic "$(cat team.json)" "Implement a login handler."

#--
# The JSON defines a "Librarian" with a task to generate search terms
ruchat ask --agentic '{
    "Librarian": { "role": "LIBRARIAN", "task": "Create a semantic search query for Chroma." },
    "Architect": { "role": "ARCHITECT", "task": "Use the DOCUMENTS field to plan." },
    ...
}' "Fix the token expiration bug."
