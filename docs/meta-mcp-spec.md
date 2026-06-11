# Meta-MCP — Build Spec

## Ziel

Baue eine Tauri 2 Desktop App namens **Meta-MCP**. Die App fungiert als zentraler MCP-Proxy: Sie aggregiert mehrere MCP-Server hinter einem einzigen HTTP/SSE-Endpunkt und exponiert Claude Desktop (oder jedem anderen MCP-Client) nur die Tools der gerade aktiven Server.

Claude Desktop trägt nur einen einzigen Eintrag in seiner `claude_desktop_config.json`:

```json
{
  "mcpServers": {
    "meta-mcp": {
      "url": "http://localhost:3663/sse"
    }
  }
}
```

Alles andere — welche Server laufen, welche Tools sichtbar sind — steuert der Nutzer über das UI der App.

---

## Stack

- **Framework:** Tauri 2 (Rust Backend + WebView Frontend)
- **Frontend:** Svelte 5 (kein SvelteKit, plain Svelte mit Vite)
- **Rust:** stable toolchain, Tokio async runtime
- **MCP-Protokoll:** Selbst implementiert (kein externes Rust-MCP-SDK nötig — das Protokoll ist schlank genug)
- **Persistenz:** Eine einzige JSON-Datei im App-Datenverzeichnis (`app_data_dir/config.json`)
- **Kein Docker, kein externe Datenbank, keine Cloud-Abhängigkeiten**

---

## Projektstruktur

```
meta-mcp/
├── src-tauri/
│   ├── src/
│   │   ├── main.rs              # Tauri App-Entry, setup
│   │   ├── proxy/
│   │   │   ├── mod.rs           # Proxy-State, öffentliche API
│   │   │   ├── server.rs        # HTTP/SSE-Server (axum)
│   │   │   ├── aggregator.rs    # Tool-Liste zusammenbauen
│   │   │   ├── router.rs        # Tool-Call → richtiger Backend-Server
│   │   │   └── child.rs         # stdio-Prozess-Management
│   │   ├── config/
│   │   │   ├── mod.rs           # Config laden/speichern
│   │   │   └── types.rs         # Datenstrukturen
│   │   └── commands.rs          # Tauri IPC Commands
│   └── Cargo.toml
├── src/
│   ├── App.svelte
│   ├── lib/
│   │   ├── ServerList.svelte    # Server-Übersicht mit Toggles
│   │   ├── ToolList.svelte      # Alle Tools aller aktiven Server
│   │   ├── ProfileBar.svelte    # Profil-Selektor
│   │   └── AddServerModal.svelte
│   └── main.ts
├── package.json
└── vite.config.ts
```

---

## Datenmodell (`config.json`)

```typescript
interface Config {
  servers: ServerEntry[];
  profiles: Profile[];
  active_profile: string | null; // null = kein Profil, manuelle Toggles gelten
}

interface ServerEntry {
  id: string;           // uuid, generiert beim Erstellen
  name: string;         // Anzeigename
  transport: "stdio" | "sse";

  // für stdio:
  command?: string;     // z.B. "npx"
  args?: string[];      // z.B. ["@modelcontextprotocol/server-github"]
  env?: Record<string, string>;

  // für sse:
  url?: string;         // z.B. "http://localhost:8080/sse"

  active: boolean;      // manueller Toggle
}

interface Profile {
  id: string;
  name: string;
  active_server_ids: string[]; // welche Server in diesem Profil aktiv sind
}
```

Rust-seitig dieselben Strukturen mit `serde::{Serialize, Deserialize}`.

---

## Rust Backend

### Abhängigkeiten (`Cargo.toml`)

```toml
[dependencies]
tauri = { version = "2", features = [] }
tokio = { version = "1", features = ["full"] }
axum = { version = "0.7", features = ["macros"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
uuid = { version = "1", features = ["v4"] }
tokio-stream = "0.1"
futures = "0.3"
```

### HTTP/SSE-Server (`proxy/server.rs`)

