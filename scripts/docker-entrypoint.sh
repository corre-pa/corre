#!/usr/bin/env bash
set -euo pipefail

# The entrypoint runs as root so it can chown the bind mount and start
# Tailscale. The main process is exec'd as the unprivileged corre user.

# Ensure the bind-mounted data dir is owned by corre
chown -R corre:corre /data

# Seed default corre.toml into the data volume if it doesn't exist yet.
# The bundled /app/corre.toml is a read-only template; the writable copy
# lives on the persistent volume so the dashboard can save settings.
if [ -f /app/corre.toml ] && [ ! -e /data/corre.toml ]; then
    cp /app/corre.toml /data/corre.toml
    chown corre:corre /data/corre.toml
    echo "Seeded default corre.toml into /data"
fi

# Seed per-capability config files into the data volume.
if [ -d /app/config ] && [ -d /data ]; then
    mkdir -p /data/config
    chown corre:corre /data/config
    for f in /app/config/*; do
        dest="/data/config/$(basename "$f")"
        if [ ! -e "$dest" ]; then
            cp "$f" "$dest"
            chown corre:corre "$dest"
            echo "Seeded default config: $(basename "$f")"
        fi
    done
fi

# Optional Tailscale integration (runs as root)
if [ "${TAILSCALE_ENABLED:-false}" = "true" ]; then
    echo "Starting Tailscale..."
    mkdir -p /data/tailscale
    tailscaled --state=/data/tailscale/tailscaled.state --socket=/var/run/tailscale/tailscaled.sock &

    # Wait for the daemon socket (up to 15 seconds)
    for i in $(seq 1 30); do
        [ -S /var/run/tailscale/tailscaled.sock ] && break
        sleep 0.5
    done

    TS_ARGS=(--authkey="${TAILSCALE_AUTHKEY:?TAILSCALE_AUTHKEY must be set when TAILSCALE_ENABLED=true}")
    [ -n "${TAILSCALE_LOGIN_SERVER:-}" ] && TS_ARGS+=(--login-server="${TAILSCALE_LOGIN_SERVER}")
    [ -n "${TS_HOSTNAME:-}" ] && TS_ARGS+=(--hostname="${TS_HOSTNAME}")

    tailscale up "${TS_ARGS[@]}"
    echo "Tailscale is up: $(tailscale ip -4)"

    # Dashboard on default HTTPS port, newspaper on 8443
    tailscale serve --bg --https=443 http://localhost:5500
    tailscale serve --bg --https=8443 http://corre-news:5510
fi

# Drop to the corre user for the main process
exec gosu corre "$@"
