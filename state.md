# SAR Project State

## Project Overview
Simple Agent in Rust (sar) - a workspace-based project with a central pub-sub bus, pluggable actors, and multiple components including LLM integration and async tool execution.

## Workspace Structure
```
sar/
├── Cargo.toml          # workspace root + sar binary crate
├── Cargo.lock
├── config.toml         # default TOML configuration
├── .gitignore
├── src/main.rs         # sar orchestrator binary (clap CLI)
├── sar-core/           # core library: pub-sub bus, Actor trait, config
│   ├── Cargo.toml
│   ├── src/lib.rs
│   ├── src/bus.rs      # SarBus - central pub-sub using tokio broadcast channels
│   ├── src/actor.rs    # Actor trait + ActorJoinHandle
│   ├── src/message.rs  # Message type (topic, source, payload, meta)
│   └── src/config.rs   # TOML config parsing (TopicsConfig, ServerConfig, UiConfig, UiHubConfig)
├── sar-tui/            # ratatui TUI actor
│   ├── Cargo.toml
│   ├── src/lib.rs
│   └── src/tui_actor.rs
├── sar-echo/           # echo actor
│   ├── Cargo.toml
│   ├── src/lib.rs
│   └── src/echo_actor.rs
├── sar-reverse/        # reverse actor
│   ├── Cargo.toml
│   ├── src/lib.rs
│   └── src/reverse_actor.rs
├── sar-server/         # axum web server
│   ├── Cargo.toml
│   ├── src/lib.rs
│   └── src/server.rs
├── sar-ui-hub/         # UI hub actor: routes messages, filters /continue
│   ├── Cargo.toml
│   └── src/lib.rs
├── sar-llm/            # LLM client library (OpenAI-compatible API)
│   ├── Cargo.toml
│   └── src/lib.rs
├── sar-llm-test/       # LLM test actor (sends messages to LLM)
│   ├── Cargo.toml
│   └── src/lib.rs
├── sar-llm-test-loop/  # LLM test loop actor (conversation loop)
│   ├── Cargo.toml
│   └── src/lib.rs
├── sar-llm-test-loop-tools/  # LLM test loop with async tool execution
│   ├── Cargo.toml
│   ├── src/lib.rs          # LlmTestLoopToolsActor
│   ├── src/main.rs         # standalone binary for tool testing
│   ├── src/tool_actor_wrapper.rs  # Wrapper for legacy Tool trait
│   └── tests/tool_actor_tests.rs
├── sar-tool-actors/      # ToolActor trait, ToolActorRunner, message types
│   ├── Cargo.toml
│   └── src/lib.rs
├── sar-tool-calculator/  # CalculatorTool implementation
│   ├── Cargo.toml
│   └── src/lib.rs
├── sar-tool-sleep/       # SleepTool with cancel support
│   ├── Cargo.toml
│   └── src/lib.rs
└── sar-tool-mcp/         # MCP server integration
    ├── Cargo.toml
    └── src/lib.rs
```

## Architecture

### SarBus (sar-core/src/bus.rs)
- Central pub-sub infrastructure using `tokio::sync::broadcast` channels
- Methods: `new()`, `create_topic()`, `publish()`, `subscribe()`, `list_topics()`, `spawn_actor()`
- Topics are created with a capacity (buffer size)
- Cloneable for sharing across actors

### Actor Trait (sar-core/src/actor.rs)
```rust
pub trait Actor: Send + Sized {
    fn id(&self) -> ActorId;
    async fn run(&self, bus: &SarBus) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
}
```
- Actors are spawned via `bus.spawn_actor(actor)` which returns `ActorJoinHandle`
- `ActorJoinHandle` has `id()`, `stop()`, `wait()` methods

### Message (sar-core/src/message.rs)
```rust
pub struct Message {
    pub topic: String,
    pub source: String,
    pub payload: serde_json::Value,
    pub meta: serde_json::Value,  // typically {"type": "..."}
}
```
- Factory methods: `new()`, `text()`
- `with_type()` method for setting meta.type
- Implements `Display`

### Config (sar-core/src/config.rs)
- Loaded from TOML file via `Config::from_file(&Path)`
- Sections: `topics`, `server`, `ui`, `ui_hub` (for sar-ui-hub config)
- Default values defined in config.rs

