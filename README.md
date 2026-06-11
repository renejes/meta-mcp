# Meta-MCP

A Tauri 2 desktop app that acts as a central **MCP proxy**: it aggregates
multiple MCP servers behind a single local endpoint and exposes only the tools
of the currently active servers to Claude Desktop (or any other MCP client).

Claude Desktop only ever needs one entry in its `claude_desktop_config.json`:

```json
{
  "mcpServers": {
    "meta-mcp": {
      "url": "http://localhost:3663/sse"
    }
  }
}
```

Everything else — which servers run, which tools are visible — is controlled
from the app's UI.

## Features

- **Two client transports on one server** (port `3663`):
  - Legacy HTTP+SSE — `GET /sse` + `POST /message`
  - Streamable HTTP — `POST /mcp` (+ `GET`/`DELETE /mcp`)
- **Two backend transports**: spawns `stdio` MCP servers as child processes and
  connects to remote `sse` MCP servers.
- **Tool aggregation** with name prefixing: `{server-slug}__{tool}` (e.g.
  `github__create_issue`). The slug is derived from the server's display name;
  collisions get a numeric suffix. The router strips the prefix before
  forwarding and splits on the *first* `__` only.
- **Profiles**: save a set of active servers and switch between them. A profile
  overrides the per-server toggles; clearing it restores the manual toggles.
- **Import** from an existing `claude_desktop_config.json` (with a preview;
  servers with a name that already exists are skipped).
- **Live status**: a `proxy-status-changed` event drives the header status dot
  (running / starting / port-in-use error) and the per-server connection state.

## Develop / Run

```bash
pnpm install
pnpm tauri dev      # or: cargo tauri dev
```

## Build

```bash
pnpm tauri build    # or: cargo tauri build
```

## Data

Configuration is a single JSON file in the app data directory:

- macOS: `~/Library/Application Support/com.metamcp.desktop/config.json`

## Architecture

```
src-tauri/src/
├── main.rs                 # binary entry → meta_mcp_lib::run()
├── lib.rs                  # Tauri setup: state, server bind, status, cleanup
├── commands.rs             # Tauri IPC commands
├── config/
│   ├── mod.rs              # load / save config.json
│   └── types.rs            # Config, ServerEntry, Profile, status types
└── proxy/
    ├── mod.rs              # ProxyState: active-set, reconcile, cache, status
    ├── server.rs           # axum: legacy SSE + Streamable HTTP, JSON-RPC core
    ├── aggregator.rs       # build prefixed tool list + slug→id map (cached)
    ├── router.rs           # tools/call → correct backend (strip prefix)
    ├── backend.rs          # Backend enum over the two transports
    ├── child.rs            # stdio child-process MCP client
    └── sse_client.rs       # remote SSE MCP client

src/                        # Svelte 5 frontend (plain Svelte + Vite)
├── App.svelte              # orchestrator: state, events, import flow
├── app.css                 # dark theme
└── lib/
    ├── api.ts, types.ts    # typed invoke() wrappers
    ├── ProfileBar.svelte
    ├── ServerList.svelte
    ├── ToolList.svelte
    └── AddServerModal.svelte
```

## Scope (intentionally not built)

No tool editing, no auth on the meta endpoint, no cloud sync, no auto-update,
no MCP resources/prompts (tools only), no tool logging.
# meta-map
