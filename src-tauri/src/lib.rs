mod claude;
mod commands;
mod config;
mod oauth;
mod proxy;
mod stdio_bridge;

use std::net::SocketAddr;

use tauri::Manager;
use tokio::sync::oneshot;

use proxy::{ProxyState, ProxyStateInner, PROXY_PORT};

/// Entry point for `meta-mcp --stdio` (the Claude Desktop bridge).
pub fn run_stdio_bridge() {
    stdio_bridge::run();
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let handle = app.handle().clone();

            // Resolve config path and load existing config (or default).
            let config_dir = handle.path().app_data_dir()?;
            std::fs::create_dir_all(&config_dir).ok();
            let config_path = config_dir.join("config.json");
            let config = config::load(&config_path);

            let state = ProxyStateInner::new(handle.clone(), config_path, config);
            app.manage(state.clone());

            // Watch config.json for external edits (other apps registering directly).
            proxy::spawn_config_watcher(state.clone());

            // Bind + serve the proxy in the background, signalling readiness.
            let (ready_tx, ready_rx) = oneshot::channel::<Result<(), String>>();
            let server_state = state.clone();
            tauri::async_runtime::spawn(async move {
                let addr = SocketAddr::from(([127, 0, 0, 1], PROXY_PORT));
                match tokio::net::TcpListener::bind(addr).await {
                    Ok(listener) => {
                        let _ = ready_tx.send(Ok(()));
                        let router = proxy::server::router(server_state);
                        if let Err(e) = axum::serve(listener, router).await {
                            eprintln!("[meta-mcp] proxy server stopped: {}", e);
                        }
                    }
                    Err(e) => {
                        let _ = ready_tx.send(Err(e.to_string()));
                    }
                }
            });

            // Connect active backends and report the final status.
            let boot_state = state.clone();
            tauri::async_runtime::spawn(async move {
                boot_state.set_status("starting", "Wird gestartet…").await;
                boot_state.reconcile().await;
                match ready_rx.await {
                    Ok(Ok(())) => {
                        boot_state
                            .set_status("running", &format!("SSE läuft auf :{}", PROXY_PORT))
                            .await
                    }
                    Ok(Err(e)) => {
                        boot_state
                            .set_status(
                                "error",
                                &format!("Fehler: Port {} belegt ({})", PROXY_PORT, e),
                            )
                            .await
                    }
                    Err(_) => {
                        boot_state
                            .set_status("error", "Server konnte nicht gestartet werden")
                            .await
                    }
                }
            });

            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { .. } = event {
                if let Some(state) = window.try_state::<ProxyState>() {
                    let st = state.inner().clone();
                    tauri::async_runtime::block_on(async move {
                        st.shutdown_all().await;
                    });
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_config,
            commands::get_proxy_status,
            commands::save_server,
            commands::delete_server,
            commands::set_server_active,
            commands::save_profile,
            commands::delete_profile,
            commands::set_active_profile,
            commands::import_claude_config,
            commands::get_tool_list,
            commands::get_server_status,
            commands::default_claude_config_path,
            commands::get_claude_status,
            commands::set_claude_code,
            commands::set_claude_desktop,
            commands::oauth_login,
            commands::oauth_logout,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Meta-MCP");
}
