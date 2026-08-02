./ruchat ask "Refactor error handling in src/core/orchestrator.rs" \
  --agentic '{
    "iterations": 2,
    "Architect": { "model": "qwen3-coder:latest" },
    "Worker": { "model": "qwen3-coder:latest" },
    "Validator": { "model": "qwen3.5:latest" },
    "Librarian": {
        "chroma_client": {
            "chroma_server": "http://localhost:8000",
            "max_retries": 3,
            "min_delay": 10,
            "max_delay": 100,
            "chroma_token": "default",
            "jitter": false,
            "tenant_id": "default_tenant",
            "chroma_database": "default"
        },
        "model": "qwen3.5:latest",
        "task": "Generate a Chroma search query to find relevant documentation for this task."
    }
  }' \
  --debug-sequence agent_debug/architect_only.json


./ruchat ask "Refactor error handling in src/core/orchestrator.rs" \
  --agentic '{
    "iterations": 2,
    "Architect": { "model": "qwen3-coder:latest" },
    "Worker": { "model": "qwen3-coder:latest" },
    "Validator": { "model": "qwen3.5:latest" },
    "Librarian": {
        "chroma_client": {
            "chroma_server": "http://localhost:8000",
            "max_retries": 3,
            "min_delay": 10,
            "max_delay": 100,
            "chroma_token": "default",
            "jitter": false,
            "tenant_id": "default_tenant",
            "chroma_database": "default"
        },
        "model": "qwen3.5:latest",
        "task": "Generate a Chroma search query to find relevant documentation for this task."
    }
  }' \
  --debug-sequence agent_debug/librarian_and_worker.json

./ruchat ask "Refactor error handling in src/core/orchestrator.rs" \
  --agentic '{
    "iterations": 2,
    "Architect": { "model": "qwen3-coder:latest" },
    "Worker": { "model": "qwen3-coder:latest" },
    "Validator": { "model": "qwen3.5:latest" },
    "Librarian": {
        "chroma_client": {
            "chroma_server": "http://localhost:8000",
            "max_retries": 3,
            "min_delay": 10,
            "max_delay": 100,
            "chroma_token": "default",
            "jitter": false,
            "tenant_id": "default_tenant",
            "chroma_database": "default"
        },
        "model": "qwen3.5:latest",
        "task": "Generate a Chroma search query to find relevant documentation for this task."
    }
  }' \
  --debug-sequence agent_debug/multiple_critics.json

./ruchat ask "Refactor error handling in src/core/orchestrator.rs" \
  --agentic '{
    "iterations": 2,
    "Architect": { "model": "qwen3-coder:latest" },
    "Worker": { "model": "qwen3-coder:latest" },
    "Validator": { "model": "qwen3.5:latest" },
    "Librarian": {
        "chroma_client": {
            "chroma_server": "http://localhost:8000",
            "max_retries": 3,
            "min_delay": 10,
            "max_delay": 100,
            "chroma_token": "default",
            "jitter": false,
            "tenant_id": "default_tenant",
            "chroma_database": "default"
        },
        "model": "qwen3.5:latest",
        "task": "Generate a Chroma search query to find relevant documentation for this task."
    }
  }' \
  --debug-sequence agent_debug/librarian_only.json

./ruchat ask "Refactor error handling in src/core/orchestrator.rs" \
  --agentic '{
    "iterations": 2,
    "Architect": { "model": "qwen3-coder:latest" },
    "Worker": { "model": "qwen3-coder:latest" },
    "Validator": { "model": "qwen3.5:latest" },
    "Librarian": {
        "chroma_client": {
            "chroma_server": "http://localhost:8000",
            "max_retries": 3,
            "min_delay": 10,
            "max_delay": 100,
            "chroma_token": "default",
            "jitter": false,
            "tenant_id": "default_tenant",
            "chroma_database": "default"
        },
        "model": "qwen3.5:latest",
        "task": "Generate a Chroma search query to find relevant documentation for this task."
    }
  }' \
  --debug-sequence agent_debug/summarizer.json

./ruchat ask "Refactor error handling in src/core/orchestrator.rs" \
  --agentic '{
    "iterations": 2,
    "Architect": { "model": "qwen3-coder:latest" },
    "Worker": { "model": "qwen3-coder:latest" },
    "Validator": { "model": "qwen3.5:latest" },
    "Librarian": {
        "chroma_client": {
            "chroma_server": "http://localhost:8000",
            "max_retries": 3,
            "min_delay": 10,
            "max_delay": 100,
            "chroma_token": "default",
            "jitter": false,
            "tenant_id": "default_tenant",
            "chroma_database": "default"
        },
        "model": "qwen3.5:latest",
        "task": "Generate a Chroma search query to find relevant documentation for this task."
    }
  }' \
  --debug-sequence agent_debug/critic.json

