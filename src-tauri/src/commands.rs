use std::collections::{HashMap, HashSet};

use serde_json::Value;
use tauri::{AppHandle, Manager, State};
use uuid::Uuid;

use crate::claude::{self, ClaudeStatus};
use crate::config::{Config, Profile, ProxyStatus, ServerEntry, ServerStatus, ToolWithServer, Transport};
use crate::proxy::ProxyState;

type CmdResult<T> = Result<T, String>;

#[tauri::command]
pub async fn get_config(state: State<'_, ProxyState>) -> CmdResult<Config> {
    Ok(state.config.read().await.clone())
}

#[tauri::command]
pub async fn get_proxy_status(state: State<'_, ProxyState>) -> CmdResult<ProxyStatus> {
    Ok(state.current_status().await)
}

#[tauri::command]
pub async fn save_server(state: State<'_, ProxyState>, server: ServerEntry) -> CmdResult<()> {
    let st = state.inner().clone();
    let mut server = server;
    if server.id.trim().is_empty() {
        server.id = Uuid::new_v4().to_string();
    }
    let id = server.id.clone();
    {
        let mut cfg = st.config.write().await;
        if let Some(existing) = cfg.servers.iter_mut().find(|s| s.id == id) {
            *existing = server;
        } else {
            cfg.servers.push(server);
        }
    }
    st.save_config().await.map_err(|e| e.to_string())?;
    // Re-spawn this backend so config edits take effect.
    st.drop_backend(&id).await;
    st.reconcile().await;
    Ok(())
}

#[tauri::command]
pub async fn delete_server(state: State<'_, ProxyState>, id: String) -> CmdResult<()> {
    let st = state.inner().clone();
    {
        let mut cfg = st.config.write().await;
        cfg.servers.retain(|s| s.id != id);
        for p in cfg.profiles.iter_mut() {
            p.active_server_ids.retain(|sid| sid != &id);
        }
    }
    st.save_config().await.map_err(|e| e.to_string())?;
    st.drop_backend(&id).await;
    st.reconcile().await;
    Ok(())
}

#[tauri::command]
pub async fn set_server_active(
    state: State<'_, ProxyState>,
    id: String,
    active: bool,
) -> CmdResult<()> {
    let st = state.inner().clone();
    {
        let mut cfg = st.config.write().await;
        if let Some(s) = cfg.servers.iter_mut().find(|s| s.id == id) {
            s.active = active;
        }
    }
    st.save_config().await.map_err(|e| e.to_string())?;
    st.reconcile().await;
    Ok(())
}

#[tauri::command]
pub async fn save_profile(state: State<'_, ProxyState>, profile: Profile) -> CmdResult<()> {
    let st = state.inner().clone();
    let mut profile = profile;
    if profile.id.trim().is_empty() {
        profile.id = Uuid::new_v4().to_string();
    }
    let pid = profile.id.clone();
    let is_active = {
        let mut cfg = st.config.write().await;
        if let Some(existing) = cfg.profiles.iter_mut().find(|p| p.id == pid) {
            *existing = profile;
        } else {
            cfg.profiles.push(profile);
        }
        cfg.active_profile.as_deref() == Some(&pid)
    };
    st.save_config().await.map_err(|e| e.to_string())?;
    if is_active {
        st.reconcile().await;
    }
    Ok(())
}

#[tauri::command]
pub async fn delete_profile(state: State<'_, ProxyState>, id: String) -> CmdResult<()> {
    let st = state.inner().clone();
    let was_active = {
        let mut cfg = st.config.write().await;
        cfg.profiles.retain(|p| p.id != id);
        if cfg.active_profile.as_deref() == Some(&id) {
            cfg.active_profile = None;
            true
        } else {
            false
        }
    };
    st.save_config().await.map_err(|e| e.to_string())?;
    if was_active {
        st.reconcile().await;
    }
    Ok(())
}

#[tauri::command]
pub async fn set_active_profile(
    state: State<'_, ProxyState>,
    profile_id: Option<String>,
) -> CmdResult<()> {
    let st = state.inner().clone();
    {
        let mut cfg = st.config.write().await;
        // Note: switching profiles never touches the per-server `active` flags,
        // so returning to "no profile" restores the previous manual state.
        cfg.active_profile = profile_id;
    }
    st.save_config().await.map_err(|e| e.to_string())?;
    st.reconcile().await;
    Ok(())
}

