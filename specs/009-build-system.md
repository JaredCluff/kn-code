# SPEC 009: Build System and Project Structure

## 1. Build System

### 1.1 Cargo Workspace

```toml
# Cargo.toml (root)
[workspace]
members = [
    "crates/kn-code",           # Main library
    "crates/kn-code-cli",       # CLI binary
    "crates/kn-code-server",    # HTTP API server
    "crates/kn-code-providers", # LLM provider implementations
    "crates/kn-code-tools",     # Tool system
    "crates/kn-code-auth",      # Authentication subsystem
    "crates/kn-code-session",   # Session management
    "crates/kn-code-atomic",    # Atomic file operations
    "crates/kn-code-plugins",   # WASM plugin runtime
    "crates/kn-code-permissions", # Permission system
    "crates/kn-code-config",    # Configuration
    "crates/kn-code-sdk",       # Plugin SDK
    "crates/kn-code-test-utils", # Test utilities
]
resolver = "2"

[workspace.package]
version = "0.1.0"
edition = "2024"
license = "MIT"
repository = "https://github.com/knowledge-nexus/kn-code"

[workspace.dependencies]
# Async runtime
tokio = { version = "1", features = ["full"] }
tokio-util = "0.7"
tokio-stream = "0.1"

# HTTP
axum = { version = "0.8", features = ["ws", "macros", "http2"] }
reqwest = { version = "0.12", features = ["json", "stream", "gzip", "brotli"] }
hyper = "1"
hyper-util = "0.1"

# Serialization
serde = { version = "1", features = ["derive"] }
serde_json = "1"
serde_yaml = "0.9"

# WASM
wasmtime = "25"
wasmtime-wasi = "25"
wit-bindgen = "0.32"

# CLI
clap = { version = "4", features = ["derive", "env"] }
ratatui = "0.28"
crossterm = "0.28"

# Crypto and security
ring = "0.17"
sha2 = "0.10"
hmac = "0.12"
age = "0.10"
secrecy = "0.8"

# Keychain
keyring = "3"

# JWT
jsonwebtoken = "9"

# Filesystem
notify = "6"
globset = "0.4"
ignore = "0.4"
tempfile = "3"
fs4 = "0.12"

# Logging
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["json", "env-filter"] }

# Time
chrono = { version = "0.4", features = ["serde"] }

# UUID
uuid = { version = "1", features = ["v4", "serde"] }

# URL
url = "2"

# Markdown conversion
html2md = "0.2"
comrak = "0.31"

# Git
git2 = "0.19"

# Diff
similar = "2"

# JSON Schema
schemars = "0.8"

# Error handling
thiserror = "2"
anyhow = "1"

# Testing
mockito = "1"
wiremock = "0.6"
```

### 1.2 Build Profiles

```toml
[profile.release]
opt-level = 3
lto = "thin"
codegen-units = 1
strip = true
panic = "abort"

[profile.dev]
opt-level = 0
debug = true

[profile.bench]
inherits = "release"
```

### 1.3 Build Commands

```bash
# Build everything
cargo build --workspace

# Build release binary
cargo build --release -p kn-code-cli

# Run tests
cargo test --workspace

# Run integration tests
cargo test --test integration

# Run benchmarks
cargo bench

# Check all crates
cargo check --workspace

# Format
cargo fmt --workspace

# Lint
cargo clippy --workspace -- -D warnings

# Build single binary
cargo build --release -p kn-code-cli --features standalone

# Build with TUI
cargo build --release -p kn-code-cli --features tui

# Build with all features
cargo build --release -p kn-code-cli --all-features
```

### 1.4 Feature Flags

```toml
# crates/kn-code-cli/Cargo.toml
[features]
default = ["server", "atomic"]
server = ["kn-code-server"]
tui = ["dep:ratatui", "dep:crossterm"]
atomic = ["kn-code-atomic"]
plugins = ["kn-code-plugins"]
sandbox = ["dep:firejail"]
all = ["server", "tui", "atomic", "plugins", "sandbox"]
```

## 2. CI/CD

### 2.1 GitHub Actions

