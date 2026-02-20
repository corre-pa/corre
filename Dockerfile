# ── Stage 1: Build ────────────────────────────────────────────────────────────
FROM rust:1.85-bookworm AS builder

WORKDIR /build

# Copy workspace manifests first for better layer caching
COPY Cargo.toml Cargo.lock ./
COPY crates/corre-core/Cargo.toml crates/corre-core/Cargo.toml
COPY crates/corre-mcp/Cargo.toml crates/corre-mcp/Cargo.toml
COPY crates/corre-llm/Cargo.toml crates/corre-llm/Cargo.toml
COPY crates/corre-news/Cargo.toml crates/corre-news/Cargo.toml
COPY crates/corre-capabilities/Cargo.toml crates/corre-capabilities/Cargo.toml
COPY crates/corre-cli/Cargo.toml crates/corre-cli/Cargo.toml

# Stub out lib.rs / main.rs so cargo can resolve deps and cache them
RUN mkdir -p crates/corre-core/src && echo "" > crates/corre-core/src/lib.rs && \
    mkdir -p crates/corre-mcp/src && echo "" > crates/corre-mcp/src/lib.rs && \
    mkdir -p crates/corre-llm/src && echo "" > crates/corre-llm/src/lib.rs && \
    mkdir -p crates/corre-news/src && echo "" > crates/corre-news/src/lib.rs && \
    mkdir -p crates/corre-capabilities/src && echo "" > crates/corre-capabilities/src/lib.rs && \
    mkdir -p crates/corre-cli/src && echo "fn main() {}" > crates/corre-cli/src/main.rs

RUN cargo build --release 2>/dev/null || true

# Now copy the real source and rebuild
COPY . .
RUN touch crates/*/src/*.rs && cargo build --release

# ── Stage 2: Runtime ──────────────────────────────────────────────────────────
FROM debian:bookworm-slim

# Install runtime dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates \
        curl \
        nodejs \
        npm \
    && rm -rf /var/lib/apt/lists/*

# Install Tailscale (present but dormant unless TAILSCALE_ENABLED=true)
RUN curl -fsSL https://pkgs.tailscale.com/stable/debian/bookworm.noarmor.gpg \
        -o /usr/share/keyrings/tailscale-archive-keyring.gpg && \
    curl -fsSL https://pkgs.tailscale.com/stable/debian/bookworm.tailscale-keyring.list \
        -o /etc/apt/sources.list.d/tailscale.list && \
    apt-get update && apt-get install -y --no-install-recommends tailscale && \
    rm -rf /var/lib/apt/lists/*

# Create app directory and data volume mount point
WORKDIR /app
RUN mkdir -p /data

# Copy binaries from builder
COPY --from=builder /build/target/release/corre /app/corre

# Copy application files
COPY corre.toml /app/corre.toml
COPY static/ /app/static/
COPY config/ /app/config/

# Copy and set up entrypoint
COPY scripts/docker-entrypoint.sh /app/docker-entrypoint.sh
RUN chmod +x /app/docker-entrypoint.sh

EXPOSE 3200

ENTRYPOINT ["/app/docker-entrypoint.sh"]
CMD ["/app/corre", "run"]