#[tauri::command]
pub async fn get_tool_list(state: State<'_, ProxyState>) -> CmdResult<Vec<ToolWithServer>> {
    let st = state.inner().clone();
    st.ensure_cache().await;
    let cache = st.tool_cache.read().await;
    let tools = cache
        .as_ref()
        .map(|c| {
            c.tools
                .iter()
                .map(|t| ToolWithServer {
                    name: t.prefixed_name.clone(),
                    server_id: t.server_id.clone(),
                    server_name: t.server_name.clone(),
                    description: t
                        .tool_json
                        .get("description")
                        .and_then(|d| d.as_str())
                        .map(|s| s.to_string()),
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(tools)
}

#[tauri::command]
pub async fn get_server_status(state: State<'_, ProxyState>) -> CmdResult<Vec<ServerStatus>> {
    let st = state.inner().clone();
    Ok(st.server_status().await)
}

/// Read a `claude_desktop_config.json`, map `mcpServers` to entries, and return
/// the ones whose name does not already exist. Nothing is saved yet.
#[tauri::command]
pub async fn import_claude_config(
    state: State<'_, ProxyState>,
    path: String,
) -> CmdResult<Vec<ServerEntry>> {
    let content =
        std::fs::read_to_string(&path).map_err(|e| format!("could not read file: {}", e))?;
    let json: Value =
        serde_json::from_str(&content).map_err(|e| format!("could not parse JSON: {}", e))?;

    let existing: HashSet<String> = state
        .config
        .read()
        .await
        .servers
        .iter()
        .map(|s| s.name.clone())
        .collect();

    let mut out = Vec::new();
    if let Some(map) = json.get("mcpServers").and_then(|m| m.as_object()) {
        for (name, def) in map {
            if existing.contains(name) {
                continue;
            }
            let entry = if let Some(url) = def.get("url").and_then(|u| u.as_str()) {
                ServerEntry {
                    id: Uuid::new_v4().to_string(),
                    name: name.clone(),
                    transport: Transport::Sse,
                    command: None,
                    args: None,
                    env: None,
                    url: Some(url.to_string()),
                    active: false,
                }
            } else {
                let command = def
                    .get("command")
                    .and_then(|c| c.as_str())
                    .map(|s| s.to_string());
                let args = def.get("args").and_then(|a| a.as_array()).map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(|s| s.to_string()))
                        .collect::<Vec<_>>()
                });
                let env = def.get("env").and_then(|e| e.as_object()).map(|o| {
                    o.iter()
                        .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                        .collect::<HashMap<_, _>>()
                });
                ServerEntry {
                    id: Uuid::new_v4().to_string(),
                    name: name.clone(),
                    transport: Transport::Stdio,
                    command,
                    args,
                    env,
                    url: None,
                    active: false,
                }
            };
            out.push(entry);
        }
    }
    Ok(out)
}

/// Whether Meta-MCP is currently registered in Claude Code / Claude Desktop.
#[tauri::command]
pub fn get_claude_status(app: AppHandle) -> CmdResult<ClaudeStatus> {
    Ok(claude::get_status(&app))
}

/// Add/remove the Meta-MCP entry in Claude Code's config (`~/.claude.json`).
#[tauri::command]
pub fn set_claude_code(app: AppHandle, enabled: bool) -> CmdResult<()> {
    claude::set_code(&app, enabled)
}

/// Add/remove the Meta-MCP stdio entry in `claude_desktop_config.json`.
#[tauri::command]
pub fn set_claude_desktop(app: AppHandle, enabled: bool) -> CmdResult<()> {
    claude::set_desktop(&app, enabled)
}

/// The conventional Claude Desktop config path for this OS.
#[tauri::command]
pub fn default_claude_config_path(app: AppHandle) -> CmdResult<String> {
    let home = app.path().home_dir().map_err(|e| e.to_string())?;
    #[cfg(target_os = "macos")]
    let p = home.join("Library/Application Support/Claude/claude_desktop_config.json");
    #[cfg(target_os = "windows")]
    let p = home.join("AppData/Roaming/Claude/claude_desktop_config.json");
    #[cfg(all(unix, not(target_os = "macos")))]
    let p = home.join(".config/Claude/claude_desktop_config.json");
    Ok(p.to_string_lossy().to_string())
}
