use anyhow::{Context, bail};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BridgeConfig {
    pub server_url: String,
    /// Optional in the file so it can come from `--token`/`AGENT_NOTIFY_TOKEN`
    /// instead; the merged config is checked by [`validate`].
    #[serde(default)]
    pub token: String,
    pub hostname: Option<String>,
    #[serde(default)]
    pub mock_display: bool,
}

/// Load the base config from a file or, failing that, the environment. The token
/// may be empty here: CLI flags are layered on top afterwards, and the merged
/// result is checked by [`validate`]. This ordering lets `--token`/`--server`
/// bootstrap a run with no config file or env vars present.
pub fn load_config(path: Option<&Path>) -> anyhow::Result<BridgeConfig> {
    if let Some(path) = path {
        return read_config(path);
    }

    for path in config_search_paths() {
        if path.exists() {
            return read_config(&path);
        }
    }

    Ok(BridgeConfig {
        server_url: std::env::var("AGENT_NOTIFY_SERVER")
            .unwrap_or_else(|_| "http://127.0.0.1:8787".to_string()),
        token: std::env::var("AGENT_NOTIFY_TOKEN").unwrap_or_default(),
        hostname: std::env::var("AGENT_NOTIFY_HOST").ok(),
        mock_display: false,
    })
}

/// Validate the fully-merged config (file/env + CLI overrides). Kept separate
/// from loading so a token supplied by any layer satisfies the requirement.
pub fn validate(config: &BridgeConfig) -> anyhow::Result<()> {
    if config.server_url.trim().is_empty() {
        bail!(
            "no server configured: pass --server, set AGENT_NOTIFY_SERVER, or set server_url in bridge.toml"
        );
    }
    if config.token.trim().is_empty() {
        bail!(
            "no token configured: pass --token, set AGENT_NOTIFY_TOKEN, or set token in bridge.toml at one of: {}",
            config_search_paths()
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    Ok(())
}

fn config_search_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();

    #[cfg(windows)]
    if let Some(appdata) = std::env::var_os("APPDATA") {
        paths.push(
            PathBuf::from(appdata)
                .join("agent-notify")
                .join("bridge.toml"),
        );
    }

    if let Some(project_dirs) = ProjectDirs::from("", "", "agent-notify") {
        paths.push(project_dirs.config_dir().join("bridge.toml"));
    }

    paths
}

fn read_config(path: &Path) -> anyhow::Result<BridgeConfig> {
    let content =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    toml::from_str(&content).with_context(|| format!("failed to parse {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(server: &str, token: &str) -> BridgeConfig {
        BridgeConfig {
            server_url: server.to_string(),
            token: token.to_string(),
            hostname: None,
            mock_display: false,
        }
    }

    fn temp_path(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("agent-notify-{tag}-{nanos}.toml"))
    }

    #[test]
    fn validate_accepts_complete_config() {
        assert!(validate(&sample("http://127.0.0.1:8787", "secret")).is_ok());
    }

    #[test]
    fn validate_rejects_blank_token() {
        let err = validate(&sample("http://127.0.0.1:8787", "  ")).unwrap_err();
        assert!(err.to_string().contains("no token configured"));
    }

    #[test]
    fn validate_rejects_blank_server() {
        let err = validate(&sample("", "secret")).unwrap_err();
        assert!(err.to_string().contains("no server configured"));
    }

    #[test]
    fn cli_token_bootstraps_base_without_token() {
        // Mirrors main(): load_config's env path leaves the token empty when
        // AGENT_NOTIFY_TOKEN is unset, and a --token override then validates.
        let mut config = sample("http://127.0.0.1:8787", "");
        assert!(validate(&config).is_err());
        config.token = "from-cli".to_string();
        assert!(validate(&config).is_ok());
    }

    #[test]
    fn read_config_allows_token_from_other_layers() {
        let path = temp_path("missing-token");
        fs::write(&path, "server_url = \"http://example:8787\"\n").unwrap();
        let config = read_config(&path).unwrap();
        fs::remove_file(&path).ok();
        assert_eq!(config.server_url, "http://example:8787");
        // Defaulted; a CLI flag or env var supplies it before validation.
        assert_eq!(config.token, "");
    }

    #[test]
    fn read_config_parses_all_fields() {
        let path = temp_path("full");
        fs::write(
            &path,
            "server_url = \"http://example:8787\"\ntoken = \"secret\"\nhostname = \"box\"\nmock_display = true\n",
        )
        .unwrap();
        let config = read_config(&path).unwrap();
        fs::remove_file(&path).ok();
        assert_eq!(config.token, "secret");
        assert_eq!(config.hostname.as_deref(), Some("box"));
        assert!(config.mock_display);
    }
}
