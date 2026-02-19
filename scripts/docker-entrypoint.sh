#!/usr/bin/env bash
set -euo pipefail

# Patch corre.toml for container environment:
#   - data_dir → /data (backed by a Docker volume)
#   - bind → 0.0.0.0:3200 (reachable from the host via port mapping)
sed -i 's|data_dir = "~/.local/share/corre"|data_dir = "/data"|' /app/corre.toml
sed -i 's|bind = "127.0.0.1:3200"|bind = "0.0.0.0:3200"|' /app/corre.toml

# Optional Tailscale integration
if [ "${TAILSCALE_ENABLED:-false}" = "true" ]; then
    echo "Starting Tailscale..."
    tailscaled --state=/data/tailscale/tailscaled.state --socket=/var/run/tailscale/tailscaled.sock &

    # Wait for the daemon socket
    for i in $(seq 1 30); do
        [ -S /var/run/tailscale/tailscaled.sock ] && break
        sleep 0.5
    done

    TS_ARGS=(--authkey="${TAILSCALE_AUTHKEY:?TAILSCALE_AUTHKEY must be set when TAILSCALE_ENABLED=true}")
    if [ -n "${TAILSCALE_LOGIN_SERVER:-}" ]; then
        TS_ARGS+=(--login-server="${TAILSCALE_LOGIN_SERVER}")
    fi

    tailscale up "${TS_ARGS[@]}"
    echo "Tailscale is up: $(tailscale ip -4)"
fi

exec "$@"
