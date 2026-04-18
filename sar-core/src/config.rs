use serde::{Deserialize, Serialize};
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
    #[serde(default = "default_llm")]
    pub llm: LlmConfig,
}

fn default_llm() -> LlmConfig {
    LlmConfig::default()
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

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LlmConfig {
    #[serde(default = "default_llm_model")]
    pub model: String,
    #[serde(default = "default_llm_base_url")]
    pub base_url: String,
    #[serde(default = "default_llm_api_key")]
    pub api_key: String,
    #[serde(default = "default_llm_temperature")]
    pub temperature: f32,
    #[serde(default = "default_llm_max_tokens")]
    pub max_tokens: i32,
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            model: default_llm_model(),
            base_url: default_llm_base_url(),
            api_key: default_llm_api_key(),
            temperature: default_llm_temperature(),
            max_tokens: default_llm_max_tokens(),
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

fn default_llm_model() -> String {
    "gpt-4o-mini".to_string()
}

fn default_llm_base_url() -> String {
    "https://api.openai.com/v1".to_string()
}

fn default_llm_api_key() -> String {
    String::new()
}

fn default_llm_temperature() -> f32 {
    0.7
}

fn default_llm_max_tokens() -> i32 {
    2048
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