Der axum-Server läuft auf Port 3663 und implementiert das MCP-Protokoll über SSE:

**Endpoints:**
- `GET /sse` — SSE-Verbindung, sendet `endpoint`-Event mit der Message-URL
- `POST /message` — Nimmt JSON-RPC-Requests entgegen, gibt Responses zurück

**MCP JSON-RPC Methoden die implementiert werden müssen:**
- `initialize` — Handshake, gibt `serverInfo` und `capabilities` zurück
- `tools/list` — Gibt die aggregierte Tool-Liste zurück (nur aktive Server)
- `tools/call` — Routet den Call an den richtigen Backend-Server

Beispiel-Response auf `initialize`:
```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": {
    "protocolVersion": "2024-11-05",
    "capabilities": { "tools": {} },
    "serverInfo": { "name": "meta-mcp", "version": "0.1.0" }
  }
}
```

Beispiel-Response auf `tools/list`:
```json
{
  "jsonrpc": "2.0",
  "id": 2,
  "result": {
    "tools": [
      {
        "name": "github__create_issue",
        "description": "Creates a GitHub issue",
        "inputSchema": { ... }
      }
    ]
  }
}
```

**Wichtig:** Tool-Namen werden beim Aggregieren mit dem Server-ID als Präfix versehen (`{server_id}__{original_name}`), damit der Router beim Tool-Call weiß, welcher Server gemeint ist. Der Präfix wird vor dem Weiterleiten an den Backend-Server wieder entfernt.

### Prozess-Management (`proxy/child.rs`)

Für stdio-Server:

```rust
pub struct ChildProcess {
    pub id: String,
    stdin: tokio::process::ChildStdin,
    stdout_reader: tokio::io::BufReader<tokio::process::ChildStdout>,
}

impl ChildProcess {
    pub async fn spawn(entry: &ServerEntry) -> Result<Self>
    pub async fn send_request(&mut self, req: serde_json::Value) -> Result<serde_json::Value>
    pub async fn list_tools(&mut self) -> Result<Vec<Tool>>
    pub async fn call_tool(&mut self, name: &str, args: serde_json::Value) -> Result<serde_json::Value>
}
```

Kommunikation über stdio läuft als newline-delimited JSON-RPC. Jeder aktive stdio-Server bekommt beim App-Start einen Child-Prozess gespawnt. Beim Deaktivieren wird der Prozess gekillt, beim Aktivieren neu gespawnt.

### Tool-Aggregator (`proxy/aggregator.rs`)

```rust
pub async fn build_tool_list(state: &ProxyState) -> Vec<Tool> {
    // Iteriert über alle aktiven Server
    // Fragt jeden Server nach tools/list
    // Prefixed die Namen: "{server_id}__{tool_name}"
    // Gibt zusammengeführte Liste zurück
}
```

Die Tool-Liste wird gecacht und nur bei Änderungen (Server aktiviert/deaktiviert, Profil gewechselt) neu gebaut. Cache-Invalidierung über einen `Arc<RwLock<Option<Vec<Tool>>>>`.

### Request Router (`proxy/router.rs`)

```rust
pub async fn route_tool_call(
    state: &ProxyState,
    prefixed_name: &str,
    args: serde_json::Value
) -> Result<serde_json::Value> {
    // Parst server_id aus dem Präfix
    // Findet den Child-Prozess
    // Sendet tools/call mit originalem Namen (ohne Präfix)
    // Gibt Response zurück
}
```

### Tauri IPC Commands (`commands.rs`)

Folgende Commands müssen implementiert werden:

