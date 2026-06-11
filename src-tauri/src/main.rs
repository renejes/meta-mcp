// Prevents an additional console window on Windows in release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // Launched as a stdio MCP server (e.g. by Claude Desktop) → run the bridge,
    // not the GUI.
    if std::env::args().skip(1).any(|a| a == "--stdio") {
        meta_mcp_lib::run_stdio_bridge();
        return;
    }
    meta_mcp_lib::run()
}
