//! OAuth 2.1 client for remote MCP servers that require an interactive login
//! (the MCP Authorization spec). Implements:
//!   - discovery: Protected Resource Metadata (RFC 9728) → Authorization Server
//!     Metadata (RFC 8414)
//!   - Dynamic Client Registration (RFC 7591)
//!   - Authorization Code + PKCE (RFC 7636) via a loopback redirect + browser
//!   - token exchange + refresh
//!   - a small on-disk token store (`oauth.json` next to `config.json`)
//!
//! Tokens are stored in plaintext in the app data dir for now (consistent with
//! how stdio `env` values are stored); moving them to the OS keychain is a
//! future hardening step.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Context, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use uuid::Uuid;

/// Fixed loopback port for the OAuth redirect. Registered with the auth server
/// via Dynamic Client Registration, so it must stay stable.
const CALLBACK_PORT: u16 = 3669;
const CALLBACK_PATH: &str = "/callback";
const LOGIN_TIMEOUT: Duration = Duration::from_secs(300);

// ---------------------------------------------------------------------------
// Token store
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthRecord {
    pub resource: String,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    #[serde(default)]
    pub registration_endpoint: Option<String>,
    pub client_id: String,
    #[serde(default)]
    pub client_secret: Option<String>,
    #[serde(default)]
    pub scope: Option<String>,
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    /// Unix seconds when the access token expires (if known).
    #[serde(default)]
    pub expires_at: Option<u64>,
}

type Store = HashMap<String, OAuthRecord>;

fn load_store(path: &Path) -> Store {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_store(path: &Path, store: &Store) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, serde_json::to_string_pretty(store)?)?;
    Ok(())
}

/// Set of server ids that currently have stored OAuth tokens.
pub fn record_ids(path: &Path) -> HashSet<String> {
    load_store(path).into_keys().collect()
}