```rust
#[tauri::command]
async fn get_config(state: State<AppState>) -> Result<Config, String>

#[tauri::command]
async fn save_server(state: State<AppState>, server: ServerEntry) -> Result<(), String>

#[tauri::command]
async fn delete_server(state: State<AppState>, id: String) -> Result<(), String>

#[tauri::command]
async fn set_server_active(state: State<AppState>, id: String, active: bool) -> Result<(), String>

#[tauri::command]
async fn save_profile(state: State<AppState>, profile: Profile) -> Result<(), String>

#[tauri::command]
async fn delete_profile(state: State<AppState>, id: String) -> Result<(), String>

#[tauri::command]
async fn set_active_profile(state: State<AppState>, profile_id: Option<String>) -> Result<(), String>

#[tauri::command]
async fn import_claude_config(state: State<AppState>, path: String) -> Result<Vec<ServerEntry>, String>
// Liest claude_desktop_config.json, parst mcpServers, gibt importierte Einträge zurück (noch nicht gespeichert)

#[tauri::command]
async fn get_tool_list(state: State<AppState>) -> Result<Vec<ToolWithServer>, String>
// Gibt aktuelle aggregierte Tool-Liste zurück, mit server_name Annotation

#[tauri::command]
async fn get_server_status(state: State<AppState>) -> Result<Vec<ServerStatus>, String>
// Gibt für jeden Server zurück: { id, active, connected: bool, tool_count: usize }
```

Jeder Command der `active`-Status ändert oder das Profil wechselt, triggert außerdem:
1. Cache-Invalidierung der Tool-Liste
2. Child-Prozesse starten/stoppen wie nötig

---

## Claude Config Import

Die Funktion `import_claude_config` liest die Datei unter dem angegebenen Pfad und parst:

```json
{
  "mcpServers": {
    "github": {
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-github"],
      "env": { "GITHUB_TOKEN": "..." }
    },
    "postgres": {
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-postgres", "postgresql://..."]
    }
  }
}
```

Jeder Eintrag wird zu einem `ServerEntry` gemappt. Der Nutzer sieht im UI eine Vorschau und bestätigt den Import. Bereits vorhandene Server (gleicher `name`) werden nicht doppelt importiert.

Der Standard-Pfad für Claude Desktop configs:
- macOS: `~/Library/Application Support/Claude/claude_desktop_config.json`
- Windows: `%APPDATA%\Claude\claude_desktop_config.json`
- Linux: `~/.config/Claude/claude_desktop_config.json`

---

## Frontend (Svelte)

### Layout

Einspaltige Desktop-App, drei Bereiche:

```
┌─────────────────────────────────────────┐
│  [Profil-Selektor]          Meta-MCP ●  │  ← Header
├─────────────────────────────────────────┤
│  Server                          [+ Add]│  ← Section
│  ┌──────────────────────────────────┐   │
│  │ GitHub MCP          ● aktiv  [●] │   │
│  │ 12 tools                         │   │
│  ├──────────────────────────────────┤   │
│  │ Postgres MCP        ○ inaktiv [○]│   │
│  │ 0 tools (inaktiv)                │   │
│  └──────────────────────────────────┘   │
├─────────────────────────────────────────┤
│  Tools (12 aktiv)                        │  ← Collapsible
│  github__create_issue     GitHub MCP    │
│  github__list_prs         GitHub MCP    │
│  ...                                    │
└─────────────────────────────────────────┘
```

### Komponenten

**`ProfileBar.svelte`**
- Dropdown mit allen Profilen + Option "Kein Profil (manuell)"
- Button "Profil speichern" (speichert aktuelle Toggle-Kombination als neues Profil)
- Button "Profil löschen"

**`ServerList.svelte`**
- Liste aller registrierten Server
- Jede Zeile: Name, Transport-Badge (stdio/SSE), Tool-Count, Status-Dot, Toggle
- Toggle-Änderung ruft `set_server_active` auf
- Rechtsklick oder Kebab-Menü: Bearbeiten, Löschen
- Button "+ Server hinzufügen" öffnet `AddServerModal`
- Button "Import aus Claude Config" öffnet Datei-Dialog → `import_claude_config`

**`AddServerModal.svelte`**
- Felder: Name, Transport (Radio: stdio / SSE)
- Wenn stdio: Command, Args (kommasepariert oder plus-Button für einzelne Args), Env-Vars (Key-Value-Paare)
- Wenn SSE: URL
- Validierung: Name nicht leer, Command nicht leer (stdio) oder valide URL (SSE)

