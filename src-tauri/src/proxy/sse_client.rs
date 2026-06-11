use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Result};
use futures::StreamExt;
use serde_json::{json, Value};
use tokio::sync::{oneshot, Mutex};
use tokio::task::JoinHandle;

use crate::config::ServerEntry;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
const ENDPOINT_TIMEOUT: Duration = Duration::from_secs(15);

type Pending = Arc<Mutex<HashMap<i64, oneshot::Sender<Result<Value>>>>>;

/// A backend MCP server reached over the legacy HTTP+SSE transport:
/// open `GET {url}` for the event stream, learn the message endpoint from the
/// `endpoint` event, then `POST` JSON-RPC and read responses back off the stream.
pub struct SseBackend {
    client: reqwest::Client,
    message_url: String,
    pending: Pending,
    next_id: AtomicI64,
    reader: JoinHandle<()>,
}

impl SseBackend {
    pub async fn connect(entry: &ServerEntry) -> Result<Self> {
        let url = entry
            .url
            .clone()
            .filter(|u| !u.trim().is_empty())
            .ok_or_else(|| anyhow!("sse server '{}' has no url", entry.name))?;

        let client = reqwest::Client::builder()
            .pool_idle_timeout(None)
            .build()?;

        let resp = client
            .get(&url)
            .header("Accept", "text/event-stream")
            .send()
            .await
            .map_err(|e| anyhow!("could not open SSE stream: {}", e))?;
        if !resp.status().is_success() {
            return Err(anyhow!("SSE connect failed: HTTP {}", resp.status()));
        }

        let base = reqwest::Url::parse(&url)?;
        let pending: Pending = Arc::new(Mutex::new(HashMap::new()));
        let (endpoint_tx, endpoint_rx) = oneshot::channel::<String>();

        let reader = spawn_reader(resp, base.clone(), pending.clone(), endpoint_tx);

        let message_path = tokio::time::timeout(ENDPOINT_TIMEOUT, endpoint_rx)
            .await
            .map_err(|_| anyhow!("timed out waiting for SSE endpoint event"))?
            .map_err(|_| anyhow!("SSE stream closed before endpoint event"))?;
        let message_url = base.join(&message_path)?.to_string();

        let backend = SseBackend {
            client,
            message_url,
            pending,
            next_id: AtomicI64::new(0),
            reader,
        };
        backend.initialize().await?;
        Ok(backend)
    }

    async fn initialize(&self) -> Result<()> {
        self.request(
            "initialize",
            json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": { "name": "meta-mcp", "version": "0.1.0" }
            }),
        )
        .await?;
        // Best-effort initialized notification.
        let _ = self
            .client
            .post(&self.message_url)
            .header("Content-Type", "application/json")
            .json(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }))
            .send()
            .await;
        Ok(())
    }

    async fn request(&self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst) + 1;
        let body = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params
        });

        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id, tx);

        let resp = self
            .client
            .post(&self.message_url)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream")
            .json(&body)
            .send()
            .await;

        match resp {
            Ok(r) => {
                let status = r.status();
                if !status.is_success() {
                    self.pending.lock().await.remove(&id);
                    return Err(anyhow!("POST to message endpoint failed: HTTP {}", status));
                }
                // Some servers reply on the POST body instead of the SSE stream.
                if let Ok(text) = r.text().await {
                    if let Ok(val) = serde_json::from_str::<Value>(text.trim()) {
                        if val.get("id").and_then(|v| v.as_i64()) == Some(id) {
                            if let Some(tx) = self.pending.lock().await.remove(&id) {
                                let _ = tx.send(extract_result(val));
                            }
                        }
                    }
                }
            }
            Err(e) => {
                self.pending.lock().await.remove(&id);
                return Err(anyhow!("POST to message endpoint failed: {}", e));
            }
        }

        let result = tokio::time::timeout(REQUEST_TIMEOUT, rx)
            .await
            .map_err(|_| anyhow!("timed out waiting for response to {}", method))?
            .map_err(|_| anyhow!("response channel dropped"))?;
        result
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
        self.reader.abort();
    }
}

fn extract_result(val: Value) -> Result<Value> {
    if let Some(err) = val.get("error") {
        return Err(anyhow!("backend error: {}", err));
    }
    Ok(val.get("result").cloned().unwrap_or(Value::Null))
}

/// Background task: parse the SSE byte stream into events and dispatch them.
fn spawn_reader(
    resp: reqwest::Response,
    base: reqwest::Url,
    pending: Pending,
    endpoint_tx: oneshot::Sender<String>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut stream = resp.bytes_stream();
        let mut buf = String::new();
        let mut event_name = String::new();
        let mut data = String::new();
        let mut endpoint_tx = Some(endpoint_tx);

        while let Some(chunk) = stream.next().await {
            let chunk = match chunk {
                Ok(c) => c,
                Err(_) => break,
            };
            buf.push_str(&String::from_utf8_lossy(&chunk));

            while let Some(pos) = buf.find('\n') {
                let line: String = buf.drain(..=pos).collect();
                let line = line.trim_end_matches(['\r', '\n']);

                if line.is_empty() {
                    dispatch_event(&event_name, &data, &base, &pending, &mut endpoint_tx).await;
                    event_name.clear();
                    data.clear();
                } else if let Some(rest) = line.strip_prefix("event:") {
                    event_name = rest.trim().to_string();
                } else if let Some(rest) = line.strip_prefix("data:") {
                    if !data.is_empty() {
                        data.push('\n');
                    }
                    data.push_str(rest.strip_prefix(' ').unwrap_or(rest));
                }
                // lines starting with ':' are comments / keep-alives — ignore.
            }
        }

        // Stream ended: fail everything still pending.
        let mut p = pending.lock().await;
        for (_, tx) in p.drain() {
            let _ = tx.send(Err(anyhow!("SSE stream closed")));
        }
    })
}

async fn dispatch_event(
    event_name: &str,
    data: &str,
    base: &reqwest::Url,
    pending: &Pending,
    endpoint_tx: &mut Option<oneshot::Sender<String>>,
) {
    if data.is_empty() && event_name.is_empty() {
        return;
    }
    if event_name == "endpoint" {
        if let Some(tx) = endpoint_tx.take() {
            let _ = tx.send(data.to_string());
        }
        return;
    }
    // Default ("message") events carry JSON-RPC.
    let _ = base; // base kept for symmetry / future relative resolution
    if let Ok(val) = serde_json::from_str::<Value>(data) {
        if let Some(id) = val.get("id").and_then(|v| v.as_i64()) {
            if let Some(tx) = pending.lock().await.remove(&id) {
                let _ = tx.send(extract_result(val));
            }
        }
    }
}
