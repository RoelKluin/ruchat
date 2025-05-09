# Ruchat

Ruchat is a command-line AI chat tool that uses `ollama` and `chroma`. It is designed for chat directly from the terminal.

## Description

Ruchat is built using Rust and provides a command-line interface for interacting with AI models.

## Installation

To install Ruchat, you need to have Rust. You can then clone the repository and build the project:

```bash
git clone https://github.com/RoelKluin/ruchat.git
cd ruchat
cargo build --release
```

## Usage

After building the project, you can run the Ruchat tool using:

```bash
./target/release/ruchat
```

You can pass various command-line arguments to customize the behavior of the chat tool. For more details, refer to the help command:

```bash
./target/release/ruchat --help
```

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

Contributions are welcome! Please fork the repository and submit a pull request for any improvements or bug fixes.

## License

This project is licensed under the MIT License.
