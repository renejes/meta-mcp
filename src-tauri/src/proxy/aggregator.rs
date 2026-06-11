use std::collections::{HashMap, HashSet};
use std::time::Duration;

use serde_json::Value;

use super::ProxyStateInner;
use crate::config::ServerEntry;

const LIST_TIMEOUT: Duration = Duration::from_secs(20);

/// One aggregated tool, with everything the proxy and UI need.
#[derive(Clone)]
pub struct AggregatedTool {
    pub prefixed_name: String,
    pub server_id: String,
    pub server_name: String,
    /// The full tool object as returned by the backend, with `name` rewritten
    /// to the prefixed name — ready to hand back in `tools/list`.
    pub tool_json: Value,
}

/// The cached tool list plus the slug→server-id map used for routing.
#[derive(Clone, Default)]
pub struct CachedTools {
    pub tools: Vec<AggregatedTool>,
    pub slug_to_id: HashMap<String, String>,
}

/// Turn a display name into a tool-name-safe slug. Collapses runs of
/// non-alphanumerics into a single `_`, so a slug never contains `__`.
pub fn slugify(name: &str) -> String {
    let mut out = String::new();
    let mut pending_us = false;
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            if pending_us && !out.is_empty() {
                out.push('_');
            }
            pending_us = false;
            out.push(ch.to_ascii_lowercase());
        } else {
            pending_us = true;
        }
    }
    out
}

/// Build the slug→server-id map deterministically (config order), resolving
/// collisions with a numeric suffix. Only active servers are included.
fn build_slugs(servers: &[ServerEntry], active: &HashSet<String>) -> HashMap<String, String> {
    let mut id_to_slug: HashMap<String, String> = HashMap::new();
    let mut used: HashSet<String> = HashSet::new();
    for s in servers {
        if !active.contains(&s.id) {
            continue;
        }
        let mut base = slugify(&s.name);
        if base.is_empty() {
            let short = s.id.split('-').next().unwrap_or("server");
            base = format!("server_{}", short);
        }
        let mut candidate = base.clone();
        let mut n = 2;
        while used.contains(&candidate) {
            candidate = format!("{}_{}", base, n);
            n += 1;
        }
        used.insert(candidate.clone());
        id_to_slug.insert(s.id.clone(), candidate);
    }
    id_to_slug
}

/// Query every active, connected backend for its tools, prefix the names, and
/// assemble the cache.
pub async fn build_cache(state: &ProxyStateInner) -> CachedTools {
    let active = state.active_ids().await;
    let servers: Vec<ServerEntry> = state.config.read().await.servers.clone();
    let backends = state.backends.read().await.clone();

    let id_to_slug = build_slugs(&servers, &active);

    // Fire all list_tools calls concurrently.
    let mut futs = Vec::new();
    for s in &servers {
        if !active.contains(&s.id) {
            continue;
        }
        let (Some(slug), Some(handle)) = (id_to_slug.get(&s.id).cloned(), backends.get(&s.id).cloned())
        else {
            continue;
        };
        let server_id = s.id.clone();
        let server_name = s.name.clone();
        futs.push(async move {
            let tools = {
                let mut b = handle.lock().await;
                match tokio::time::timeout(LIST_TIMEOUT, b.list_tools()).await {
                    Ok(Ok(t)) => t,
                    Ok(Err(e)) => {
                        eprintln!("[meta-mcp] tools/list failed for {}: {}", server_name, e);
                        Vec::new()
                    }
                    Err(_) => {
                        eprintln!("[meta-mcp] tools/list timed out for {}", server_name);
                        Vec::new()
                    }
                }
            };
            (server_id, server_name, slug, tools)
        });
    }

    let results = futures::future::join_all(futs).await;

    let mut tools = Vec::new();
    let mut slug_to_id = HashMap::new();
    for (server_id, server_name, slug, raw_tools) in results {
        slug_to_id.insert(slug.clone(), server_id.clone());
        for mut tool in raw_tools {
            let original = tool
                .get("name")
                .and_then(|n| n.as_str())
                .unwrap_or("")
                .to_string();
            if original.is_empty() {
                continue;
            }
            let prefixed = format!("{}__{}", slug, original);
            if let Some(obj) = tool.as_object_mut() {
                obj.insert("name".into(), Value::String(prefixed.clone()));
            }
            tools.push(AggregatedTool {
                prefixed_name: prefixed,
                server_id: server_id.clone(),
                server_name: server_name.clone(),
                tool_json: tool,
            });
        }
    }

    CachedTools { tools, slug_to_id }
}
