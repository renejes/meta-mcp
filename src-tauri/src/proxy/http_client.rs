use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

use anyhow::{anyhow, Result};
use futures::StreamExt;
use serde_json::{json, Value};
use tokio::sync::Mutex;

use crate::config::ServerEntry;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

/// A backend MCP server reached over the **Streamable HTTP** transport (the
/// current MCP spec). Unlike the legacy HTTP+SSE backend, there is a single
/// endpoint: every JSON-RPC message is `POST`ed to `{url}`. The server replies
/// either with `application/json` (one response) or a short-lived
/// `text/event-stream` carrying the response as a `message` event. The session
/// is identified by the `Mcp-Session-Id` header the server hands back on
/// `initialize`; we echo it (and the negotiated `MCP-Protocol-Version`) on every
/// subsequent request.
pub struct StreamableHttpBackend {
    client: reqwest::Client,
    url: String,
    session_id: Mutex<Option<String>>,
    protocol_version: Mutex<Option<String>>,
    next_id: AtomicI64,
}

impl StreamableHttpBackend {
    pub async fn connect(entry: &ServerEntry) -> Result<Self> {
        let url = entry
            .url
            .clone()
            .filter(|u| !u.trim().is_empty())
            .ok_or_else(|| anyhow!("http server '{}' has no url", entry.name))?;

        let client = reqwest::Client::builder().build()?;

        let backend = StreamableHttpBackend {
            client,
            url,
            session_id: Mutex::new(None),
            protocol_version: Mutex::new(None),
            next_id: AtomicI64::new(0),
        };
        backend.initialize().await?;
        Ok(backend)
    }

    async fn initialize(&self) -> Result<()> {
        let result = self
            .request(
                "initialize",
                json!({
                    "protocolVersion": "2025-06-18",
                    "capabilities": {},
                    "clientInfo": { "name": "meta-mcp", "version": "0.1.0" }
                }),
            )
            .await?;
        // Remember the version the server negotiated; spec requires we send it
        // back in the `MCP-Protocol-Version` header on later requests.
        if let Some(v) = result.get("protocolVersion").and_then(|v| v.as_str()) {
            *self.protocol_version.lock().await = Some(v.to_string());
        }
        // Best-effort `initialized` notification (the server replies 202).
        let _ = self
            .post(json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }))
            .await
            .map(|_| ());
        Ok(())
    }

    /// POST one JSON-RPC frame, attaching the session + protocol headers we hold,
    /// and capture the `Mcp-Session-Id` the server assigns on `initialize`.
    async fn post(&self, body: Value) -> Result<reqwest::Response> {
        let session = self.session_id.lock().await.clone();
        let protocol = self.protocol_version.lock().await.clone();

        let mut req = self
            .client
            .post(&self.url)
            .header("Content-Type", "application/json")
            // Streamable HTTP servers require BOTH to be accepted, else 406.
            .header("Accept", "application/json, text/event-stream");
        if let Some(sid) = session {
            req = req.header("Mcp-Session-Id", sid);
        }
        if let Some(pv) = protocol {
            req = req.header("MCP-Protocol-Version", pv);
        }

        let resp = req
            .json(&body)
            .send()
            .await
            .map_err(|e| anyhow!("POST to {} failed: {}", self.url, e))?;

        if let Some(sid) = resp
            .headers()
            .get("mcp-session-id")
            .and_then(|v| v.to_str().ok())
        {
            let mut guard = self.session_id.lock().await;
            if guard.is_none() {
                *guard = Some(sid.to_string());
            }
        }

        Ok(resp)
    }

    async fn request(&self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst) + 1;
        let body = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params
        });

        let fut = async {
            let resp = self.post(body).await?;
            let status = resp.status();
            if !status.is_success() {
                let text = resp.text().await.unwrap_or_default();
                return Err(anyhow!(
                    "backend returned HTTP {}{}",
                    status,
                    if text.trim().is_empty() {
                        String::new()
                    } else {
                        format!(": {}", text.trim())
                    }
                ));
            }
            let content_type = resp
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_ascii_lowercase();

            if content_type.contains("text/event-stream") {
                read_sse_response(resp, id).await
            } else {
                let text = resp.text().await.unwrap_or_default();
                let val: Value = serde_json::from_str(text.trim())
                    .map_err(|e| anyhow!("invalid JSON response to {}: {}", method, e))?;
                extract_result(val)
            }
        };

        tokio::time::timeout(REQUEST_TIMEOUT, fut)
            .await
            .map_err(|_| anyhow!("timed out waiting for response to {}", method))?
    }

    pub async fn list_tools(&mut self) -> Result<Vec<Value>> {
        let result = self.request("tools/list", json!({})).await?;
        let tools = result
            .get("tools")
            .and_then(|t| t.as_array())
            .cloned()
            .unwrap_or_default();
        Ok(tools)
    }

    pub async fn call_tool(&mut self, name: &str, args: Value) -> Result<Value> {
        self.request("tools/call", json!({ "name": name, "arguments": args }))
            .await
    }

    pub async fn shutdown(&mut self) {
        // Best-effort: tell the server to drop the session (spec: DELETE /mcp).
        let session = self.session_id.lock().await.clone();
        if let Some(sid) = session {
            let _ = self
                .client
                .delete(&self.url)
                .header("Mcp-Session-Id", sid)
                .send()
                .await;
        }
    }
}

