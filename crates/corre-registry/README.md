# corre-registry

A curated MCP server registry and installer for Corre. Browse, install, test, and
manage MCP servers from a single JSON manifest served at any URL.

## Overview

`corre-registry` provides:

- **Registry client** -- fetches a remote JSON manifest of available MCP servers with
  in-memory TTL caching.
- **Installer** -- installs MCP servers via npx, pip/uvx, or direct binary download
  (with SHA-256 verification).
- **Connection tester** -- spins up a temporary MCP server, calls `list_tools`, and
  reports the result.
- **Dependency checker** -- verifies that required host tools (node, npx, pip, uvx,
  etc.) are available on `PATH`.

The crate is consumed by `corre-dashboard` (HTTP API + web UI) and `corre-cli`
(wiring). End users interact with it through the dashboard's **MCP Manager** and
**MCP Store** windows.

## Registry manifest format

The registry is a single JSON file. It can live anywhere -- GitHub Pages, S3, a local
file server, etc. Point Corre at it via `[registry]` in `corre.toml`.

```json
{
  "version": 1,
  "updated_at": "2026-02-24T12:00:00Z",
  "servers": [
    {
      "id": "brave-search",
      "name": "Brave Search",
      "description": "Web search via the Brave Search API",
      "version": "1.1.0",
      "install": {
        "method": "npx",
        "package": "@brave/brave-search-mcp-server",
        "command": "npx",
        "args": ["-y", "@brave/brave-search-mcp-server"]
      },
      "config": [
        {
          "name": "BRAVE_API_KEY",
          "description": "Brave Search API key",
          "required": true
        }
      ],
      "dependencies": ["node", "npx"],
      "homepage": "https://github.com/brave/brave-search-mcp-server",
      "tags": ["search", "web"],
      "verified": true
    }
  ]
}
```

### Install methods

The `install.method` field selects the installation strategy:

| Method   | Fields                                                                        | Behaviour                                                                 |
|----------|-------------------------------------------------------------------------------|---------------------------------------------------------------------------|
| `npx`    | `package`, `command`, `args`                                                  | No pre-install step; npx auto-downloads the package on first run.         |
| `pip`    | `package`, `command`, `args`                                                  | No pre-install step; assumes uvx/pipx handles on-demand installation.     |
| `binary` | `download_url_template`, `binary_name`, `sha256`, `command`, `args`           | Downloads the binary, verifies SHA-256, writes to `{data_dir}/bin/`, chmod +x. |

Binary URL templates support these placeholders: `{version}`, `{platform}` (linux /
darwin), `{arch}` (x86_64 / aarch64), `{bin_dir}`.

### Entry fields

| Field          | Type       | Required | Description                                          |
|----------------|------------|----------|------------------------------------------------------|
| `id`           | string     | yes      | Unique identifier, used as the MCP server name in config. |
| `name`         | string     | yes      | Human-readable display name.                         |
| `description`  | string     | yes      | Short description shown in the store UI.             |
| `version`      | string     | yes      | Semver version of the MCP server.                    |
| `install`      | object     | yes      | Installation method (see above).                     |
| `config`       | array      | no       | Environment variables the server needs.              |
| `dependencies` | array      | no       | Host tools that must be on PATH (e.g. `["node", "npx"]`). |
| `homepage`     | string     | no       | URL to the project's homepage or repository.         |
| `tags`         | array      | no       | Searchable tags (e.g. `["search", "web"]`).          |
| `verified`     | bool       | no       | Whether the entry has been verified by the registry maintainer. |

## Configuration

Add a `[registry]` section to `corre.toml`:

```toml
[registry]
url = "https://example.com/mcp-registry.json"
cache_ttl_secs = 3600
```

| Field            | Default | Description                                              |
|------------------|---------|----------------------------------------------------------|
| `url`            | `""`    | URL of the registry manifest. Left empty to disable.     |
| `cache_ttl_secs` | `3600`  | How long (seconds) to cache the manifest in memory.      |

