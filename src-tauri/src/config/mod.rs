pub mod types;

pub use types::*;

use std::path::Path;

/// Load the config from disk. Returns a default (empty) config if the file
/// does not exist or cannot be parsed.
pub fn load(path: &Path) -> Config {
    match std::fs::read_to_string(path) {
        Ok(contents) => match serde_json::from_str::<Config>(&contents) {
            Ok(cfg) => cfg,
            Err(e) => {
                eprintln!("[meta-mcp] config parse error ({}); starting empty", e);
                Config::default()
            }
        },
        Err(_) => Config::default(),
    }
}

/// Persist the config to disk (pretty-printed).
pub fn save(path: &Path, config: &Config) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(config)?;
    std::fs::write(path, json)?;
    Ok(())
}