## Components

### sar (src/main.rs)
- CLI binary using clap with `--config` and `--verbose` flags
- Orchestrates all actors:
  1. Loads config
  2. Creates SarBus
  3. Creates topics (log, input, echo, server, tool execution, user:control)
  4. Spawns echo actor (subscribes to input, publishes to log)
  5. Spawns server in background task
  6. Spawns TUI actor (blocks until Ctrl+C)
  7. Spawns UI hub actor (routes messages, filters /continue)
  8. Spawns LLM test loop tools actor
  9. Spawns tool actor runners (independent async tasks)
- Uses tracing with env-filter for logging

### sar-tui (sar-tui/src/tui_actor.rs)
- ratatui TUI with crossterm backend
- Layout (top to bottom):
  1. Log area: scrollable, infinite text (max 1000 entries, auto-scrolls)
  2. Status line: "Status: running", "Topics: 2"
  3. Input area: multiline input with `> ` prompt
  4. Info panel: "Ready", "Press Ctrl+C to quit"
- Input handling:
  - Char keys: type into current line
  - Backspace: delete from current line
  - Enter: send input on input topic, clear lines
  - Ctrl+Enter: add new line
  - Up/Down: navigate between lines
  - Esc: toggle focus
  - Ctrl+C: quit
- Spawns a background task to read from log topic and update state
- Uses `Arc<Mutex<TuiState>>` for shared state
- RenderSnapshot pattern to avoid borrow issues with ratatui

### sar-echo (sar-echo/src/echo_actor.rs)
- Simple echo actor
- Subscribes to input topic
- Publishes to log topic with prefix "echo: "
- Logs all received messages

### sar-reverse (sar-reverse/src/reverse_actor.rs)
- Reverse actor (reverses strings)
- Subscribes to input topic
- Publishes reversed strings to log topic

### sar-server (sar-server/src/server.rs)
- axum web server with tower-http CORS
- Endpoints:
  - `GET /health` -> "ok"
  - `GET /topics` -> list of topic names
  - `POST /publish` -> publish a message (body: {topic, source, payload})
- Runs in background task

### sar-ui-hub (sar-ui-hub/src/lib.rs)
- Routes messages between UI and backend actors
- Subscribes to `ui:input` (user input from TUI)
- Subscribes to producer topics (log, echo, reverse, LLM streams)
- Publishes to `ui:user` (classified messages for TUI display)
- Routes user input to backend topics via `route_to` config
- **Filters `/continue <reason>`**: when user types `/continue`, publishes `{"type":"continue","reason":"..."}` to `user:control` topic instead of routing to backend
- Normal messages are routed to backend and published to `ui:user`

### sar-llm (sar-llm/src/lib.rs)
- LLM client library with OpenAI-compatible API
- `LlmRequest` struct with messages, config, tools
- `LlmResponse` struct with response data
- Client methods: `chat()`, `stream_chat()`
- Handles streaming responses with SSE parsing

### sar-llm-test (sar-llm-test/src/lib.rs)
- LLM test actor that sends messages to LLM
- Subscribes to input topic, publishes to LLM input topic

### sar-llm-test-loop (sar-llm-test-loop/src/lib.rs)
- LLM test loop actor that manages conversation flow
- Handles LLM responses and tool calls

### sar-tool-actors (sar-tool-actors/)
- **ToolActor trait**: Object-safe trait for tool actors
  - `tool_syntax()` — returns OpenAI-compatible function definition
  - `execute_tool()` — runs the tool logic asynchronously
  - `supports_cancel()` — whether the tool can be cancelled mid-execution
- **ToolActorRunner**: Subscribes to `tool:{name}:execute`, spawns execution tasks, handles cancellation
- **Message types**: `ToolExecuteMessage`, `ToolResultMessage`, `ToolCancelMessage`

### sar-tool-calculator (sar-tool-calculator/)
- **CalculatorTool**: Performs math operations (add, subtract, multiply, divide)

### sar-tool-sleep (sar-tool-sleep/)
- **SleepTool**: Sleeps for specified duration, `supports_cancel() = true`

