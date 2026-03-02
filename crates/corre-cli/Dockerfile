# ── Stage 1: Build ────────────────────────────────────────────────────────────
FROM rust:1.91-trixie AS builder

WORKDIR /build

# Copy workspace manifests first for better layer caching
COPY Cargo.toml Cargo.lock ./
COPY crates/corre-sdk/Cargo.toml crates/corre-sdk/Cargo.toml
COPY crates/corre-core/Cargo.toml crates/corre-core/Cargo.toml
COPY crates/corre-mcp/Cargo.toml crates/corre-mcp/Cargo.toml
COPY crates/corre-llm/Cargo.toml crates/corre-llm/Cargo.toml
COPY crates/corre-news/Cargo.toml crates/corre-news/Cargo.toml
COPY crates/corre-capabilities/Cargo.toml crates/corre-capabilities/Cargo.toml
COPY crates/corre-safety/Cargo.toml crates/corre-safety/Cargo.toml
COPY crates/corre-dashboard/Cargo.toml crates/corre-dashboard/Cargo.toml
COPY crates/corre-registry/Cargo.toml crates/corre-registry/Cargo.toml
COPY crates/corre-cli/Cargo.toml crates/corre-cli/Cargo.toml
COPY crates/mcp-smtp/Cargo.toml crates/mcp-smtp/Cargo.toml
COPY crates/mcp-telegram/Cargo.toml crates/mcp-telegram/Cargo.toml
COPY crates/daily-brief/Cargo.toml crates/daily-brief/Cargo.toml

# Stub out lib.rs / main.rs so cargo can resolve deps and cache them
RUN for c in corre-sdk corre-core corre-mcp corre-llm \
             corre-capabilities corre-safety corre-dashboard corre-registry; do \
        mkdir -p "crates/$c/src" && echo "" > "crates/$c/src/lib.rs"; \
    done && \
    mkdir -p crates/corre-news/src && \
    echo "" > crates/corre-news/src/lib.rs && \
    echo "fn main() {}" > crates/corre-news/src/main.rs && \
    for c in corre-cli mcp-smtp mcp-telegram daily-brief; do \
        mkdir -p "crates/$c/src" && echo "fn main() {}" > "crates/$c/src/main.rs"; \
    done

RUN cargo build --release -p corre-cli 2>/dev/null || true

# Now copy the real source and rebuild
COPY . .
RUN touch crates/*/src/*.rs && cargo build --release -p corre-cli

# ── Stage 2: Core runtime (scheduler + capabilities + dashboard) ─────────────
FROM debian:trixie-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates \
        curl \
        nodejs \
        npm \
        gosu \
    && rm -rf /var/lib/apt/lists/*

# Install Tailscale (dormant unless TAILSCALE_ENABLED=true)
RUN curl -fsSL https://pkgs.tailscale.com/stable/debian/trixie.noarmor.gpg \
        -o /usr/share/keyrings/tailscale-archive-keyring.gpg && \
    curl -fsSL https://pkgs.tailscale.com/stable/debian/trixie.tailscale-keyring.list \
        -o /etc/apt/sources.list.d/tailscale.list && \
    apt-get update && apt-get install -y --no-install-recommends tailscale && \
    rm -rf /var/lib/apt/lists/*

RUN groupadd -g 1000 corre && useradd -u 1000 -g corre -m corre

WORKDIR /app
RUN mkdir -p /data && chown corre:corre /app /data

COPY --from=builder /build/target/release/corre /app/corre
COPY corre.toml /app/corre.toml
COPY static/ /app/static/
COPY config/ /app/config/
COPY scripts/docker-entrypoint.sh /app/docker-entrypoint.sh
RUN chmod +x /app/docker-entrypoint.sh

EXPOSE 5500
ENV RUST_LOG=${RUST_LOG:-info}
ENTRYPOINT ["/app/docker-entrypoint.sh"]
CMD ["/app/corre", "-c", "/data/corre.toml", "run", "--dashboard-bind", "0.0.0.0:5500"]
