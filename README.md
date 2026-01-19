# Ruchat

Ruchat is a command-line AI chat tool that uses `ollama` and `chroma`. It allows you to interact with AI models directly from the terminal.

## Description

Ruchat provides a simple and powerful way to engage in conversations or perform various operations with AI models. The project is designed to be ex tensible and flexible, offering multiple subcommands for different use cases.

## Installation

To install Ruchat and its requirements, see [INSTALL.md].
```

You can use the following Docker command to run a Chroma database:

```bash
docker pull chromadb/chroma
# with auth using tokens and persistent storage:
docker run -p 8000:8000 \
               -e chroma_server_auth_credentials_provider="chromadb.auth.token.tokenconfigserverauthcredentialsprovider" \
               -e chroma_server_auth_provider="chromadb.auth.token.tokenauthserverprovider" \
               -e chroma_server_auth_token_transport_header="$(sed -n 1p ~/.chroma_creds.txt)" \
               -e chroma_server_auth_credentials="$(sed -n 2p ~/.chroma_creds.txt)" \
               -v ~/chroma_storage/:/chroma/chroma chromadb/chroma
```

## Usage

After building the project, you can run Ruchat the terminal:

```bash
./target/release/ruchat
```

For more information on availavle subcommands and options, you can use the help command:


### Subcommands

- **Ask**: Interact with the AI model by asking questions.
- **Pipe**: Pipe input through the chat tool.
- **Chat**: Engage in a chat session with the AI model.
- **Ls**: List available models.
- **Rm**: Remove a specified model.
- **Pull**: Pull a model from a repository.
- **Func**: Execute a function with the AI model.
- **FuncStruct**: Execute a structured function with the AI model.
- **Embed**: Generate embeddings for input data.
- **Query**: Perform a query operation.
- **Similarity**: Conduct a similarity search.
- **ChromaLs**: List chroma-related information.

## Contributing

Contributions are welcome! If you want to contribute, please fork the repository and submit a pull request. Any improvements or bug fixes are great ly appreciated!

## License

This project is licensed under the MIT License.
