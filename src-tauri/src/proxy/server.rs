use std::convert::Infallible;

use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use futures::{Stream, StreamExt};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio_stream::wrappers::UnboundedReceiverStream;
use uuid::Uuid;

use std::collections::HashMap;

use super::{router as tool_router, ProxyState, ProxyStateInner};
use crate::config::{ServerEntry, Transport};

/// Build the axum router exposing both MCP transports.
pub fn router(state: ProxyState) -> Router {
    Router::new()
        .route("/", get(health))
        .route("/sse", get(sse_handler))
        .route("/message", post(message_handler))
        .route("/mcp", get(mcp_get).post(mcp_post).delete(mcp_delete))
        .route("/register", post(register_handler))
        .with_state(state)
}

/// Body for `POST /register` — lets another local app register itself with
/// Meta-MCP instead of writing into Claude's config directly.
#[derive(Deserialize)]
struct RegisterRequest {
    name: String,
    transport: Transport,
    #[serde(default)]
    command: Option<String>,
    #[serde(default)]
    args: Option<Vec<String>>,
    #[serde(default)]
    env: Option<HashMap<String, String>>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default = "default_true")]
    active: bool,
}

fn default_true() -> bool {
    true
}

async fn register_handler(
    State(state): State<ProxyState>,
    body: String,
) -> impl IntoResponse {
    let req: RegisterRequest = match serde_json::from_str(&body) {
        Ok(v) => v,
        Err(e) => {
            return (StatusCode::BAD_REQUEST, format!("invalid body: {}", e)).into_response()
        }
    };
    if req.name.trim().is_empty() {
        return (StatusCode::BAD_REQUEST, "name is required").into_response();
    }
    let entry = ServerEntry {
        id: String::new(),
        name: req.name,
        transport: req.transport,
        command: req.command,
        args: req.args,
        env: req.env,
        url: req.url,
        active: req.active,
    };
    let saved = state.register_server(entry).await;
    (StatusCode::OK, Json(saved)).into_response()
}

async fn health() -> impl IntoResponse {
    "Meta-MCP proxy is running. Connect MCP clients to /sse or /mcp."
}

// ---------------------------------------------------------------------------
// JSON-RPC core
// ---------------------------------------------------------------------------

fn ok(id: Option<Value>, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id.unwrap_or(Value::Null), "result": result })
}

fn err(id: Option<Value>, code: i64, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id.unwrap_or(Value::Null),
        "error": { "code": code, "message": message }
    })
}

/// Handle a single JSON-RPC message. Returns `None` for notifications.
async fn handle_jsonrpc(state: &ProxyStateInner, req: &Value) -> Option<Value> {
    let method = req.get("method").and_then(|m| m.as_str()).unwrap_or("");
    let id = req.get("id").cloned();
    let is_notification = id.is_none();

    match method {
        "initialize" => {
            let version = req
                .get("params")
                .and_then(|p| p.get("protocolVersion"))
                .and_then(|v| v.as_str())
                .unwrap_or("2024-11-05")
                .to_string();
            Some(ok(
                id,
                json!({
                    "protocolVersion": version,
                    "capabilities": { "tools": {} },
                    "serverInfo": { "name": "meta-mcp", "version": "0.1.0" }
                }),
            ))
        }
        "notifications/initialized" | "notifications/cancelled" => None,
        "ping" => Some(ok(id, json!({}))),
        "tools/list" => {
            state.ensure_cache().await;
            let cache = state.tool_cache.read().await;
            let tools: Vec<Value> = cache
                .as_ref()
                .map(|c| c.tools.iter().map(|t| t.tool_json.clone()).collect())
                .unwrap_or_default();
            Some(ok(id, json!({ "tools": tools })))
        }
        "tools/call" => {
            let params = req.get("params").cloned().unwrap_or_else(|| json!({}));
            let name = params.get("name").and_then(|n| n.as_str()).unwrap_or("");
            let args = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            match tool_router::route_tool_call(state, name, args).await {
                Ok(result) => Some(ok(id, result)),
                Err(e) => Some(err(id, -32000, &e.to_string())),
            }
        }
        // We only expose tools, but answer these gracefully if probed.
        "resources/list" => Some(ok(id, json!({ "resources": [] }))),
        "resources/templates/list" => Some(ok(id, json!({ "resourceTemplates": [] }))),
        "prompts/list" => Some(ok(id, json!({ "prompts": [] }))),
        _ => {
            if is_notification {
                None
            } else {
                Some(err(id, -32601, &format!("Method not found: {}", method)))
            }
        }
    }
}