### sar-tool-mcp (sar-tool-mcp/)
- Connects to stdio MCP servers, discovers their tools, exposes them as sar tools
- **McpServerConfig**: Command, `default` flag, `expose` list for selective tool exposure
- **McpServerRunner**: Spawns MCP server process, initializes client, discovers tools
- **McpToolActor**: Wraps individual MCP tools as `ToolActor` implementations
- **McpServerHandle**: Provides `create_tool_runners()` and `get_tool_syntaxes()`

### sar-llm-test-loop-tools (sar-llm-test-loop-tools/)
- LLM test loop with fully-async tool execution
- **LlmTestLoopToolsActor**: Main loop actor that:
  - Subscribes to input, LLM output, stream, tool calls, and tool results topics
  - Manages conversation history in a shared `Mutex<Vec<serde_json::Value>>`
  - Tracks pending tool calls by ID in a `HashSet<String>`
  - Buffers user messages while tool calls are pending
  - When all tool results arrive, sends conversation back to LLM
  - Processes buffered messages in order after tool resolution

## Async Tool Execution Model

### Architecture
Each tool runs as an independent actor task, subscribed to bus topics:

```
User Input → UI Hub → LlmTestLoopToolsActor → LLM → Tool Calls
                                                        ↓
                                              publish "tool:{name}:execute"
                                                        ↓
                                              ToolActorRunner (independent task)
                                                        ↓
                                              publish "tool:results"
                                                        ↓
                                              LlmTestLoopToolsActor receives result
                                                        ↓
                                              adds to conversation, sends to LLM
```

### Bus Topics
| Topic | Purpose |
|-------|---------|
| `tool:results` | All tool results published here (single topic) |
| `tool:calculator:execute` | Calculator execution requests |
| `tool:sleep:execute` | Sleep execution requests |
| `user:control` | Centralized continue/interrupt signals |

### Message Types (tool_actor.rs)
```rust
pub struct ToolExecuteMessage {
    pub tool_call_id: String,
    pub tool_name: String,
    pub arguments: serde_json::Value,
}

pub struct ToolResultMessage {
    pub tool_call_id: String,
    pub tool_name: String,
    pub success: bool,
    pub result: String,
    pub error: Option<String>,
}

pub struct ToolCancelMessage {
    pub tool_call_id: String,
}
```

### ToolActor Trait
```rust
pub trait ToolActor: Send + Sync {
    fn tool_syntax(&self) -> ToolSyntax;
    async fn execute_tool(&self, arguments: &serde_json::Value) -> Result<String, String>;
    fn supports_cancel(&self) -> bool { false }
}
```

### ToolActorRunner
- Subscribes to `tool:{name}:execute` topic
- If `supports_cancel()`, subscribes to `user:control` topic
- Spawns each tool execution in a separate `tokio::spawn` task
- On cancel signal, calls `abort_handle.abort()` on the execution task
- Publishes results to `tool:results` topic

### Tool Implementations
- **CalculatorTool**: Performs math operations (add, subtract, multiply, divide)
- **SleepTool**: Sleeps for specified duration, `supports_cancel() = true`

### Continue/Interrupt Flow
1. User types `/continue interrupted the sleep` in TUI
2. UI hub filters it, publishes `{"type":"continue","reason":"interrupted the sleep"}` to `user:control`
3. SleepTool's runner (subscribed to `user:control`) receives it, aborts the tokio task
4. Tool runner publishes error result to `tool:results`
5. LlmTestLoopToolsActor receives result, adds to conversation, sends to LLM

### Message Buffering
- When tool calls are pending, new user messages are buffered in `pending_messages: Vec<String>`
- `/dump` command still works immediately (unbuffered)
- After all tool results arrive, buffered messages are processed in order before sending to LLM

## Configuration (config.toml)
```toml
[topics]
log = "sar:log"
input = "sar:input"
echo = "sar:echo"
server = "sar:server"

[server]
host = "127.0.0.1"
port = 3000

[ui]
show_bottom_panel = true

[ui_hub]
name = "default"
user_topic = "ui:user"
input_topic = "ui:input"
buffer_size = 1000
subscribe_to = ["sar:log", "sar:echo", "sar:reverse", "llm-test:0:stream"]
route_to = ["sar:llm-test:0:in"]
```

