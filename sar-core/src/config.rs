use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use tracing::info;

#[derive(Debug, Deserialize, Serialize)]
pub struct Config {
    #[serde(default = "default_topics")]
    pub topics: TopicsConfig,
    #[serde(default = "default_server")]
    pub server: ServerConfig,
    #[serde(default = "default_ui")]
    pub ui: UiConfig,
    #[serde(default = "default_llm")]
    pub llm: LlmConfig,
    #[serde(default)]
    pub ui_hubs: HashMap<String, UiHubConfig>,
}

impl Default for Config {
    fn default() -> Self {
        let mut hubs = HashMap::new();
        let default_hub = UiHubConfig {
            name: "default".to_string(),
            user_topic: "ui:user".to_string(),
            input_topic: "ui:input".to_string(),
            buffer_size: 1000,
            subscribe_to: vec![
                "sar:echo".to_string(),
                "sar:reverse".to_string(),
                "sar:llm:stream".to_string(),
            ],
            route_to: vec!["sar:llm-test:0:in".to_string()],
        };
        hubs.insert("default".to_string(), default_hub);
        Self {
            topics: TopicsConfig::default(),
            server: ServerConfig::default(),
            ui: UiConfig::default(),
            llm: LlmConfig::default(),
            ui_hubs: hubs,
        }
    }
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

#[derive(Debug, Deserialize, Serialize)]
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

#[derive(Debug, Deserialize, Serialize)]
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

#[derive(Debug, Deserialize, Serialize)]
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

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct UiHubConfig {
    #[serde(default = "default_ui_hub_name")]
    pub name: String,
    #[serde(default = "default_ui_hub_user_topic")]
    pub user_topic: String,
    #[serde(default = "default_ui_hub_input_topic")]
    pub input_topic: String,
    #[serde(default = "default_ui_hub_buffer_size")]
    pub buffer_size: usize,
    #[serde(default)]
    pub subscribe_to: Vec<String>,
    #[serde(default)]
    pub route_to: Vec<String>,
}

impl Default for UiHubConfig {
    fn default() -> Self {
        Self {
            name: default_ui_hub_name(),
            user_topic: default_ui_hub_user_topic(),
            input_topic: default_ui_hub_input_topic(),
            buffer_size: default_ui_hub_buffer_size(),
            subscribe_to: vec![],
            route_to: vec![],
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
    "http://ayourtch-desktop:8000/v1".to_string()
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

fn default_ui_hub_name() -> String {
    "default".to_string()
}

fn default_ui_hub_user_topic() -> String {
    "ui:user".to_string()
}

fn default_ui_hub_input_topic() -> String {
    "ui:input".to_string()
}

fn default_ui_hub_buffer_size() -> usize {
    1000
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

    pub fn to_toml(&self) -> Result<String, ConfigError> {
        let mut root = toml::Value::Table(toml::map::Map::new());

        fn to_toml_value(v: impl serde::Serialize) -> toml::Value {
            let json = serde_json::to_value(&v).unwrap();
            toml::Value::try_from(json).unwrap()
        }

        root.as_table_mut().unwrap().insert("topics".to_string(), to_toml_value(&self.topics));
        root.as_table_mut().unwrap().insert("server".to_string(), to_toml_value(&self.server));
        root.as_table_mut().unwrap().insert("ui".to_string(), to_toml_value(&self.ui));
        root.as_table_mut().unwrap().insert("llm".to_string(), to_toml_value(&self.llm));

        let mut hubs_table = toml::Value::Table(toml::map::Map::new());
        for (name, hub) in &self.ui_hubs {
            hubs_table.as_table_mut().unwrap().insert(name.clone(), to_toml_value(hub));
        }
        root.as_table_mut().unwrap().insert("ui_hubs".to_string(), hubs_table);

        toml::to_string_pretty(&root)
            .map_err(|e| ConfigError::SerializeFailed(e.to_string()))
    }
}

#[derive(thiserror::Error, Debug)]
pub enum ConfigError {
    #[error("Failed to read config file '{0}': {1}")]
    ReadFailed(String, std::io::Error),
    #[error("Failed to parse config: {0}")]
    ParseFailed(#[from] toml::de::Error),
    #[error("Failed to serialize config: {0}")]
    SerializeFailed(String),
}
