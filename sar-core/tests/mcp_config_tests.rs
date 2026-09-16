#[test]
fn mcp_command_arguments_and_env_round_trip() {
    let config = sar_core::config::Config::from_str(r#"
[mcp_servers.llm_search]
command = ["mcp-llm-search", "--max-steps", "24"]
expose = ["search"]
[mcp_servers.llm_search.env]
LLM_MODEL = "research-model"
"#).unwrap();
    let server = &config.mcp_servers["llm_search"];
    assert_eq!(server.command[2], "24");
    assert_eq!(server.env["LLM_MODEL"], "research-model");
    let copy = sar_core::config::Config::from_str(&config.to_toml().unwrap()).unwrap();
    assert_eq!(copy.mcp_servers["llm_search"].env, server.env);
}
