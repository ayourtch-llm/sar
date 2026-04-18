use sar_core::Config;
use std::io::Write;

#[test]
fn test_config_default() {
    let config = Config::default();
    assert_eq!(config.topics.log, "sar:log");
    assert_eq!(config.topics.input, "sar:input");
    assert_eq!(config.topics.echo, "sar:echo");
    assert_eq!(config.topics.reverse, "sar:reverse");
    assert_eq!(config.topics.server, "sar:server");
    assert_eq!(config.server.host, "127.0.0.1");
    assert_eq!(config.server.port, 3000);
    assert!(config.ui.show_bottom_panel);
}

#[test]
fn test_config_from_str_valid() {
    let toml_str = r#"
[topics]
log = "custom:log"
input = "custom:input"

[server]
host = "0.0.0.0"
port = 8080

[ui]
show_bottom_panel = false
"#;
    let config = Config::from_str(toml_str).unwrap();
    assert_eq!(config.topics.log, "custom:log");
    assert_eq!(config.topics.input, "custom:input");
    assert_eq!(config.topics.echo, "sar:echo");
    assert_eq!(config.topics.reverse, "sar:reverse");
    assert_eq!(config.topics.server, "sar:server");
    assert_eq!(config.server.host, "0.0.0.0");
    assert_eq!(config.server.port, 8080);
    assert!(!config.ui.show_bottom_panel);
}

#[test]
fn test_config_from_str_empty() {
    let config = Config::from_str("").unwrap();
    assert_eq!(config.topics.log, "sar:log");
    assert_eq!(config.server.port, 3000);
}

#[test]
fn test_config_from_str_invalid() {
    let result = Config::from_str("invalid toml [[[");
    assert!(result.is_err());
}

#[test]
fn test_config_from_file_valid() {
    let toml_str = r#"
[topics]
log = "file:log"

[server]
port = 9090
"#;
    let mut file = tempfile::NamedTempFile::new().unwrap();
    write!(file, "{}", toml_str).unwrap();
    
    let config = Config::from_file(file.path()).unwrap();
    assert_eq!(config.topics.log, "file:log");
    assert_eq!(config.server.port, 9090);
}

#[test]
fn test_config_from_file_missing() {
    let result = Config::from_file(std::path::Path::new("/nonexistent/path/config.toml"));
    assert!(result.is_err());
}

#[test]
fn test_config_from_file_partial() {
    let toml_str = r#"
[server]
host = "192.168.1.1"
"#;
    let mut file = tempfile::NamedTempFile::new().unwrap();
    write!(file, "{}", toml_str).unwrap();
    
    let config = Config::from_file(file.path()).unwrap();
    assert_eq!(config.server.host, "192.168.1.1");
    assert_eq!(config.server.port, 3000);
    assert_eq!(config.topics.log, "sar:log");
}

#[test]
fn test_config_from_str_full() {
    let toml_str = r#"
[topics]
log = "log:topic"
input = "input:topic"
echo = "echo:topic"
reverse = "reverse:topic"
server = "server:topic"

[server]
host = "10.0.0.1"
port = 4000

[ui]
show_bottom_panel = true
"#;
    let config = Config::from_str(toml_str).unwrap();
    assert_eq!(config.topics.log, "log:topic");
    assert_eq!(config.topics.input, "input:topic");
    assert_eq!(config.topics.echo, "echo:topic");
    assert_eq!(config.topics.reverse, "reverse:topic");
    assert_eq!(config.topics.server, "server:topic");
    assert_eq!(config.server.host, "10.0.0.1");
    assert_eq!(config.server.port, 4000);
    assert!(config.ui.show_bottom_panel);
}