**`ToolList.svelte`**
- Aufklappbare Sektion
- Zeigt alle Tools der aktiven Server
- Spalten: Tool-Name, Server-Name, kurze Description
- Filterbar per Text-Input

### Tauri-Aufrufe im Frontend

```typescript
import { invoke } from '@tauri-apps/api/core';

// Beispiel:
const config = await invoke<Config>('get_config');
await invoke('set_server_active', { id: server.id, active: true });
const tools = await invoke<ToolWithServer[]>('get_tool_list');
```

---

## Verbindungsstatusanzeige

In der Titelleiste: kleiner Status-Dot + Text.
- Grün + "SSE läuft auf :3663" — axum-Server ist up
- Grau + "Wird gestartet…" — Startphase
- Rot + "Fehler: Port belegt" — Port 3663 nicht verfügbar

Ein Tauri Event `proxy-status-changed` wird vom Backend emittiert, wann immer sich der Status ändert. Das Frontend lauscht darauf mit `listen('proxy-status-changed', ...)`.

---

## Wichtige Implementierungshinweise

### MCP-Protokoll-Details

Das MCP-Protokoll über SSE läuft so:
1. Client öffnet `GET /sse` — bekommt SSE-Stream
2. Server sendet sofort ein Event: `event: endpoint\ndata: /message\n\n`
3. Client sendet JSON-RPC-Requests an `POST /message`
4. Server antwortet direkt per HTTP-Response auf den POST (nicht über SSE)

Alternativ kann der Server auch Responses über den SSE-Stream senden. Claude Desktop akzeptiert beide Varianten. Die einfachere Option: direkte HTTP-Response auf den POST.

### Präfix-Kollisionen

Falls zwei Server ein Tool mit demselben Namen haben, werden beide über den Präfix disambiguiert. Das Präfix-Trennzeichen `__` (doppelter Unterstrich) ist so gewählt, dass es in normalen MCP-Tool-Namen nicht vorkommt. Falls ein Tool-Name bereits `__` enthält, ist das kein Problem — der Router splittet nur beim *ersten* `__`.

### Profil vs. manuelle Toggles

Wenn ein Profil aktiv ist, überschreibt es die `active`-Flags der Server. Das heißt:
- `active_profile = "coding"` → Server-`active`-Flags werden ignoriert, stattdessen gelten `profile.active_server_ids`
- `active_profile = null` → Die `active`-Flags der einzelnen Server gelten

Beim Wechsel zurück zu "kein Profil" bleiben die `active`-Flags so wie sie waren, bevor das Profil aktiviert wurde (d.h. nicht zurücksetzen).

### Prozesse beim App-Quit

In `tauri::Builder::on_window_event` oder über einen Cleanup-Hook: alle Child-Prozesse beim App-Quit graceful beenden (`child.kill()` + `child.wait()`).

---

## Was explizit NICHT gebaut werden soll

- Kein Tool-Editing (Namen, Descriptions ändern)
- Keine Authentifizierung / API-Keys für den Meta-MCP-Endpunkt selbst
- Kein Cloud-Sync, keine Remote-Konfiguration
- Kein Auto-Update
- Keine Unterstützung für MCP Resources oder Prompts (nur Tools)
- Kein Tool-Logging / Tracing (kann später ergänzt werden)

---

## Deliverable

Eine funktionierende Tauri 2 App, die:
1. Sich bauen lässt mit `cargo tauri dev` und `cargo tauri build`
2. Einen lokalen SSE-Server auf Port 3663 startet
3. Claude Desktop über diesen Endpunkt korrekt antwortet (initialize, tools/list, tools/call)
4. Eine funktionsfähige UI zum Verwalten von Servern und Profilen bietet
5. Den Import aus `claude_desktop_config.json` unterstützt

Zuerst eine funktionierende Basis (Proxy-Core + minimales UI) bauen, dann UI ausbauen.
