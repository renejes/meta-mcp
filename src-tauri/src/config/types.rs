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
    Sse,
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

    // sse:
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,

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
