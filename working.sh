
#./ruchat similarity --model "all-minilm:l6-v2" --query "how can we embed an answer from ask.rs?"

#./ruchat chroma-metadata -c repo_src-all-minilm_l6-v2 --where-metadata '{"file": "src/main.rs"}'
#./ruchat chroma-metadata -c repo_src-all-minilm_l6-v2 --where-metadata '{"language": "Rust"}'

 ./ruchat chroma-query -m all-minilm:l6-v2 -c repo_src-all-minilm_l6-v2 -q "ChromaResponse" --fields doc --format markdown



#./ruchat chroma-get --model "qwen2.5vl:latest" --collection repo_src-all-minilm_l6-v2  --prompt 'what argument(s) does the ask function in Ask require?'

./ruchat chroma-ls

chroma browse --db default repo_src-all-minilm_l6-v2 --host http://localhost:8000
chroma browse --db default repo_hist-all-minilm_l6-v2 --host http://localhost:8000




repo_src-all-minilm_l6-v2

./ruchat pipe \
  --team-model qwen2.5-coder:14b \
  --validator-model qwen2.5-coder:14b \
  --critic Security --critic Performance \
  --collection repo_src-all-minilm_l6-v2 \
  --iterations 5 \
  "Refactor the borrow-ordering conflict between self.client and self.librarian in Orchestrator::new"

./ruchat pipe --agentic '{
  "iterations": 5,
  "Architect": {
    "model": "qwen2.5-coder:14b",
    "temperature": 0.2,
    "status_msg": "Architect is planning..."
  },
  "Worker": {
    "model": "qwen2.5-coder:14b",
    "temperature": 0.4,
    "status_msg": "Worker is implementing...",
    "embed_args": {
      "collection_config": {"collection": "repo_src-all-minilm_l6-v2"},
      "client_config": {"chroma_server": "http://localhost:8000"}
    }
  },
  "Validator": {
    "model": "qwen2.5-coder:14b",
    "temperature": 0.0,
    "status_msg": "Validator is checking..."
  },
  "Summarizer": {
    "model": "qwen2.5-coder:7b",
    "temperature": 0.0
  },
  "Critics": [
    {
      "model": "qwen2.5-coder:7b",
      "task": "Review the implementation strictly for correctness and safety",
      "approval_signal": "APPROVED",
      "status_msg": "Correctness Critic reviewing..."
    },
    {
      "model": "qwen2.5-coder:7b",
      "task": "Review the implementation strictly for performance regressions",
      "approval_signal": "APPROVED",
      "status_msg": "Performance Critic reviewing..."
    }
  ],
  "Librarian": {
    "model": "all-minilm:l6-v2",
    "chroma_client": "{\"chroma_server\":\"http://localhost:8000\"}",
    "db_config_path": "db_config.json",
    "status_msg": "Searching knowledge base..."
  }
}' "Refactor the borrow-ordering conflict between self.client and self.librarian in Orchestrator::new"

./ruchat pipe   --team-model qwen2.5-coder:14b   --validator-model qwen2.5-coder:14b   --critic Security --critic Performance   --collection repo_src-all-minilm_l6-v2   --iterations 5  "Refactor the clap options to be more user friendly, hide existing advanced options by default. Require a flag to show advanced options."

./ruchat pipe   --team-model qwen2.5-coder:14b   --validator-model qwen2.5-coder:14b --librarian-embed-model all-minilm:l6-v2  --critic Security --critic Performance   --collection repo_src-all-minilm_l6-v2   --iterations 5  "Check the last 5 commits touching src/core/orchestrator.rs with git_log, then apply a patch fixing the Critic_N index parsing bug in debug_stage_machine, then memorize a one-line summary of the fix."
