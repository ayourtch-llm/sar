//! MCP server integration for SAR.
//!
//! Re-exports server lifecycle types from `sar-mcp-server` and provides
//! the `McpServerConfig` type alias for config parsing.

pub use sar_mcp_server::{
    McpClientHandler, McpServerActor, McpServerHandle, McpServerRunner,
};

pub use sar_core::config::McpServerConfig;