pub fn forget(path: &Path, server_id: &str) {
    let mut store = load_store(path);
    if store.remove(server_id).is_some() {
        let _ = save_store(path, &store);
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// PKCE + randomness
// ---------------------------------------------------------------------------

fn random_base64url(byte_len: usize) -> String {
    let mut bytes = Vec::with_capacity(byte_len + 16);
    while bytes.len() < byte_len {
        bytes.extend_from_slice(Uuid::new_v4().as_bytes());
    }
    bytes.truncate(byte_len);
    URL_SAFE_NO_PAD.encode(bytes)
}

fn pkce_challenge(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    URL_SAFE_NO_PAD.encode(digest)
}

// ---------------------------------------------------------------------------
// Discovery
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct ProtectedResourceMetadata {
    #[serde(default)]
    authorization_servers: Vec<String>,
    #[serde(default)]
    resource: Option<String>,
    #[serde(default)]
    scopes_supported: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct AuthServerMetadata {
    authorization_endpoint: String,
    token_endpoint: String,
    #[serde(default)]
    registration_endpoint: Option<String>,
    #[serde(default)]
    scopes_supported: Option<Vec<String>>,
}

struct Discovery {
    resource: String,
    scope: Option<String>,
    asm: AuthServerMetadata,
}

/// `https://host[:port]/.well-known/<name>` for the given URL's origin.
fn well_known(url: &reqwest::Url, name: &str) -> Option<String> {
    let scheme = url.scheme();
    let host = url.host_str()?;
    let port = url
        .port()
        .map(|p| format!(":{p}"))
        .unwrap_or_default();
    Some(format!("{scheme}://{host}{port}/.well-known/{name}"))
}

/// Pull `resource_metadata="..."` out of a `WWW-Authenticate` header value.
fn parse_resource_metadata(www: &str) -> Option<String> {
    let key = "resource_metadata=";
    let start = www.find(key)? + key.len();
    let rest = &www[start..];
    let rest = rest.trim_start_matches('"');
    let end = rest.find(['"', ',', ' ']).unwrap_or(rest.len());
    Some(rest[..end].to_string())
}

async fn fetch_asm(client: &reqwest::Client, issuer: &reqwest::Url) -> Result<AuthServerMetadata> {
    for name in ["oauth-authorization-server", "openid-configuration"] {
        if let Some(wk) = well_known(issuer, name) {
            if let Ok(resp) = client.get(&wk).send().await {
                if resp.status().is_success() {
                    if let Ok(asm) = resp.json::<AuthServerMetadata>().await {
                        return Ok(asm);
                    }
                }
            }
        }
    }
    Err(anyhow!(
        "could not fetch OAuth authorization-server metadata from {}",
        issuer
    ))
}

async fn discover(
    client: &reqwest::Client,
    server_url: &str,
    www_authenticate: Option<String>,
) -> Result<Discovery> {
    let url = reqwest::Url::parse(server_url)?;

    // 1. Protected Resource Metadata (from the WWW-Authenticate hint or well-known).
    let prm_url = www_authenticate
        .as_deref()
        .and_then(parse_resource_metadata)
        .or_else(|| well_known(&url, "oauth-protected-resource"));

    let mut resource = server_url.to_string();
    let mut scope = None;
    let mut issuer: Option<reqwest::Url> = None;

    if let Some(prm_url) = prm_url {
        if let Ok(resp) = client.get(&prm_url).send().await {
            if resp.status().is_success() {
                if let Ok(prm) = resp.json::<ProtectedResourceMetadata>().await {
                    if let Some(r) = prm.resource {
                        resource = r;
                    }
                    scope = prm.scopes_supported.map(|s| s.join(" "));
                    if let Some(first) = prm.authorization_servers.into_iter().next() {
                        issuer = reqwest::Url::parse(&first).ok();
                    }
                }
            }
        }
    }

    // 2. Authorization Server Metadata. Fall back to treating the MCP server's
    //    own origin as the auth server (some servers are their own AS).
    let issuer = match issuer {
        Some(i) => i,
        None => reqwest::Url::parse(&format!(
            "{}://{}{}",
            url.scheme(),
            url.host_str().ok_or_else(|| anyhow!("server url has no host"))?,
            url.port().map(|p| format!(":{p}")).unwrap_or_default()
        ))?,
    };
    let asm = fetch_asm(client, &issuer).await?;
    let scope = scope.or_else(|| asm.scopes_supported.clone().map(|s| s.join(" ")));

    Ok(Discovery { resource, scope, asm })
}

// ---------------------------------------------------------------------------
// Dynamic Client Registration
// ---------------------------------------------------------------------------

async fn register_client(
    client: &reqwest::Client,
    registration_endpoint: &str,
    redirect_uri: &str,
) -> Result<(String, Option<String>)> {
    let body = json!({
        "client_name": "Meta-MCP",
        "redirect_uris": [redirect_uri],
        "grant_types": ["authorization_code", "refresh_token"],
        "response_types": ["code"],
        "token_endpoint_auth_method": "none",
        "application_type": "native"
    });
    let resp = client
        .post(registration_endpoint)
        .json(&body)
        .send()
        .await
        .context("dynamic client registration request failed")?;
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(anyhow!("client registration failed: HTTP {status} {text}"));
    }
    let v: serde_json::Value = resp.json().await?;
    let client_id = v
        .get("client_id")
        .and_then(|c| c.as_str())
        .ok_or_else(|| anyhow!("registration response had no client_id"))?
        .to_string();
    let client_secret = v
        .get("client_secret")
        .and_then(|c| c.as_str())
        .map(|s| s.to_string());
    Ok((client_id, client_secret))
}

// ---------------------------------------------------------------------------
// Authorization Code + PKCE via loopback redirect
// ---------------------------------------------------------------------------

fn open_browser(url: &str) {
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg(url).spawn();
    #[cfg(target_os = "windows")]
    let _ = std::process::Command::new("cmd").args(["/C", "start", "", url]).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let _ = std::process::Command::new("xdg-open").arg(url).spawn();
}

/// Accept loopback connections until the redirect with a matching `state`
/// arrives; return its authorization `code`.
async fn wait_for_code(listener: TcpListener, expected_state: &str) -> Result<String> {
    loop {
        let (mut stream, _) = listener.accept().await?;
        let mut buf = vec![0u8; 8192];
        let n = stream.read(&mut buf).await.unwrap_or(0);
        let req = String::from_utf8_lossy(&buf[..n]);
        let request_line = req.lines().next().unwrap_or("");
        // "GET /callback?code=...&state=... HTTP/1.1"
        let path = request_line.split_whitespace().nth(1).unwrap_or("");

        if !path.starts_with(CALLBACK_PATH) {
            // Ignore stray requests (favicon etc.).
            let _ = stream
                .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n")
                .await;
            continue;
        }

        let query = path.split_once('?').map(|(_, q)| q).unwrap_or("");
        let mut code = None;
        let mut state = None;
        let mut error = None;
        for (k, v) in url::form_urlencoded::parse(query.as_bytes()) {
            match k.as_ref() {
                "code" => code = Some(v.into_owned()),
                "state" => state = Some(v.into_owned()),
                "error" => error = Some(v.into_owned()),
                _ => {}
            }
        }

        let (status, msg) = if let Some(err) = &error {
            ("400 Bad Request", format!("Login fehlgeschlagen: {err}"))
        } else if state.as_deref() != Some(expected_state) {
            ("400 Bad Request", "Ungültiger state-Parameter.".to_string())
        } else if code.is_some() {
            ("200 OK", "Meta-MCP: Login erfolgreich. Du kannst dieses Fenster schließen.".to_string())
        } else {
            ("400 Bad Request", "Kein Authorization-Code erhalten.".to_string())
        };

        let html = format!(
            "<!doctype html><meta charset=utf-8><title>Meta-MCP</title>\
             <body style=\"font-family:-apple-system,sans-serif;background:#0d0f13;color:#e7e9ef;display:grid;place-items:center;height:100vh;margin:0\">\
             <p style=\"font-size:16px\">{msg}</p></body>"
        );
        let _ = stream
            .write_all(
                format!(
                    "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{html}",
                    html.len()
                )
                .as_bytes(),
            )
            .await;
        let _ = stream.shutdown().await;

        if let Some(err) = error {
            return Err(anyhow!("authorization denied: {err}"));
        }
        if state.as_deref() == Some(expected_state) {
            if let Some(code) = code {
                return Ok(code);
            }
        }
    }
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<u64>,
    #[serde(default)]
    scope: Option<String>,
}

#[allow(clippy::too_many_arguments)]
async fn exchange_token(
    client: &reqwest::Client,
    token_endpoint: &str,
    params: &[(&str, &str)],
    client_id: &str,
    client_secret: Option<&str>,
) -> Result<TokenResponse> {
    let mut req = client.post(token_endpoint).form(params);
    if let Some(secret) = client_secret {
        req = req.basic_auth(client_id, Some(secret));
    }
    let resp = req.send().await.context("token request failed")?;
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(anyhow!("token endpoint returned HTTP {status}: {text}"));
    }
    resp.json::<TokenResponse>()
        .await
        .context("could not parse token response")
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Run the full interactive OAuth login for a server and persist the tokens.
pub async fn login(store_path: &Path, server_id: &str, server_url: &str) -> Result<()> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()?;

    // Probe the server (unauthenticated) to read its WWW-Authenticate hint.
    let www = client
        .post(server_url)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/event-stream")
        .json(&json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": { "protocolVersion": "2025-06-18", "capabilities": {},
                        "clientInfo": { "name": "meta-mcp", "version": "0.1.1" } }
        }))
        .send()
        .await
        .ok()
        .and_then(|r| {
            r.headers()
                .get("www-authenticate")
                .and_then(|v| v.to_str().ok().map(|s| s.to_string()))
        });

    let disco = discover(&client, server_url, www).await?;
    let redirect_uri = format!("http://127.0.0.1:{CALLBACK_PORT}{CALLBACK_PATH}");

    // Reuse a stored client_id if we have one, else register dynamically.
    let store = load_store(store_path);
    let (client_id, client_secret) = match store.get(server_id) {
        Some(rec) if !rec.client_id.is_empty() => {
            (rec.client_id.clone(), rec.client_secret.clone())
        }
        _ => {
            let reg = disco
                .asm
                .registration_endpoint
                .as_deref()
                .ok_or_else(|| anyhow!(
                    "server requires OAuth but its authorization server offers no \
                     dynamic client registration; a manual client_id would be needed"
                ))?;
            register_client(&client, reg, &redirect_uri).await?
        }
    };

    // PKCE + state, then open the browser and wait for the redirect.
    let verifier = random_base64url(32);
    let challenge = pkce_challenge(&verifier);
    let state = random_base64url(16);

    let listener = TcpListener::bind(("127.0.0.1", CALLBACK_PORT))
        .await
        .map_err(|e| anyhow!("could not bind loopback :{CALLBACK_PORT} for OAuth: {e}"))?;

    let mut auth_url = reqwest::Url::parse(&disco.asm.authorization_endpoint)?;
    {
        let mut q = auth_url.query_pairs_mut();
        q.append_pair("response_type", "code");
        q.append_pair("client_id", &client_id);
        q.append_pair("redirect_uri", &redirect_uri);
        q.append_pair("code_challenge", &challenge);
        q.append_pair("code_challenge_method", "S256");
        q.append_pair("state", &state);
        q.append_pair("resource", &disco.resource);
        if let Some(scope) = &disco.scope {
            q.append_pair("scope", scope);
        }
    }
    open_browser(auth_url.as_str());

    let code = tokio::time::timeout(LOGIN_TIMEOUT, wait_for_code(listener, &state))
        .await
        .map_err(|_| anyhow!("timed out waiting for the OAuth redirect"))??;

    // Exchange the code for tokens.
    let tokens = exchange_token(
        &client,
        &disco.asm.token_endpoint,
        &[
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("redirect_uri", &redirect_uri),
            ("client_id", &client_id),
            ("code_verifier", &verifier),
            ("resource", &disco.resource),
        ],
        &client_id,
        client_secret.as_deref(),
    )
    .await?;

    let record = OAuthRecord {
        resource: disco.resource,
        authorization_endpoint: disco.asm.authorization_endpoint,
        token_endpoint: disco.asm.token_endpoint,
        registration_endpoint: disco.asm.registration_endpoint,
        client_id,
        client_secret,
        scope: tokens.scope.or(disco.scope),
        access_token: tokens.access_token,
        refresh_token: tokens.refresh_token,
        expires_at: tokens.expires_in.map(|s| now_secs() + s),
    };

    let mut store = load_store(store_path);
    store.insert(server_id.to_string(), record);
    save_store(store_path, &store)?;
    Ok(())
}

