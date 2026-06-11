//! Built-in stdio↔HTTP bridge.
//!
//! When the binary is launched with `--stdio` (e.g. by Claude Desktop, whose
//! JSON config only supports stdio servers), it does NOT start the GUI. Instead
//! it speaks newline-delimited JSON-RPC on stdio and forwards every message to
//! the running Meta-MCP proxy at `http://127.0.0.1:<port>/mcp`. If the GUI/proxy
//! isn't running yet, it best-effort launches the app bundle and waits for it.

use std::io::Write as _;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, BufReader};

use crate::proxy::PROXY_PORT;

pub fn run() {
    let rt = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("[meta-mcp --stdio] runtime error: {e}");
            return;
        }
    };
    rt.block_on(bridge());
}

fn mcp_url() -> String {
    format!("http://127.0.0.1:{PROXY_PORT}/mcp")
}

fn health_url() -> String {
    format!("http://127.0.0.1:{PROXY_PORT}/")
}

async fn bridge() {
    let client = reqwest::Client::new();
    ensure_proxy_running(&client).await;

    let stdin = tokio::io::stdin();
    let mut reader = BufReader::new(stdin).lines();
    let stdout = std::io::stdout();

    while let Ok(Some(line)) = reader.next_line().await {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        // Forward the raw JSON-RPC message to the proxy's Streamable-HTTP endpoint.
        let resp = client
            .post(mcp_url())
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream")
            .body(trimmed.to_owned())
            .send()
            .await;

        match resp {
            Ok(r) => {
                // 202 (notifications) → no body → nothing to write back.
                if let Ok(text) = r.text().await {
                    let body = text.trim();
                    if !body.is_empty() {
                        let mut out = stdout.lock();
                        let _ = writeln!(out, "{body}");
                        let _ = out.flush();
                    }
                }
            }
            Err(e) => {
                // Surface a JSON-RPC error so the client sees a clean failure.
                if let Some(id) = extract_id(trimmed) {
                    let err = format!(
                        "{{\"jsonrpc\":\"2.0\",\"id\":{id},\"error\":{{\"code\":-32000,\"message\":\"Meta-MCP proxy unreachable: {}\"}}}}",
                        e.to_string().replace('"', "'")
                    );
                    let mut out = stdout.lock();
                    let _ = writeln!(out, "{err}");
                    let _ = out.flush();
                }
            }
        }
    }
}

/// Probe the proxy; if it's down, try to launch the GUI app bundle and wait.
async fn ensure_proxy_running(client: &reqwest::Client) {
    if probe(client).await {
        return;
    }
    launch_app();
    for _ in 0..30 {
        tokio::time::sleep(Duration::from_millis(500)).await;
        if probe(client).await {
            return;
        }
    }
    eprintln!("[meta-mcp --stdio] proxy did not come up; forwarding will fail until the app runs");
}

async fn probe(client: &reqwest::Client) -> bool {
    client
        .get(health_url())
        .timeout(Duration::from_millis(800))
        .send()
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false)
}

/// Best-effort: derive the `.app` bundle from our own path and `open` it.
fn launch_app() {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    // exe = …/Meta-MCP.app/Contents/MacOS/meta-mcp → ancestor[3] = …/Meta-MCP.app
    if let Some(app) = exe.ancestors().nth(3) {
        if app.extension().map(|e| e == "app").unwrap_or(false) {
            let _ = std::process::Command::new("open").arg(app).spawn();
            return;
        }
    }
    // Fallback (e.g. dev binary): relaunch ourselves as the GUI.
    let _ = std::process::Command::new(&exe).spawn();
}

fn extract_id(line: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    v.get("id").map(|id| id.to_string())
}
