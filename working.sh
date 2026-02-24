
./ruchat similarity --model "all-minilm:l6-v2" --query "how can we embed an answer from ask.rs?"

./ruchat chroma-metadata -c repo_src-all-minilm_l6-v2 --where-metadata '{"file": "src/main.rs"}'
./ruchat chroma-metadata -c repo_src-all-minilm_l6-v2 --where-metadata '{"language": "Rust"}'

./ruchat query --model 'qwen3:latest' --model "all-minilm:l6-v2" \
--query "Contents of file: src/ollama/ask.rs" \
--prompt "how can we embed an answer from ask.rs?"


./ruchat get --model "qwen2.5vl:latest" --collection repo_src-all-minilm_l6-v2  --prompt 'what argument(s) does the ask function in Ask require?'

chroma browse repo_src-all-minilm_l6-v2 --host http://localhost:8000
chroma browse repo_hist-all-minilm_l6-v2 --host http://localhost:8000


