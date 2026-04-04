# kn-code

A high-performance, headless-first AI coding agent built in Rust.

## Features

- **Multi-provider**: Anthropic (OAuth + API key), GitHub Copilot (OAuth), OpenAI (API key + OAuth), and any OpenAI-compatible provider
- **Headless API server**: Designed for orchestration by Paperclip or any external system
- **Atomic operations**: Transactional file changes with rollback on failure
- **WASM plugins**: Sandboxed third-party extensions
- **Rust-native**: Lower memory, faster startup, true async concurrency

## Quick Start

```bash
# Install
curl -fsSL https://kn-code.ai/install | bash

# Run interactively
kn-code

# Run headless
kn-code run --format json --model anthropic/claude-sonnet-4-5 "Build a REST API"

# Start API server
kn-code serve --port 3200
```

## Configuration

See `~/.kn-code/config/kn-code.json` for configuration options.

## License

MIT