```yaml
# .github/workflows/ci.yml
name: CI
on:
  push:
    branches: [main]
  pull_request:
    branches: [main]

jobs:
  check:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - run: cargo check --workspace

  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - run: cargo test --workspace

  clippy:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: clippy
      - run: cargo clippy --workspace -- -D warnings

  fmt:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: rustfmt
      - run: cargo fmt --workspace -- --check

  build:
    strategy:
      matrix:
        os: [ubuntu-latest, macos-latest]
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - run: cargo build --release -p kn-code-cli
      - uses: actions/upload-artifact@v4
        with:
          name: kn-code-${{ matrix.os }}
          path: target/release/kn-code
```

### 2.2 Release Process

```bash
# Bump version
cargo set-version -p kn-code-cli 0.2.0
cargo set-version -p kn-code-server 0.2.0
# ... all crates

# Build release binaries
cargo build --release -p kn-code-cli

# Create GitHub release
gh release create v0.2.0 \
    target/release/kn-code \
    --title "v0.2.0" \
    --generate-notes
```

## 3. Install Script

```bash
#!/usr/bin/env bash
# install.sh

set -euo pipefail

REPO="knowledge-nexus/kn-code"
INSTALL_DIR="${INSTALL_DIR:-/usr/local/bin}"

# Detect platform
OS="$(uname -s | tr '[:upper:]' '[:lower:]')"
ARCH="$(uname -m)"
case "$ARCH" in
    x86_64) ARCH="x86_64" ;;
    aarch64|arm64) ARCH="aarch64" ;;
    *) echo "Unsupported architecture: $ARCH"; exit 1 ;;
esac

# Get latest release
LATEST=$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" | jq -r .tag_name)
BINARY="kn-code-${OS}-${ARCH}"
URL="https://github.com/${REPO}/releases/download/${LATEST}/${BINARY}"

echo "Installing kn-code ${LATEST} for ${OS}/${ARCH}..."

# Download and install
curl -fsSL "$URL" -o /tmp/kn-code
chmod +x /tmp/kn-code
sudo mv /tmp/kn-code "${INSTALL_DIR}/kn-code"

echo "Installed to $(which kn-code)"
kn-code --version
```

## 4. Docker

```dockerfile
FROM rust:1.85-slim AS builder
WORKDIR /app
COPY . .
RUN cargo build --release -p kn-code-cli
RUN strip target/release/kn-code

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y \
    git \
    curl \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/kn-code /usr/local/bin/kn-code

# Pre-install common tools
RUN apt-get update && apt-get install -y \
    ripgrep \
    fd-find \
    jq \
    && rm -rf /var/lib/apt/lists/*

EXPOSE 3200
ENTRYPOINT ["kn-code"]
CMD ["serve"]
```

## 5. Complete Directory Structure

