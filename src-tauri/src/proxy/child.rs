use std::process::Stdio;
use std::time::Duration;

use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

use crate::config::ServerEntry;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// A backend MCP server spoken to over stdio (newline-delimited JSON-RPC).
pub struct StdioBackend {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: i64,
}

impl StdioBackend {
    pub async fn spawn(entry: &ServerEntry) -> Result<Self> {
        let command = entry
            .command
            .clone()
            .filter(|c| !c.trim().is_empty())
            .ok_or_else(|| anyhow!("stdio server '{}' has no command", entry.name))?;

        let mut cmd = Command::new(&command);
        if let Some(args) = &entry.args {
            cmd.args(args);
        }
        if let Some(env) = &entry.env {
            for (k, v) in env {
                cmd.env(k, v);
            }
        }
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        cmd.kill_on_drop(true);

        let mut child = cmd
            .spawn()
            .map_err(|e| anyhow!("failed to spawn '{}': {}", command, e))?;

        let stdin = child.stdin.take().ok_or_else(|| anyhow!("no stdin"))?;
        let stdout = child.stdout.take().ok_or_else(|| anyhow!("no stdout"))?;
        let stderr = child.stderr.take();

        // Drain stderr in the background so the child never blocks on a full pipe.
        if let Some(stderr) = stderr {
            let name = entry.name.clone();
            tokio::spawn(async move {
                let mut reader = BufReader::new(stderr);
                let mut line = String::new();
                loop {
                    line.clear();
                    match reader.read_line(&mut line).await {
                        Ok(0) | Err(_) => break,
                        Ok(_) => eprintln!("[{}] {}", name, line.trim_end()),
                    }
                }
            });
        }

        let mut backend = StdioBackend {
            child,
            stdin,
            stdout: BufReader::new(stdout),
            next_id: 0,
        };
        backend.initialize().await?;
        Ok(backend)
    }

    fn next_id(&mut self) -> i64 {
        self.next_id += 1;
        self.next_id
    }

    async fn initialize(&mut self) -> Result<()> {
        let id = self.next_id();
        let init = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": { "name": "meta-mcp", "version": "0.1.0" }
            }
        });
        self.request(init, id).await?;
        self.notify(json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized"
        }))
        .await?;
        Ok(())
    }

    async fn notify(&mut self, msg: Value) -> Result<()> {
        let mut line = serde_json::to_string(&msg)?;
        line.push('\n');
        self.stdin.write_all(line.as_bytes()).await?;
        self.stdin.flush().await?;
        Ok(())
    }

    /// Send a request and read lines until the response with the matching id arrives.
    async fn request(&mut self, req: Value, id: i64) -> Result<Value> {
        let mut line = serde_json::to_string(&req)?;
        line.push('\n');
        self.stdin.write_all(line.as_bytes()).await?;
        self.stdin.flush().await?;

        loop {
            let mut buf = String::new();
            let n = tokio::time::timeout(REQUEST_TIMEOUT, self.stdout.read_line(&mut buf))
                .await
                .map_err(|_| anyhow!("timed out waiting for response"))??;
            if n == 0 {
                return Err(anyhow!("backend closed stdout"));
            }
            let trimmed = buf.trim();
            if trimmed.is_empty() {
                continue;
            }
            let val: Value = match serde_json::from_str(trimmed) {
                Ok(v) => v,
                Err(_) => continue, // ignore non-JSON log lines
            };
            // Match by id; skip notifications and responses to other requests.
            if val.get("id").and_then(|v| v.as_i64()) == Some(id) {
                if let Some(err) = val.get("error") {
                    return Err(anyhow!("backend error: {}", err));
                }
                return Ok(val.get("result").cloned().unwrap_or(Value::Null));
            }
        }
    }

    pub async fn list_tools(&mut self) -> Result<Vec<Value>> {
        let id = self.next_id();
        let req = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "tools/list",
            "params": {}
        });
        let result = self.request(req, id).await?;
        let tools = result
            .get("tools")
            .and_then(|t| t.as_array())
            .cloned()
            .unwrap_or_default();
        Ok(tools)
    }

    pub async fn call_tool(&mut self, name: &str, args: Value) -> Result<Value> {
        let id = self.next_id();
        let req = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "tools/call",
            "params": { "name": name, "arguments": args }
        });
        self.request(req, id).await
    }

    pub async fn shutdown(&mut self) {
        let _ = self.child.start_kill();
        let _ = self.child.wait().await;
    }
}
