# ── Stage 1: Build daily-brief binary ──────────────────────────────────────────
FROM rust:1.91-bookworm AS builder

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

RUN cargo build --release -p daily-brief 2>/dev/null || true

# Copy the real source and rebuild
COPY . .
RUN touch crates/*/src/*.rs && cargo build --release -p daily-brief

# Compute SHA256 of the binary and resolve the capability manifest.
# Build the V2 registry manifest by resolving template placeholders and merging
# the resolved capability entry into the capabilities array.
RUN apt-get update && apt-get install -y --no-install-recommends jq && rm -rf /var/lib/apt/lists/* && \
    DAILY_BRIEF_SHA256=$(sha256sum /build/target/release/daily-brief | awk '{print $1}') && \
    VERSION=$(grep '^version' /build/Cargo.toml | head -1 | sed 's/.*"\(.*\)".*/\1/') && \
    sed -e "s/\${VERSION}/${VERSION}/g" \
        -e "s/\"\${SHA256:daily-brief}\"/{ \"linux-x86_64\": \"${DAILY_BRIEF_SHA256}\" }/g" \
        /build/corre-registry/capabilities/registered/daily-brief.json > /tmp/daily-brief-resolved.json && \
    jq --slurpfile cap /tmp/daily-brief-resolved.json \
       '.version = 2 | .capabilities = $cap' \
       /build/corre-registry/mcp-registry.json > /tmp/registry-v2.json

# ── Stage 2: Serve via nginx ──────────────────────────────────────────────────
FROM nginx:alpine

COPY corre-registry/nginx.conf /etc/nginx/nginx.conf
COPY corre-registry/registered/ /usr/share/nginx/html/mcp/registered/
COPY corre-registry/capabilities/ /usr/share/nginx/html/capabilities/

# Copy the resolved V2 manifest from the builder
COPY --from=builder /tmp/registry-v2.json /usr/share/nginx/html/mcp/registry.json

# Copy the daily-brief binary for download
RUN mkdir -p /usr/share/nginx/html/capabilities/bin
COPY --from=builder /build/target/release/daily-brief /usr/share/nginx/html/capabilities/bin/daily-brief-linux-x86_64

EXPOSE 5580

CMD ["nginx", "-g", "daemon off;"]
