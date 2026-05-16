use anyhow::{Context, bail};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::{fs, path::Path};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BridgeConfig {
    pub server_url: String,
    pub token: String,
    pub hostname: Option<String>,
    #[serde(default)]
    pub mock_display: bool,
}

pub fn load_config(path: Option<&Path>) -> anyhow::Result<BridgeConfig> {
    if let Some(path) = path {
        return read_config(path);
    }

    if let Some(project_dirs) = ProjectDirs::from("", "", "agent-notify") {
        let path = project_dirs.config_dir().join("bridge.toml");
        if path.exists() {
            return read_config(&path);
        }
    }

    let server_url = std::env::var("AGENT_NOTIFY_SERVER")
        .unwrap_or_else(|_| "http://127.0.0.1:8787".to_string());
    let token = std::env::var("AGENT_NOTIFY_TOKEN")
        .context("set AGENT_NOTIFY_TOKEN or create bridge.toml with token")?;

    Ok(BridgeConfig {
        server_url,
        token,
        hostname: std::env::var("AGENT_NOTIFY_HOST").ok(),
        mock_display: false,
    })
}

fn read_config(path: &Path) -> anyhow::Result<BridgeConfig> {
    let content =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    let config: BridgeConfig =
        toml::from_str(&content).with_context(|| format!("failed to parse {}", path.display()))?;
    if config.server_url.trim().is_empty() {
        bail!("bridge config server_url is required");
    }
    if config.token.trim().is_empty() {
        bail!("bridge config token is required");
    }
    Ok(config)
}
