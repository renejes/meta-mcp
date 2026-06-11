pub mod aggregator;
pub mod backend;
pub mod child;
pub mod http_client;
pub mod router;
pub mod server;
pub mod sse_client;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use tauri::{AppHandle, Emitter};
use tokio::sync::{Mutex, RwLock};
use uuid::Uuid;

use aggregator::CachedTools;
use backend::Backend;
use crate::config::{self, Config, ProxyStatus, ServerEntry, ServerStatus};

pub const PROXY_PORT: u16 = 3663;

/// Shared, reference-counted proxy state. Stored in Tauri's managed state.
pub type ProxyState = Arc<ProxyStateInner>;

/// A connected backend, individually lockable so calls to different servers
/// can run concurrently.
type BackendHandle = Arc<Mutex<Backend>>;

pub struct ProxyStateInner {
    pub config: RwLock<Config>,
    pub config_path: PathBuf,
    pub backends: RwLock<HashMap<String, BackendHandle>>,
    /// Active servers whose connection attempt failed.
    pub failed: RwLock<HashSet<String>>,
    pub tool_cache: RwLock<Option<CachedTools>>,
    /// Legacy-SSE client sessions: session id → SSE event sender.
    pub sessions: Mutex<HashMap<String, tokio::sync::mpsc::UnboundedSender<String>>>,
    pub status: RwLock<ProxyStatus>,
    pub app: AppHandle,
}

impl ProxyStateInner {
    pub fn new(app: AppHandle, config_path: PathBuf, config: Config) -> ProxyState {
        Arc::new(ProxyStateInner {
            config: RwLock::new(config),
            config_path,
            backends: RwLock::new(HashMap::new()),
            failed: RwLock::new(HashSet::new()),
            tool_cache: RwLock::new(None),
            sessions: Mutex::new(HashMap::new()),
            status: RwLock::new(ProxyStatus {
                state: "starting".into(),
                port: PROXY_PORT,
                message: "Wird gestartet…".into(),
            }),
            app,
        })
    }

    /// The set of server ids that should be active right now: the active
    /// profile's list, or — with no profile — the manual `active` flags.
    pub async fn active_ids(&self) -> HashSet<String> {
        let cfg = self.config.read().await;
        match &cfg.active_profile {
            Some(pid) => cfg
                .profiles
                .iter()
                .find(|p| &p.id == pid)
                .map(|p| p.active_server_ids.iter().cloned().collect())
                .unwrap_or_default(),
            None => cfg
                .servers
                .iter()
                .filter(|s| s.active)
                .map(|s| s.id.clone())
                .collect(),
        }
    }

    pub async fn save_config(&self) -> anyhow::Result<()> {
        let cfg = self.config.read().await;
        config::save(&self.config_path, &cfg)
    }

    /// Add (or update, matched by name) a server — used by the `/register`
    /// endpoint so other apps can register themselves with Meta-MCP.
    pub async fn register_server(&self, mut entry: ServerEntry) -> ServerEntry {
        if entry.id.trim().is_empty() {
            entry.id = Uuid::new_v4().to_string();
        }
        {
            let mut cfg = self.config.write().await;
            if let Some(existing) = cfg.servers.iter_mut().find(|s| s.name == entry.name) {
                entry.id = existing.id.clone();
                *existing = entry.clone();
            } else {
                cfg.servers.push(entry.clone());
            }
        }
        let _ = self.save_config().await;
        self.reconcile().await;
        entry
    }

    /// Re-read config.json and, if it differs from what we hold, adopt it and
    /// reconcile. Lets external writers (other apps) change the config live.
    pub async fn reload_from_disk(&self) {
        let on_disk = config::load(&self.config_path);
        let changed = {
            let cur = self.config.read().await;
            serde_json::to_value(&*cur).ok() != serde_json::to_value(&on_disk).ok()
        };
        if !changed {
            return;
        }
        eprintln!("[meta-mcp] config.json changed externally → reloading");
        *self.config.write().await = on_disk;
        self.reconcile().await;
    }

    pub async fn invalidate_cache(&self) {
        *self.tool_cache.write().await = None;
    }

