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

const DEFAULT_SERVER: &str = "http://127.0.0.1:8787";

/// Load the merged file + environment config. Per-field precedence is the config
/// file first, then the environment, then the built-in default, so a `bridge.toml`
/// that sets only `server_url` still picks up `AGENT_NOTIFY_TOKEN` from the
/// environment instead of the file shadowing the environment wholesale. CLI flags
/// are layered on top by `main`, and the final result is checked by [`validate`];
/// the token may be empty here.
pub fn load_config(path: Option<&Path>) -> anyhow::Result<BridgeConfig> {
    let env = env_config();
    match load_file(path)? {
        Some(file) => Ok(overlay_file(env, file)),
        None => Ok(env),
    }
}

/// Lowest layer: environment variables, with the built-in default server. The
/// token is empty when `AGENT_NOTIFY_TOKEN` is unset.
fn env_config() -> BridgeConfig {
    BridgeConfig {
        server_url: std::env::var("AGENT_NOTIFY_SERVER")
            .unwrap_or_else(|_| DEFAULT_SERVER.to_string()),
        token: std::env::var("AGENT_NOTIFY_TOKEN").unwrap_or_default(),
        hostname: std::env::var("AGENT_NOTIFY_HOST").ok(),
        mock_display: false,
    }
}

/// Overlay the file's values onto the environment-derived `base`, for each field
/// the file actually provides. Fields the file omits (or leaves blank) keep the
/// environment value, so the two layers compose.
fn overlay_file(mut base: BridgeConfig, file: BridgeConfig) -> BridgeConfig {
    if !file.server_url.trim().is_empty() {
        base.server_url = file.server_url;
    }
    if !file.token.trim().is_empty() {
        base.token = file.token;
    }
    if file.hostname.is_some() {
        base.hostname = file.hostname;
    }
    if file.mock_display {
        base.mock_display = true;
    }
    base
}

/// Read the explicit `--config` path (which must exist) or the first discovered
/// `bridge.toml`, returning `None` when no config file is present.
fn load_file(path: Option<&Path>) -> anyhow::Result<Option<BridgeConfig>> {
    if let Some(path) = path {
        return read_config(path).map(Some);
    }
    for candidate in config_search_paths() {
        if candidate.exists() {
            return read_config(&candidate).map(Some);
        }
    }
    Ok(None)
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
    fn file_without_token_keeps_env_token() {
        // The reported case: bridge.toml sets server_url, the secret comes from
        // AGENT_NOTIFY_TOKEN. The env token must survive the file overlay.
        let env = sample("http://env:8787", "env-token");
        let file = sample("http://file:8787", "");
        let merged = overlay_file(env, file);
        assert_eq!(merged.server_url, "http://file:8787");
        assert_eq!(merged.token, "env-token");
    }

    #[test]
    fn file_values_override_env_when_present() {
        let mut env = sample("http://env:8787", "env-token");
        env.hostname = Some("env-host".to_string());
        let mut file = sample("http://file:8787", "file-token");
        file.hostname = Some("file-host".to_string());
        file.mock_display = true;
        let merged = overlay_file(env, file);
        assert_eq!(merged.server_url, "http://file:8787");
        assert_eq!(merged.token, "file-token");
        assert_eq!(merged.hostname.as_deref(), Some("file-host"));
        assert!(merged.mock_display);
    }

    #[test]
    fn file_omitting_fields_keeps_env_values() {
        let mut env = sample("http://env:8787", "env-token");
        env.hostname = Some("env-host".to_string());
        // A file that only sets server_url leaves the rest to the environment.
        let file = sample("http://file:8787", "");
        let merged = overlay_file(env, file);
        assert_eq!(merged.token, "env-token");
        assert_eq!(merged.hostname.as_deref(), Some("env-host"));
        assert!(!merged.mock_display);
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