If the `[registry]` section is omitted entirely, registry features are disabled (the
store UI will show an error when opened).

## Dashboard UI

### MCP Manager

Visible by default. Shows a table of all MCP servers currently in `corre.toml`:

| Column  | Description                                                    |
|---------|----------------------------------------------------------------|
| Name    | Server name (key in `[mcp.servers.*]`).                        |
| Command | The full command + args used to start the server.              |
| Source  | `registry` if installed from the store, `manual` otherwise.   |
| Tools   | Number of tools discovered (populated after clicking **Test**).|
| Actions | **Test** (start server, list tools, shut down) and **Remove**. |

### MCP Store

Initially minimized. Open it from the taskbar or Start menu. Displays a card grid of
all servers in the registry manifest.

Each card shows the server name, description, version, tags, install method badge
(npx / pip / binary), and a verified checkmark if applicable. Servers already in your
config show an "Installed" badge instead of an Install button.

The toolbar provides a text search (filters by name, description, and tags) and a
Refresh button to force-reload the manifest.

### Install flow

1. Click **Install** on a store card.
2. A modal opens showing:
   - **Dependencies** -- each required host tool with a green check or red X and its
     detected version.
   - **Environment variables** -- one text input per `env_var` entry, pre-filled with
     the variable name. Enter the *name of the env var* that holds the secret (not the
     secret itself), matching Corre's convention.
3. Click **Install**. The server config is written to `corre.toml` and appears in the
   MCP Manager.

## Dashboard API

All endpoints require the `editor_token` configured in `[news]`. Pass it as a
`Bearer` header or `?token=` query parameter.

### Registry

| Route                       | Method | Description                                      |
|-----------------------------|--------|--------------------------------------------------|
| `/api/registry/catalog`     | GET    | Full cached manifest.                            |
| `/api/registry/search?q=…`  | GET    | Search entries by name, description, or tags.    |
| `/api/registry/refresh`     | POST   | Force-refresh the manifest cache.                |

### MCP management

| Route                       | Method | Description                                      |
|-----------------------------|--------|--------------------------------------------------|
| `/api/mcp/installed`        | GET    | List installed MCP servers from config.          |
| `/api/mcp/install`          | POST   | Install a server. Body: `{ "id": "…", "env_values": { … } }` |
| `/api/mcp/uninstall/{name}` | POST   | Remove a server from config (and binary if applicable). |
| `/api/mcp/test/{name}`      | POST   | Start the server, call `list_tools`, shut down.  |
| `/api/mcp/deps/{id}`        | GET    | Check host dependencies for a registry entry.    |

### Install request body

```json
{
  "id": "brave-search",
  "env_values": {
    "BRAVE_API_KEY": "BRAVE_API_KEY"
  }
}
```

The values in `env_values` are env var **names**, not secrets. They are written into
the `[mcp.servers.*.env]` table in `corre.toml` and resolved from the host environment
at server start time.

## Crate modules

| Module         | Public types / functions                                        |
|----------------|-----------------------------------------------------------------|
| `manifest`     | `RegistryManifest`, `RegistryEntry`, `InstallMethod`, `EnvVarSpec` |
| `client`       | `RegistryClient`, `RegistryError`                               |
| `installer`    | `McpInstaller`, `InstallError`                                  |
| `tester`       | `test_mcp_server(name, config) -> Result<Vec<String>, String>`  |
| `deps`         | `check_deps(deps) -> HashMap<String, DepStatus>`, `DepStatus`  |

## Hosting your own registry

Serve a JSON file matching the manifest format at any stable URL:

```sh
# quick local test
python3 -m http.server 8000 --directory ./registry/

# in corre.toml
[registry]
url = "http://localhost:8000/registry.json"
```

For production, host the file on GitHub Pages, S3, or any static file host. The
registry client fetches it with a plain GET request and expects `Content-Type:
application/json`.