```
kn-code/
├── Cargo.toml                    # Workspace root
├── Cargo.lock
├── rust-toolchain.toml           # Rust version pinning
├── rustfmt.toml                  # Formatting config
├── clippy.toml                   # Clippy config
├── .github/
│   └── workflows/
│       ├── ci.yml                # CI pipeline
│       ├── release.yml           # Release automation
│       └── docker.yml            # Docker image build
├── crates/
│   ├── kn-code/                  # Main library crate
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs            # Re-exports all sub-crates
│   │       └── prelude.rs        # Common imports
│   ├── kn-code-cli/              # CLI binary
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── main.rs           # Entrypoint (clap)
│   │       ├── commands/
│   │       │   ├── mod.rs
│   │       │   ├── serve.rs      # `kn-code serve`
│   │       │   ├── run.rs        # `kn-code run`
│   │       │   ├── models.rs     # `kn-code models`
│   │       │   ├── auth.rs       # `kn-code auth`
│   │       │   └── session.rs    # `kn-code session`
│   │       └── tui/              # Optional TUI
│   │           ├── mod.rs
│   │           └── app.rs
│   ├── kn-code-server/           # HTTP API server
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── server.rs         # Axum server setup
│   │       ├── routes/
│   │       │   ├── mod.rs
│   │       │   ├── health.rs
│   │       │   ├── run.rs
│   │       │   ├── sessions.rs
│   │       │   ├── providers.rs
│   │       │   └── models.rs
│   │       ├── ws.rs             # WebSocket handler
│   │       ├── middleware/
│   │       │   ├── mod.rs
│   │       │   ├── auth.rs
│   │       │   ├── logging.rs
│   │       │   └── rate_limit.rs
│   │       └── sse.rs            # Server-sent events
│   ├── kn-code-providers/        # LLM providers
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── traits.rs         # Provider trait
│   │       ├── anthropic.rs
│   │       ├── github_copilot.rs
│   │       ├── openai.rs
│   │       └── compatible.rs
│   ├── kn-code-auth/             # Auth subsystem
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── oauth.rs
│   │       ├── api_key.rs
│   │       ├── token_store.rs
│   │       └── pkce.rs
│   ├── kn-code-tools/            # Tool system
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── traits.rs         # Tool trait
│   │       ├── registry.rs
│   │       ├── bash.rs
│   │       ├── file_read.rs
│   │       ├── file_write.rs
│   │       ├── file_edit.rs
│   │       ├── glob.rs
│   │       ├── grep.rs
│   │       ├── web_fetch.rs
│   │       ├── web_search.rs
│   │       ├── todo_write.rs
│   │       ├── agent.rs
│   │       ├── mcp.rs
│   │       ├── skill.rs
│   │       └── ask_user.rs
│   ├── kn-code-permissions/      # Permission system
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── rules.rs
│   │       ├── bash_perms.rs
│   │       └── sandbox.rs
│   ├── kn-code-session/          # Session management
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── manager.rs
│   │       ├── store.rs
│   │       ├── compact.rs
│   │       └── messages.rs
│   ├── kn-code-atomic/           # Atomic operations
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── transaction.rs
│   │       ├── journal.rs
│   │       └── staging.rs
│   ├── kn-code-plugins/          # WASM plugin runtime
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── runtime.rs
│   │       ├── host.rs
│   │       ├── sandbox.rs
│   │       └── lifecycle.rs
│   ├── kn-code-config/           # Configuration
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── settings.rs
│   │       ├── migrations.rs
│   │       └── env.rs
│   ├── kn-code-sdk/              # Plugin SDK
│   │   ├── Cargo.toml
│   │   └── src/
│   │       └── lib.rs            # Macros and helpers for plugin authors
│   └── kn-code-test-utils/       # Test utilities
│       ├── Cargo.toml
│       └── src/
│           └── lib.rs
├── tests/
│   └── integration/
│       ├── mod.rs
│       ├── test_anthropic.rs
│       ├── test_openai.rs
│       ├── test_tools.rs
│       ├── test_atomic.rs
│       ├── test_session.rs
│       └── test_server.rs
├── benches/
│   ├── tool_execution.rs
│   ├── file_operations.rs
│   └── jsonl_parsing.rs
├── skills/                       # Built-in skills
│   ├── commit/
│   │   └── SKILL.md
│   ├── review/
│   │   └── SKILL.md
│   └── init/
│       └── SKILL.md
├── scripts/
│   ├── install.sh                # Install script
│   ├── release.sh                # Release automation
│   └── bench.sh                  # Benchmark runner
├── doc/
│   ├── ARCHITECTURE.md
│   ├── CONTRIBUTING.md
│   └── RELEASE.md
├── specs/                        # These spec files
│   ├── 001-overview-and-architecture.md
│   ├── 002-provider-and-auth.md
│   ├── 003-tool-system.md
│   ├── 004-headless-api-server.md
│   ├── 005-atomic-operations.md
│   ├── 006-wasm-plugin-system.md
│   ├── 007-session-management.md
│   ├── 008-paperclip-adapter-compatibility.md
│   └── 009-build-system.md
├── Dockerfile
├── docker-compose.yml
├── .gitignore
├── .editorconfig
└── README.md
```

## 6. rust-toolchain.toml

```toml
[toolchain]
channel = "1.85"
components = ["rustfmt", "clippy"]
profile = "minimal"
```

## 7. rustfmt.toml

```toml
edition = "2024"
max_width = 100
tab_spaces = 4
newline_style = "Unix"
use_field_init_shorthand = true
merge_derives = true
```

## 8. clippy.toml

```toml
# Disallow common footguns
disallowed-types = [
    { path = "std::sync::Mutex", reason = "Use tokio::sync::Mutex for async code" },
]
disallowed-methods = [
    { path = "std::fs::read_to_string", reason = "Use tokio::fs::read_to_string" },
    { path = "std::fs::write", reason = "Use tokio::fs::write" },
    { path = "std::process::Command::output", reason = "Use tokio::process::Command" },
]
```

## 9. .gitignore

```
/target/
**/*.rs.bk
*.pdb
Cargo.lock
.env
.kn-code/
*.wasm
```

## 10. README.md

```markdown
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
```
