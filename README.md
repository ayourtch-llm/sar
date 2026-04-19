# SAR - Simple Agent in Rust

A modular agent framework with a central pub-sub message bus, pluggable actors, LLM integration, and MCP server support.

## Architecture

SAR uses an actor model where every component implements a common `Actor` trait and communicates exclusively through a central broadcast message bus. Actors are independent, concurrent tasks that subscribe to and publish messages on named topics.

```
User Input → UI Hub → LLM Actor → Tool Calls → Tool Actors → Results → LLM
```

### Core Concepts

- **SarBus** — Central pub-sub bus using `tokio::sync::broadcast` channels. Cloneable for sharing across actors.
- **Actor** — Object-safe trait with `id()` and `async run(&self, bus: &SarBus)`. Each actor runs in its own `tokio::spawn` task.
- **Message** — Contains `topic`, `source`, `payload` (serde_json::Value), `reference` (UUID), and `meta`.
- **ToolActor** — Object-safe trait for async tool execution. Tools run as independent tasks with optional cancellation support.
- **UI Hub** — Routes messages between TUI and backend actors, classifies message types for display.

## Crates

| Crate | Purpose |
|-------|---------|
| `sar` | Main orchestrator binary — loads config, creates topics, spawns all actors |
| `sar-core` | Core infrastructure — bus, actor trait, message type, config parsing |
| `sar-tui` | Terminal UI using ratatui — log area, status line, input, info panel |
| `sar-ui-hub` | Message router between TUI and backend actors |
| `sar-llm` | OpenAI-compatible LLM client with streaming and tool call support |
| `sar-llm-test` | Simple LLM test actor |
| `sar-llm-test-loop` | LLM test loop with conversation history |
| `sar-llm-test-loop-tools` | Full LLM loop with async tool execution and message buffering |
| `sar-tool-actors` | Tool actor trait and runner infrastructure |
| `sar-tool-calculator` | Calculator tool with recursive descent parser |
| `sar-tool-sleep` | Sleep tool with cancellation support |
| `sar-mcp-server` | MCP server lifecycle — one actor per server, owns service loop, spawns tool runners |
| `sar-tool-mcp` | MCP tool actor wrapper — re-exports server types, provides `McpToolActor` |
| `sar-server` | Axum web server with `/health`, `/topics`, `/publish` endpoints |
| `sar-echo` | Simple echo actor |
| `sar-reverse` | String reverse actor |
| `sar-tracing` | Tracing subscriber layer publishing log events to the bus |

## Build & Run

```bash
cargo build
cargo run                    # default config, info log level
cargo run -- -v             # verbose (debug log level)
cargo run -- --config path.toml  # custom config
cargo run -- --log /tmp/sar.log  # log to file
cargo test                    # all workspace tests
```

## Configuration

Config is TOML format with these sections:

```toml
[topics]
log = "sar:log"
input = "sar:input"

[server]
host = "127.0.0.1"
port = 3000

[llm]
model = "gpt-4o-mini"
base_url = "http://localhost:8000/v1"
api_key = ""
temperature = 0.7
max_tokens = 65536

[ui_hubs.default]
name = "default"
user_topic = "ui:user"
input_topic = "ui:input"
buffer_size = 1000
subscribe_to = ["sar:echo", "sar:reverse", "llm-test-tools:0:stream", "llm:0:stats"]
route_to = ["llm-test-tools:0:in"]

[mcp_servers.find_files]
command = ["/path/to/mcp-server"]
default = true
expose = ["tool1", "tool2"]
```

- **`[topics]`** — Named topic identifiers
- **`[llm]`** — LLM model, base URL, API key, temperature, max tokens
- **`[ui_hubs.<name>]`** — UI hub config; `subscribe_to` lists topics to display, `route_to` lists backend targets for user input
- **`[mcp_servers.<name>]`** — MCP server config; `command` is the binary to spawn, `default = true` auto-adds tools to LLM loop, `expose` selectively exposes tools

## Topics

| Topic | Purpose |
|-------|---------|
| `sar:log` | Central log topic |
| `sar:input` | Input from TUI |
| `ui:user` | Classified messages for TUI display |
| `ui:input` | User input from TUI |
| `llm:*:in` / `llm:*:out` / `llm:*:stream` / `llm:*:stats` | LLM input, output, streaming, and token stats |
| `llm:*:tool_calls` | LLM tool calls |
| `tool:{name}:execute` | Tool execution requests |
| `tool:results` | All tool results |
| `user:control` | Continue/interrupt signals |

## Tool Execution

Tools are discovered at runtime and registered with the LLM loop:

1. LLM returns tool calls → published to `llm:*:tool_calls`
2. `LlmTestLoopToolsActor` parses calls → publishes `tool:{name}:execute`
3. `ToolActorRunner` spawns independent task per call
4. Results published to `tool:results` → loop adds to conversation and sends to LLM

User messages are buffered while tool calls are pending. The `/dump` command works immediately (unbuffered).

Tools can support cancellation via `supports_cancel()`. The UI hub detects `/continue <reason>` and publishes to `user:control`.

## MCP Servers

MCP servers are defined in config.toml and spawned at startup. Each server gets its own actor that owns the service loop, keeping the stdio transport alive. Tools are discovered via `list_all_tools()` and wrapped as `McpToolActor` implementations.

## Web Server

The built-in server runs as a detached task:

```bash
curl http://127.0.0.1:3000/health
curl http://127.0.0.1:3000/api/list/topics
curl http://127.0.0.1:3000/api/list/actors
curl http://127.0.0.1:3000/api/list/announced-topics
curl -X POST http://127.0.0.1:3000/api/list/publish \
  -H "Content-Type: application/json" \
  -d '{"topic":"sar:input","source":"test","payload":"hello"}'
```

## TUI Commands

| Key / Command | Action |
|---------------|--------|
| Enter | Send message |
| Ctrl+Enter | New line |
| Up/Down | Navigate input lines |
| Esc | Toggle focus |
| Ctrl+C | Quit |
| `/quit` | Quit |
| `/target <topic>` | Change output target |
| `/bottom` | Switch to bottom panel |
| `/log <msg>` | Log a message |
| `/list actors` | List actors |
| `/list topics` | List topics |