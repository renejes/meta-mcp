use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// The full persisted configuration (`config.json`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub servers: Vec<ServerEntry>,
    #[serde(default)]
    pub profiles: Vec<Profile>,
    /// `None` = no profile, manual toggles apply.
    #[serde(default)]
    pub active_profile: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Transport {
    Stdio,
    /// Legacy HTTP+SSE transport (deprecated by the MCP spec, kept for older servers).
    Sse,
    /// Streamable HTTP transport (current spec): a single `/mcp` endpoint reached
    /// with `Mcp-Session-Id` sessions and SSE-framed responses.
    Http,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerEntry {
    /// uuid, generated on creation.
    #[serde(default)]
    pub id: String,
    pub name: String,
    pub transport: Transport,

    // stdio:
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env: Option<HashMap<String, String>>,

    // sse / http:
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Extra HTTP headers sent on every request to an http/sse backend
    /// (e.g. `Authorization: Bearer <token>` or an `X-API-Key`). The OAuth
    /// access token, when present, is injected here at connect time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub headers: Option<HashMap<String, String>>,

    /// Manual toggle (used when no profile is active).
    #[serde(default)]
    pub active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub active_server_ids: Vec<String>,
}

/// A tool annotated with its owning server — returned to the frontend.
#[derive(Debug, Clone, Serialize)]
pub struct ToolWithServer {
    /// Prefixed name (`{slug}__{original}`) as Claude sees it.
    pub name: String,
    pub server_id: String,
    pub server_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// Per-server live status for the UI.
#[derive(Debug, Clone, Serialize)]
pub struct ServerStatus {
    pub id: String,
    pub active: bool,
    pub connected: bool,
    #[serde(default)]
    pub needs_auth: bool,
    /// Has stored OAuth tokens (i.e. the user has logged in).
    #[serde(default)]
    pub authenticated: bool,
    pub tool_count: usize,
}

/// Proxy/server status pushed to the frontend via the `proxy-status-changed` event.
#[derive(Debug, Clone, Serialize)]
pub struct ProxyStatus {
    /// "starting" | "running" | "error"
    pub state: String,
    pub port: u16,
    pub message: String,
}
