use serde::Deserialize;
use std::path::Path;
use tracing::info;

#[derive(Debug, Deserialize, Default)]
pub struct Config {
    #[serde(default = "default_topics")]
    pub topics: TopicsConfig,
    #[serde(default = "default_server")]
    pub server: ServerConfig,
    #[serde(default = "default_ui")]
    pub ui: UiConfig,
}

fn default_topics() -> TopicsConfig {
    TopicsConfig::default()
}

fn default_server() -> ServerConfig {
    ServerConfig::default()
}

fn default_ui() -> UiConfig {
    UiConfig::default()
}

#[derive(Debug, Deserialize)]
pub struct TopicsConfig {
    #[serde(default = "default_log_topic")]
    pub log: String,
    #[serde(default = "default_input_topic")]
    pub input: String,
    #[serde(default = "default_echo_topic")]
    pub echo: String,
    #[serde(default = "default_reverse_topic")]
    pub reverse: String,
    #[serde(default = "default_server_topic")]
    pub server: String,
}

impl Default for TopicsConfig {
    fn default() -> Self {
        Self {
            log: default_log_topic(),
            input: default_input_topic(),
            echo: default_echo_topic(),
            reverse: default_reverse_topic(),
            server: default_server_topic(),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct ServerConfig {
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: default_host(),
            port: default_port(),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct UiConfig {
    #[serde(default = "default_true")]
    pub show_bottom_panel: bool,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            show_bottom_panel: default_true(),
        }
    }
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

fn default_reverse_topic() -> String {
    "sar:reverse".to_string()
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
