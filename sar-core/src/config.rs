use serde::Deserialize;
use std::path::Path;
use tracing::info;

#[derive(Debug, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub topics: TopicsConfig,
    #[serde(default)]
    pub server: ServerConfig,
    #[serde(default)]
    pub ui: UiConfig,
}

#[derive(Debug, Deserialize, Default)]
pub struct TopicsConfig {
    #[serde(default = "default_log_topic")]
    pub log: String,
    #[serde(default = "default_input_topic")]
    pub input: String,
    #[serde(default = "default_echo_topic")]
    pub echo: String,
    #[serde(default = "default_server_topic")]
    pub server: String,
}

#[derive(Debug, Deserialize, Default)]
pub struct ServerConfig {
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
}

#[derive(Debug, Deserialize, Default)]
pub struct UiConfig {
    #[serde(default = "default_true")]
    pub show_bottom_panel: bool,
}

fn default_log_topic() -> String {
    "sar:log".to_string()
}

fn default_input_topic() -> String {
    "sar:input".to_string()
}

fn default_echo_topic() -> String {
    "sar:echo".to_string()
}

fn default_server_topic() -> String {
    "sar:server".to_string()
}

fn default_host() -> String {
    "127.0.0.1".to_string()
}

fn default_port() -> u16 {
    3000
}

fn default_true() -> bool {
    true
}

impl Config {
    pub fn from_file(path: &Path) -> Result<Self, ConfigError> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| ConfigError::ReadFailed(path.display().to_string(), e))?;
        let config: Config = toml::from_str(&content).map_err(|e| ConfigError::ParseFailed(e))?;
        info!("Loaded config from: {}", path.display());
        Ok(config)
    }

    pub fn from_str(content: &str) -> Result<Self, ConfigError> {
        let config: Config = toml::from_str(content).map_err(|e| ConfigError::ParseFailed(e))?;
        Ok(config)
    }
}

#[derive(thiserror::Error, Debug)]
pub enum ConfigError {
    #[error("Failed to read config file '{0}': {1}")]
    ReadFailed(String, std::io::Error),
    #[error("Failed to parse config: {0}")]
    ParseFailed(#[from] toml::de::Error),
}
