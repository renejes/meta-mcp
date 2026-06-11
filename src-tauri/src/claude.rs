//! Self-registration into Claude Code and Claude Desktop.
//!
//! - Claude Code natively supports remote servers, so we add a `type:"http"`
//!   entry pointing at our Streamable-HTTP endpoint in `~/.claude.json`.
//! - Claude Desktop's JSON only supports stdio servers, so we register our own
//!   binary in `--stdio` mode (the built-in bridge) in `claude_desktop_config.json`.
//!
//! Both edits parse → modify only the `meta-mcp` key → write back, preserving
//! every other entry and setting in the file.

use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::{json, Map, Value};
use tauri::{AppHandle, Manager};

use crate::proxy::PROXY_PORT;

const SERVER_KEY: &str = "meta-mcp";

#[derive(Debug, Clone, Serialize)]
pub struct ClaudeStatus {
    pub code: bool,
    pub desktop: bool,
}

fn code_path(app: &AppHandle) -> Option<PathBuf> {
    app.path().home_dir().ok().map(|h| h.join(".claude.json"))
}

fn desktop_path(app: &AppHandle) -> Option<PathBuf> {
    let home = app.path().home_dir().ok()?;
    #[cfg(target_os = "macos")]
    let p = home.join("Library/Application Support/Claude/claude_desktop_config.json");
    #[cfg(target_os = "windows")]
    let p = home.join("AppData/Roaming/Claude/claude_desktop_config.json");
    #[cfg(all(unix, not(target_os = "macos")))]
    let p = home.join(".config/Claude/claude_desktop_config.json");
    Some(p)
}

fn code_entry() -> Value {
    json!({ "type": "http", "url": format!("http://localhost:{PROXY_PORT}/mcp") })
}

fn desktop_entry() -> Value {
    let exe = std::env::current_exe()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| "meta-mcp".to_string());
    json!({ "command": exe, "args": ["--stdio"] })
}

fn read_obj(path: &Path) -> Map<String, Value> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default()
}

fn has_entry(path: &Path) -> bool {
    read_obj(path)
        .get("mcpServers")
        .and_then(|s| s.as_object())
        .map(|m| m.contains_key(SERVER_KEY))
        .unwrap_or(false)
}

fn apply(path: &Path, entry: Option<Value>) -> std::io::Result<()> {
    let mut obj = read_obj(path);
    let servers = obj
        .entry("mcpServers".to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    if !servers.is_object() {
        *servers = Value::Object(Map::new());
    }
    let m = servers.as_object_mut().expect("just ensured object");
    match entry {
        Some(e) => {
            m.insert(SERVER_KEY.to_string(), e);
        }
        None => {
            m.remove(SERVER_KEY);
        }
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string_pretty(&Value::Object(obj))
        .unwrap_or_else(|_| "{}".to_string());
    std::fs::write(path, text)
}

pub fn get_status(app: &AppHandle) -> ClaudeStatus {
    ClaudeStatus {
        code: code_path(app).map(|p| has_entry(&p)).unwrap_or(false),
        desktop: desktop_path(app).map(|p| has_entry(&p)).unwrap_or(false),
    }
}

pub fn set_code(app: &AppHandle, enabled: bool) -> Result<(), String> {
    let path = code_path(app).ok_or("could not resolve ~/.claude.json")?;
    apply(&path, enabled.then(code_entry)).map_err(|e| e.to_string())
}

pub fn set_desktop(app: &AppHandle, enabled: bool) -> Result<(), String> {
    let path = desktop_path(app).ok_or("could not resolve Claude Desktop config path")?;
    apply(&path, enabled.then(desktop_entry)).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_preserves_other_content() {
        let path = std::env::temp_dir().join("metamcp-claude-apply-test.json");
        // A config with unrelated keys and another MCP server.
        let original = r#"{
  "numStartups": 42,
  "theme": "dark",
  "mcpServers": {
    "github": { "command": "npx", "args": ["-y", "server-github"] }
  },
  "tipsShown": ["a", "b"]
}"#;
        std::fs::write(&path, original).unwrap();

        // Add our entry.
        apply(&path, Some(json!({ "type": "http", "url": "http://x/mcp" }))).unwrap();
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(v["numStartups"], 42);
        assert_eq!(v["theme"], "dark");
        assert_eq!(v["tipsShown"][1], "b");
        assert!(v["mcpServers"]["github"].is_object(), "other server kept");
        assert_eq!(v["mcpServers"][SERVER_KEY]["type"], "http");
        // preserve_order: first key stays first.
        assert_eq!(
            v.as_object().unwrap().keys().next().map(String::as_str),
            Some("numStartups")
        );

        // Remove our entry → everything else intact, github untouched.
        apply(&path, None).unwrap();
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert!(v["mcpServers"].get(SERVER_KEY).is_none());
        assert!(v["mcpServers"]["github"].is_object());
        assert_eq!(v["numStartups"], 42);

        std::fs::remove_file(&path).ok();
    }
}