/// Return a currently-valid access token for the server, refreshing it if it has
/// expired (or is about to). `None` means the user must (re-)login.
pub async fn valid_access_token(store_path: &Path, server_id: &str) -> Option<String> {
    let mut store = load_store(store_path);
    let rec = store.get(server_id)?.clone();

    let still_valid = rec
        .expires_at
        .map(|exp| exp > now_secs() + 60)
        .unwrap_or(true);
    if still_valid {
        return Some(rec.access_token);
    }

    // Expired → try refresh.
    let refresh_token = rec.refresh_token.clone()?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .ok()?;
    let tokens = exchange_token(
        &client,
        &rec.token_endpoint,
        &[
            ("grant_type", "refresh_token"),
            ("refresh_token", &refresh_token),
            ("client_id", &rec.client_id),
            ("resource", &rec.resource),
        ],
        &rec.client_id,
        rec.client_secret.as_deref(),
    )
    .await
    .ok()?;

    let mut updated = rec;
    updated.access_token = tokens.access_token.clone();
    if tokens.refresh_token.is_some() {
        updated.refresh_token = tokens.refresh_token;
    }
    updated.expires_at = tokens.expires_in.map(|s| now_secs() + s);
    store.insert(server_id.to_string(), updated);
    let _ = save_store(store_path, &store);
    Some(tokens.access_token)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_is_base64url_sha256() {
        // Known RFC 7636 appendix B vector.
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        assert_eq!(pkce_challenge(verifier), "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
    }

    #[test]
    fn parses_resource_metadata_param() {
        let www = r#"Bearer resource_metadata="https://example.com/.well-known/oauth-protected-resource", error="invalid_token""#;
        assert_eq!(
            parse_resource_metadata(www).as_deref(),
            Some("https://example.com/.well-known/oauth-protected-resource")
        );
    }
}
