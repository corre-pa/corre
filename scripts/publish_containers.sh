#!/usr/bin/env bash
set -euo pipefail

REGISTRY="ghcr.io/corre-pa"
SERVICES=("corre-core" "corre-news" "corre-registry")

# Extract version from workspace Cargo.toml
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
VERSION=$(grep '^version' "$REPO_ROOT/Cargo.toml" | head -1 | sed 's/.*"\(.*\)".*/\1/')

if [[ -z "$VERSION" ]]; then
    echo "ERROR: could not extract version from Cargo.toml"
    exit 1
fi

echo "Publishing containers for v${VERSION}"
echo "Registry: ${REGISTRY}"
echo ""

# Build all images via docker compose
echo "==> Building images..."
docker compose -f "$REPO_ROOT/docker-compose.yml" build

# Tag and push each service
for service in "${SERVICES[@]}"; do
    image="${REGISTRY}/${service}"
    compose_image="${image}:latest"

    echo ""
    echo "==> Pushing ${service}..."

    # Tag with version
    docker tag "$compose_image" "${image}:${VERSION}"

    # Push both version and latest tags
    docker push "${image}:${VERSION}"
    docker push "${image}:latest"

    echo "    pushed ${image}:${VERSION}"
    echo "    pushed ${image}:latest"
done

echo ""
echo "Done. All containers published as v${VERSION} + :latest"
