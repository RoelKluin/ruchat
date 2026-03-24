# Requirements:
- Rust (latest stable version)
- Git
- Ollama
- Chroma (For embedding generation)

# Ruchat installation Guide

To install Ruchat, ensure you have Rust installed on your system. You can install Rust by following the instructions at [rust-lang.org](https://www.rust-lang.org/tools/install). Then follow these steps:

1. Clone the repository:
   ```bash
   git clone https://github.com/RoelKluin/ruchat.git
   cd ruchat
   ```

2. Build the project:
   ```bash
   cargo build --release
   ```

3. Run ollama server:
   ```bash
   OLLAMA_HOST=localhost:11434 CUDA_VISIBLE_DEVICES=0 ollama serve
    ```

4. run chroma server:
   ```bash
   # docker pull chromadb/chroma
   docker run -p 8000:8000 -e chroma_server_auth_credentials_provider="chromadb.auth.token.tokenconfigserverauthcredentialsprovider" -e chroma_server_auth_provider="chromadb.auth.token.tokenauthserverprovider" -e chroma_server_auth_token_transport_header="${CHROMA_AUTH_HEADER}" -e chroma_server_auth_credentials="${CHROMA_AUTH_CREDENTIALS}" -v ~/chroma_storage/:/chroma/chroma chromadb/chroma
   ```