    /// Build the tool cache if it is currently empty.
    pub async fn ensure_cache(&self) {
        if self.tool_cache.read().await.is_some() {
            return;
        }
        let cache = aggregator::build_cache(self).await;
        *self.tool_cache.write().await = Some(cache);
    }

    /// Remove and shut down a single backend (e.g. before re-spawning after an edit).
    pub async fn drop_backend(&self, id: &str) {
        let handle = self.backends.write().await.remove(id);
        if let Some(handle) = handle {
            handle.lock().await.shutdown().await;
        }
        self.failed.write().await.remove(id);
    }

    /// Spawn/kill backends until the connected set matches the active set,
    /// then invalidate the cache and notify the UI.
    pub async fn reconcile(&self) {
        let active = self.active_ids().await;
        let servers: HashMap<String, ServerEntry> = self
            .config
            .read()
            .await
            .servers
            .iter()
            .map(|s| (s.id.clone(), s.clone()))
            .collect();

        // Kill backends that should no longer be running.
        let current: Vec<String> = self.backends.read().await.keys().cloned().collect();
        for id in current {
            if !active.contains(&id) {
                self.drop_backend(&id).await;
            }
        }

        // Spawn newly active backends (in parallel).
        let current_set: HashSet<String> = self.backends.read().await.keys().cloned().collect();
        let to_spawn: Vec<ServerEntry> = active
            .iter()
            .filter(|id| !current_set.contains(*id))
            .filter_map(|id| servers.get(id).cloned())
            .collect();

        let results = futures::future::join_all(to_spawn.into_iter().map(|entry| async move {
            let r = Backend::connect(&entry).await;
            (entry, r)
        }))
        .await;

        for (entry, result) in results {
            match result {
                Ok(backend) => {
                    self.backends
                        .write()
                        .await
                        .insert(entry.id.clone(), Arc::new(Mutex::new(backend)));
                    self.failed.write().await.remove(&entry.id);
                }
                Err(e) => {
                    eprintln!(
                        "[meta-mcp] could not connect '{}' ({}): {}",
                        entry.name, entry.id, e
                    );
                    self.failed.write().await.insert(entry.id.clone());
                }
            }
        }

        self.invalidate_cache().await;
        self.emit_status().await;
    }

    /// Per-server status for the UI.
    pub async fn server_status(&self) -> Vec<ServerStatus> {
        self.ensure_cache().await;
        let active = self.active_ids().await;
        let backends = self.backends.read().await;
        let cache = self.tool_cache.read().await;
        let cfg = self.config.read().await;
        cfg.servers
            .iter()
            .map(|s| {
                let tool_count = cache
                    .as_ref()
                    .map(|c| c.tools.iter().filter(|t| t.server_id == s.id).count())
                    .unwrap_or(0);
                ServerStatus {
                    id: s.id.clone(),
                    active: active.contains(&s.id),
                    connected: backends.contains_key(&s.id),
                    tool_count,
                }
            })
            .collect()
    }

    /// Kill every backend (used on app quit).
    pub async fn shutdown_all(&self) {
        let handles: Vec<BackendHandle> = self.backends.write().await.drain().map(|(_, h)| h).collect();
        for handle in handles {
            handle.lock().await.shutdown().await;
        }
    }

    pub async fn set_status(&self, state: &str, message: &str) {
        {
            let mut s = self.status.write().await;
            s.state = state.to_string();
            s.message = message.to_string();
        }
        self.emit_status().await;
    }

    pub async fn current_status(&self) -> ProxyStatus {
        self.status.read().await.clone()
    }

    async fn emit_status(&self) {
        let status = self.status.read().await.clone();
        let _ = self.app.emit("proxy-status-changed", status);
    }
}

/// Poll config.json for external modifications (other apps writing it directly)
/// and reload when its mtime changes.
pub fn spawn_config_watcher(state: ProxyState) {
    tauri::async_runtime::spawn(async move {
        let path = state.config_path.clone();
        let mtime =
            |p: &std::path::Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
        let mut last = mtime(&path);
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            let now = mtime(&path);
            if now != last {
                last = now;
                state.reload_from_disk().await;
            }
        }
    });
}