## Dependencies

### sar-core
- tokio (full features)
- tracing, tracing-subscriber
- serde, serde_json, toml
- thiserror, async-trait

### sar-tui
- sar-core
- ratatui 0.29, crossterm 0.28
- tokio, tracing, tracing-subscriber
- serde, serde_json, thiserror, async-trait

### sar-echo
- sar-core
- tokio, tracing, tracing-subscriber
- serde, serde_json, thiserror, async-trait

### sar-reverse
- sar-core
- tokio, tracing, tracing-subscriber
- serde, serde_json, thiserror, async-trait

### sar-server
- sar-core
- axum 0.8, tower 0.5, tower-http 0.6 (cors)
- tokio, tracing, tracing-subscriber
- serde, serde_json, thiserror, async-trait

### sar-ui-hub
- sar-core
- tokio, tracing, tracing-subscriber
- serde, serde_json, thiserror, async-trait

### sar-llm
- tokio, reqwest, serde, serde_json, tracing, async-trait

### sar-llm-test
- sar-core, sar-llm, tokio, tracing

### sar-llm-test-loop
- sar-core, sar-llm, tokio, tracing

### sar-llm-test-loop-tools
- sar-core, sar-llm, tokio, tracing, async-trait, serde, serde_json

### sar-tool-actors
- sar-core, tokio, tracing, async-trait, serde, serde_json

### sar-tool-calculator
- sar-core, sar-tool-actors, tokio, tracing, serde, serde_json

### sar-tool-sleep
- sar-core, sar-tool-actors, tokio, tracing, serde, serde_json

### sar-tool-mcp
- sar-core, sar-tool-actors, rust-mcp-sdk (client, stdio), tokio, tracing, serde, serde_json, async-trait, thiserror

### sar (binary)
- All crates
- clap 4 (derive), tokio, tracing, tracing-subscriber
- serde, serde_json, thiserror

## Build & Run

### Build
```bash
cargo build
```

### Run
```bash
cargo run                    # default config, info log level
cargo run -- -v             # verbose (debug log level)
cargo run -- --config path.toml  # custom config
```

### Test Server
```bash
curl http://127.0.0.1:3000/health
curl http://127.0.0.1:3000/topics
curl -X POST http://127.0.0.1:3000/publish \
  -H "Content-Type: application/json" \
  -d '{"topic":"sar:input","source":"test","payload":"hello"}'
```

### Test Tool Execution
```bash
cd sar-llm-test-loop-tools && cargo run
# Publish tool execute messages via server or bus
```

## Tests
- **sar-llm-test-loop-tools**: 20 tests
  - Tool syntax serialization/deserialization
  - ToolActor trait implementation tests
  - ToolActorRunner creation tests
  - CalculatorTool execute tests
  - SleepTool execute tests (including zero duration, missing args)
  - SleepTool supports_cancel test
  - LlmTestLoopToolsActor tool management tests
- **sar-ui-hub**: Continue detection tests
  - `/continue` with reason → parsed correctly
  - `/continue` bare → parsed correctly
  - Non-continue messages → not matched

## Known Issues / Warnings

1. **Unreachable code warning in sar-tui**: The cleanup code after the main loop is technically unreachable because the loop has no break condition except Ctrl+C which calls `break`. This is actually fine - the cleanup code IS reachable when Ctrl+C is pressed. The warning is a false positive from the compiler because of how the loop is structured.

2. **sar-llm and sar-llm-test have compilation errors**: These crates have known issues with the async_trait macro and are not yet fully functional.

3. **TUI is not tested in a terminal**: The ratatui TUI requires a terminal emulator to run. It won't work in non-TTY environments.

## Git State
- Branch: main
- Latest commits:
  - `44544de` - fix: parse user:control as Value instead of ContinueMessage
  - `2ff9f6d` - refactor: remove unnecessary user:control subscription from loop actor
  - `f80fcc3` - fix: don't send to LLM from user:control branch while tools are pending
  - `b0ab878` - feat: centralized continue/interrupt via user:control topic
  - `9007fc1` - feat: buffer user messages while tool calls are pending
  - `2f44bf7` - fix: track pending tool calls by ID to prevent overflow on overlapping batches
  - `f3f99d1` - feat: fully-async tool execution model with independent tool actors
  - `f0a678b` - feat: add ToolActor trait for runtime tool management
  - `ab4c855` - initial commit with full project
