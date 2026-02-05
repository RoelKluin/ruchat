
./ruchat similarity --model "all-minilm:l6-v2" --query "how can we embed an answer from ask.rs?"

./ruchat chroma-metadata -c repo_src-all-minilm_l6-v2 --where-metadata '{"file": "src/main.rs"}
./ruchat chroma-metadata -c repo_src-all-minilm_l6-v2 --where-metadata '{"language": "Rust"}

./ruchat query --model 'qwen3:latest' --model "all-minilm:l6-v2" \
--query "Contents of file: src/ollama/ask.rs" \
--prompt "how can we embed an answer from ask.rs?"