fn extract_result(val: Value) -> Result<Value> {
    if let Some(err) = val.get("error") {
        return Err(anyhow!("backend error: {}", err));
    }
    Ok(val.get("result").cloned().unwrap_or(Value::Null))
}

/// Read a streamable-HTTP SSE response body and return the result of the first
/// JSON-RPC message whose `id` matches. The per-request stream carries the lone
/// response and then closes, so this returns as soon as it sees the match.
async fn read_sse_response(resp: reqwest::Response, want_id: i64) -> Result<Value> {
    let mut stream = resp.bytes_stream();
    let mut buf = String::new();
    let mut data = String::new();

    let try_dispatch = |data: &mut String| -> Option<Result<Value>> {
        if data.is_empty() {
            return None;
        }
        let parsed = serde_json::from_str::<Value>(data);
        data.clear();
        match parsed {
            Ok(val) if val.get("id").and_then(|v| v.as_i64()) == Some(want_id) => {
                Some(extract_result(val))
            }
            _ => None,
        }
    };

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| anyhow!("stream error: {}", e))?;
        buf.push_str(&String::from_utf8_lossy(&chunk));

        while let Some(pos) = buf.find('\n') {
            let line: String = buf.drain(..=pos).collect();
            let line = line.trim_end_matches(['\r', '\n']);

            if line.is_empty() {
                // Blank line terminates one SSE event.
                if let Some(result) = try_dispatch(&mut data) {
                    return result;
                }
            } else if let Some(rest) = line.strip_prefix("data:") {
                if !data.is_empty() {
                    data.push('\n');
                }
                data.push_str(rest.strip_prefix(' ').unwrap_or(rest));
            }
            // `event:`, `id:`, `retry:` and `:`-comments are ignored.
        }
    }

    // Stream ended without a trailing blank line — try whatever buffered.
    if let Some(result) = try_dispatch(&mut data) {
        return result;
    }
    Err(anyhow!("stream closed without a response for id {}", want_id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ServerEntry, Transport};

    /// Round-trips against a real streamable-HTTP MCP server. Skipped unless
    /// `CDB_TEST_URL` points at one (e.g. a running `cdb-mcp-serve`):
    ///   CDB_TEST_URL=http://127.0.0.1:7879/mcp cargo test -p meta-mcp streamable -- --nocapture
    #[tokio::test]
    async fn streamable_round_trip() {
        let Ok(url) = std::env::var("CDB_TEST_URL") else {
            eprintln!("CDB_TEST_URL not set — skipping streamable_round_trip");
            return;
        };
        let entry = ServerEntry {
            id: String::new(),
            name: "common-database".into(),
            transport: Transport::Http,
            command: None,
            args: None,
            env: None,
            url: Some(url),
            active: true,
        };
        let mut backend = StreamableHttpBackend::connect(&entry)
            .await
            .expect("connect + initialize");
        let tools = backend.list_tools().await.expect("tools/list");
        assert!(!tools.is_empty(), "expected a non-empty tool list");
        let names: Vec<&str> = tools
            .iter()
            .filter_map(|t| t.get("name").and_then(|n| n.as_str()))
            .collect();
        assert!(
            names.contains(&"list_categories"),
            "list_categories should be present, got {names:?}"
        );
        let result = backend
            .call_tool("list_categories", serde_json::json!({}))
            .await
            .expect("tools/call list_categories");
        assert!(result.get("content").is_some(), "result has content: {result}");
        backend.shutdown().await;
        eprintln!("streamable_round_trip OK — {} tools", tools.len());
    }
}
