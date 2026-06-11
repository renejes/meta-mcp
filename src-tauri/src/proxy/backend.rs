use anyhow::Result;
use serde_json::Value;

use super::child::StdioBackend;
use super::http_client::StreamableHttpBackend;
use super::sse_client::SseBackend;
use crate::config::{ServerEntry, Transport};

/// A connected backend MCP server, regardless of transport.
pub enum Backend {
    Stdio(StdioBackend),
    Sse(SseBackend),
    Http(StreamableHttpBackend),
}

impl Backend {
    pub async fn connect(entry: &ServerEntry) -> Result<Backend> {
        match entry.transport {
            Transport::Stdio => Ok(Backend::Stdio(StdioBackend::spawn(entry).await?)),
            Transport::Sse => Ok(Backend::Sse(SseBackend::connect(entry).await?)),
            Transport::Http => Ok(Backend::Http(StreamableHttpBackend::connect(entry).await?)),
        }
    }

    pub async fn list_tools(&mut self) -> Result<Vec<Value>> {
        match self {
            Backend::Stdio(b) => b.list_tools().await,
            Backend::Sse(b) => b.list_tools().await,
            Backend::Http(b) => b.list_tools().await,
        }
    }

    pub async fn call_tool(&mut self, name: &str, args: Value) -> Result<Value> {
        match self {
            Backend::Stdio(b) => b.call_tool(name, args).await,
            Backend::Sse(b) => b.call_tool(name, args).await,
            Backend::Http(b) => b.call_tool(name, args).await,
        }
    }

    pub async fn shutdown(&mut self) {
        match self {
            Backend::Stdio(b) => b.shutdown().await,
            Backend::Sse(b) => b.shutdown().await,
            Backend::Http(b) => b.shutdown().await,
        }
    }
}