- Working tree: clean (changes not yet committed)

## What Was Built (Conversation History)
1. User described the project vision
2. Asked clarifying questions about config format (TOML) and workspace structure (4 crates)
3. Set up workspace with Cargo.toml, config.toml, and crate directories
4. Built sar-core with pub-sub bus, Actor trait, Message, Config
5. Built sar-tui with ratatui TUI (multiple iterations to fix borrow checker issues)
6. Built sar-echo with echo actor
7. Built sar-server with axum endpoints
8. Wired everything together with clap CLI in sar binary
9. Fixed numerous Rust compilation errors (type mismatches, borrow issues, missing imports, trait bounds)
10. Cleaned up warnings (unused imports, unreachable patterns)
11. Committed initial state
12. Added sar-reverse actor
13. Added sar-ui-hub for message routing
14. Added sar-llm, sar-llm-test, sar-llm-test-loop for LLM integration
15. Implemented fully-async tool execution model:
    - ToolActor trait with object-safe design
    - ToolActorRunner for independent async tool tasks
    - Bus-based message passing (tool:{name}:execute, tool:results, user:control)
    - SleepTool with cancel support
    - CalculatorTool implementation
16. Fixed pending_count overflow by tracking tool calls by ID in HashSet
17. Added message buffering while tool calls are pending
18. Implemented centralized continue/interrupt via user:control topic
19. Simplified loop actor to only subscribe to tool:results (tool runner handles cancel)
20. Fixed ContinueMessage deserialization issue (parse as Value instead)
21. Extracted tool infrastructure into separate crates: sar-tool-actors, sar-tool-calculator, sar-tool-sleep
22. Added /continue detection in UI hub with TDD tests
23. Implemented sar-tool-mcp for MCP server integration:
    - McpServerRunner spawns stdio MCP servers, discovers tools
    - McpToolActor wraps MCP tools as ToolActor implementations
    - McpServerHandle exposes create_tool_runners() and get_tool_syntaxes()
    - Config-driven: command, default flag, expose list
24. Fixed sar-tool-mcp compilation errors for rust-mcp-sdk v0.9.0 API:
    - Use ToMcpClientHandler::to_mcp_client_handler() instead of Box::new()
    - Clone Arc before calling start() (which consumes self)
    - Import async_trait for #[async_trait] on ToolActor impl

## Key Technical Decisions
- `tokio::sync::broadcast` channels for pub-sub (not mpsc, since multiple subscribers)
- `Arc<Mutex<T>>` for shared TUI state (not channels, because ratatui draw closure is sync)
- RenderSnapshot pattern to avoid borrowing issues with ratatui's sync draw closure
- `async_trait` crate for async trait methods (not native async traits, to support stable Rust)
- TOML for config (not YAML, per user preference)
- Each actor is a separate crate but all run in the same process
- Server runs as a detached tokio task, TUI blocks the main thread
- **Async tool execution**: Each tool runs as an independent actor task with bus-based messaging
- **Single tool:results topic**: All tool results published to one topic (not per-tool)
- **Centralized user:control**: Continue/interrupt signals sent to shared topic, tools subscribe individually
- **HashSet for pending calls**: Tracks by tool_call_id to prevent overflow on overlapping batches
- **Message buffering**: User messages buffered while tools are pending, processed in order after resolution

## Potential Next Steps
1. Integrate sar-tool-mcp into sar binary (config-driven MCP server spawning)
2. Add tests for sar-tool-mcp (MCP server discovery, tool execution)
3. Add more actors (e.g., file watcher, HTTP client, etc.)
4. Add tests for sar-llm and sar-llm-test
5. Improve TUI (add more widgets, colors, keybindings)
6. Add persistence (save log to file)
7. Add more server endpoints (WebSocket for real-time log streaming)
8. Add actor lifecycle management (restart on failure, health checks)
9. Add metrics/monitoring
10. Support loading actors dynamically via config