/// Process a single message or a JSON-RPC batch.
async fn process(state: &ProxyStateInner, req: Value) -> Vec<Value> {
    if let Some(arr) = req.as_array() {
        let mut out = Vec::new();
        for item in arr {
            if let Some(resp) = handle_jsonrpc(state, item).await {
                out.push(resp);
            }
        }
        out
    } else {
        match handle_jsonrpc(state, &req).await {
            Some(resp) => vec![resp],
            None => vec![],
        }
    }
}

// ---------------------------------------------------------------------------
// Legacy HTTP+SSE transport: GET /sse + POST /message
// ---------------------------------------------------------------------------

/// Removes its session from the map when the SSE stream is dropped.
struct SessionGuard {
    id: String,
    state: ProxyState,
}

impl Drop for SessionGuard {
    fn drop(&mut self) {
        let id = self.id.clone();
        let state = self.state.clone();
        tokio::spawn(async move {
            state.sessions.lock().await.remove(&id);
        });
    }
}

async fn sse_handler(
    State(state): State<ProxyState>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let session_id = Uuid::new_v4().to_string();
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    state.sessions.lock().await.insert(session_id.clone(), tx);

    let guard = SessionGuard {
        id: session_id.clone(),
        state: state.clone(),
    };

    // First, tell the client where to POST. Then stream responses as they come.
    let endpoint = Event::default()
        .event("endpoint")
        .data(format!("/message?sessionId={}", session_id));
    let initial = futures::stream::once(async move { Ok::<Event, Infallible>(endpoint) });
    let messages = UnboundedReceiverStream::new(rx).map(move |data| {
        // `guard` is held for the life of the stream so the session is cleaned up.
        let _keep = &guard;
        Ok::<Event, Infallible>(Event::default().event("message").data(data))
    });

    Sse::new(initial.chain(messages)).keep_alive(KeepAlive::default())
}

#[derive(Deserialize)]
struct MessageQuery {
    #[serde(rename = "sessionId")]
    session_id: Option<String>,
}

async fn message_handler(
    State(state): State<ProxyState>,
    Query(q): Query<MessageQuery>,
    body: String,
) -> impl IntoResponse {
    let req: Value = match serde_json::from_str(&body) {
        Ok(v) => v,
        Err(e) => {
            return (StatusCode::BAD_REQUEST, format!("invalid JSON: {}", e)).into_response()
        }
    };

    let responses = process(&state, req).await;

    // Deliver responses over the matching SSE session.
    if let Some(sid) = &q.session_id {
        let sessions = state.sessions.lock().await;
        if let Some(tx) = sessions.get(sid) {
            for resp in &responses {
                let _ = tx.send(serde_json::to_string(resp).unwrap_or_default());
            }
            return StatusCode::ACCEPTED.into_response();
        }
    }

    // No session → fall back to replying on the POST itself.
    match responses.len() {
        0 => StatusCode::ACCEPTED.into_response(),
        1 => Json(responses.into_iter().next().unwrap()).into_response(),
        _ => Json(responses).into_response(),
    }
}

// ---------------------------------------------------------------------------
// Streamable HTTP transport: POST /mcp (and GET/DELETE /mcp)
// ---------------------------------------------------------------------------

async fn mcp_post(
    State(state): State<ProxyState>,
    headers: HeaderMap,
    body: String,
) -> impl IntoResponse {
    let req: Value = match serde_json::from_str(&body) {
        Ok(v) => v,
        Err(e) => {
            return (StatusCode::BAD_REQUEST, format!("invalid JSON: {}", e)).into_response()
        }
    };

    let is_init = req.get("method").and_then(|m| m.as_str()) == Some("initialize");
    let responses = process(&state, req).await;

    let mut resp_headers = HeaderMap::new();
    if let Some(sid) = headers.get("mcp-session-id").and_then(|v| v.to_str().ok()) {
        if let Ok(val) = sid.parse() {
            resp_headers.insert("Mcp-Session-Id", val);
        }
    } else if is_init {
        if let Ok(val) = Uuid::new_v4().to_string().parse() {
            resp_headers.insert("Mcp-Session-Id", val);
        }
    }

    if responses.is_empty() {
        return (StatusCode::ACCEPTED, resp_headers).into_response();
    }
    let payload = if responses.len() == 1 {
        responses.into_iter().next().unwrap()
    } else {
        Value::Array(responses)
    };
    (resp_headers, Json(payload)).into_response()
}

/// Some clients open a GET stream for server-initiated messages. We have none,
/// so we hold an idle keep-alive stream open.
async fn mcp_get() -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let stream = futures::stream::pending::<Result<Event, Infallible>>();
    Sse::new(stream).keep_alive(KeepAlive::default())
}

async fn mcp_delete() -> impl IntoResponse {
    StatusCode::OK
}
