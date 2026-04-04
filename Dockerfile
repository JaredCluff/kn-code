FROM rust:1.85-slim AS builder
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY crates/ crates/
RUN cargo build --release -p kn-code-cli
RUN strip target/release/kn-code

FROM debian:bookworm-slim
RUN groupadd -r kn-code && useradd -r -g kn-code -d /home/kn-code -s /sbin/nologin kn-code
RUN mkdir -p /home/kn-code && chown -R kn-code:kn-code /home/kn-code

RUN apt-get update && apt-get install -y \
    ca-certificates \
    ripgrep \
    fd-find \
    jq \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/kn-code /usr/local/bin/kn-code

USER kn-code
WORKDIR /home/kn-code

EXPOSE 3200

HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD curl -f http://localhost:3200/health || exit 1

ENTRYPOINT ["kn-code"]
CMD ["serve"]
