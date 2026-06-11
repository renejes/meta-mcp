use anyhow::{anyhow, Result};
use serde_json::Value;

use super::ProxyStateInner;

/// Route a `tools/call` to the correct backend, stripping the slug prefix.
pub async fn route_tool_call(
    state: &ProxyStateInner,
    prefixed_name: &str,
    args: Value,
) -> Result<Value> {
    // Make sure the slug map is populated.
    state.ensure_cache().await;

    // Split on the *first* `__`; the original name may itself contain `__`.
    let (slug, original) = prefixed_name
        .split_once("__")
        .ok_or_else(|| anyhow!("tool name '{}' has no server prefix", prefixed_name))?;

    let server_id = {
        let cache = state.tool_cache.read().await;
        cache
            .as_ref()
            .and_then(|c| c.slug_to_id.get(slug).cloned())
    }
    .ok_or_else(|| anyhow!("no active server for prefix '{}'", slug))?;

    let handle = {
        let backends = state.backends.read().await;
        backends.get(&server_id).cloned()
    }
    .ok_or_else(|| anyhow!("server '{}' is not connected", slug))?;

    let mut backend = handle.lock().await;
    backend.call_tool(original, args).await
}