./ruchat ask "Refactor error handling in src/core/orchestrator.rs" \
  --agentic '{
    "iterations": 2,
    "Architect": { "model": "qwen3-coder:latest" },
    "Worker": { "model": "qwen3-coder:latest" },
    "Validator": { "model": "qwen3.5:latest" },
    "Librarian": {
        "chroma_client": {
            "chroma_server": "http://localhost:8000",
            "max_retries": 3,
            "min_delay": 10,
            "max_delay": 100,
            "chroma_token": "default",
            "jitter": false,
            "tenant_id": "default_tenant",
            "chroma_database": "default"
        },
        "model": "qwen3.5:latest",
        "task": "Generate a Chroma search query to find relevant documentation for this task."
    }
  }' \
  --debug-sequence agent_debug/architect_librarian_and_worker.json

./ruchat ask "Refactor error handling in src/core/orchestrator.rs" \
  --agentic '{
    "iterations": 2,
    "Architect": { "model": "qwen3-coder:latest" },
    "Worker": { "model": "qwen3-coder:latest" },
    "Validator": { "model": "qwen3.5:latest" },
    "Librarian": {
        "chroma_client": {
            "chroma_server": "http://localhost:8000",
            "max_retries": 3,
            "min_delay": 10,
            "max_delay": 100,
            "chroma_token": "default",
            "jitter": false,
            "tenant_id": "default_tenant",
            "chroma_database": "default"
        },
        "model": "qwen3.5:latest",
        "task": "Generate a Chroma search query to find relevant documentation for this task."
    }
  }' \
  --debug-sequence agent_debug/debug_sequences.md

./ruchat ask "Refactor error handling in src/core/orchestrator.rs" \
  --agentic '{
    "iterations": 2,
    "Architect": { "model": "qwen3-coder:latest" },
    "Worker": { "model": "qwen3-coder:latest" },
    "Validator": { "model": "qwen3.5:latest" },
    "Librarian": {
        "chroma_client": {
            "chroma_server": "http://localhost:8000",
            "max_retries": 3,
            "min_delay": 10,
            "max_delay": 100,
            "chroma_token": "default",
            "jitter": false,
            "tenant_id": "default_tenant",
            "chroma_database": "default"
        },
        "model": "qwen3.5:latest",
        "task": "Generate a Chroma search query to find relevant documentation for this task."
    }
  }' \
  --debug-sequence agent_debug/worker_and_validator.json

./ruchat ask "Refactor error handling in src/core/orchestrator.rs" \
  --agentic '{
    "iterations": 2,
    "Architect": { "model": "qwen3-coder:latest" },
    "Worker": { "model": "qwen3-coder:latest" },
    "Validator": { "model": "qwen3.5:latest" },
    "Librarian": {
        "chroma_client": {
            "chroma_server": "http://localhost:8000",
            "max_retries": 3,
            "min_delay": 10,
            "max_delay": 100,
            "chroma_token": "default",
            "jitter": false,
            "tenant_id": "default_tenant",
            "chroma_database": "default"
        },
        "model": "qwen3.5:latest",
        "task": "Generate a Chroma search query to find relevant documentation for this task."
    }
  }' \
  --debug-sequence agent_debug/validator_only.json

./ruchat ask "Refactor error handling in src/core/orchestrator.rs" \
  --agentic '{
    "iterations": 2,
    "Architect": { "model": "qwen3-coder:latest" },
    "Worker": { "model": "qwen3-coder:latest" },
    "Validator": { "model": "qwen3.5:latest" },
    "Librarian": {
        "chroma_client": {
            "chroma_server": "http://localhost:8000",
            "max_retries": 3,
            "min_delay": 10,
            "max_delay": 100,
            "chroma_token": "default",
            "jitter": false,
            "tenant_id": "default_tenant",
            "chroma_database": "default"
        },
        "model": "qwen3.5:latest",
        "task": "Generate a Chroma search query to find relevant documentation for this task."
    }
  }' \
  --debug-sequence agent_debug/architect_librarian_worker_validator.json

./ruchat ask "Refactor error handling in src/core/orchestrator.rs" \
  --agentic '{
    "iterations": 2,
    "Architect": { "model": "qwen3-coder:latest" },
    "Worker": { "model": "qwen3-coder:latest" },
    "Validator": { "model": "qwen3.5:latest" },
    "Librarian": {
        "chroma_client": {
            "chroma_server": "http://localhost:8000",
            "max_retries": 3,
            "min_delay": 10,
            "max_delay": 100,
            "chroma_token": "default",
            "jitter": false,
            "tenant_id": "default_tenant",
            "chroma_database": "default"
        },
        "model": "qwen3.5:latest",
        "task": "Generate a Chroma search query to find relevant documentation for this task."
    }
  }' \
  --debug-sequence agent_debug/worker_and_validator_rejection.jso
