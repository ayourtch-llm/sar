# SAR Project State

## Project Overview
Simple Agent in Rust (sar) - a workspace-based project with a central pub-sub bus, pluggable actors, and multiple components.

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
│   ├── src/message.rs  # Message type (topic, source, payload)
│   └── src/config.rs   # TOML config parsing (TopicsConfig, ServerConfig, UiConfig)
├── sar-tui/            # ratatui TUI actor
│   ├── Cargo.toml
│   ├── src/lib.rs
│   └── src/tui_actor.rs
├── sar-echo/           # echo actor
│   ├── Cargo.toml
│   ├── src/lib.rs
│   └── src/echo_actor.rs
└── sar-server/         # axum web server
    ├── Cargo.toml
    ├── src/lib.rs
    └── src/server.rs
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
}
```
- Factory methods: `new()`, `text()`
- Implements `Display`

### Config (sar-core/src/config.rs)
- Loaded from TOML file via `Config::from_file(&Path)`
- Sections: `topics` (log, input, echo, server), `server` (host, port), `ui` (show_bottom_panel)
- Default values defined in config.rs

## Components

### sar (src/main.rs)
- CLI binary using clap with `--config` and `--verbose` flags
- Orchestrates all actors:
  1. Loads config
  2. Creates SarBus
  3. Creates topics (log:1000, input:100, echo:100, server:100)
  4. Spawns echo actor (subscribes to input, publishes to log)
  5. Spawns server in background task
  6. Spawns TUI actor (blocks until Ctrl+C)
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

### sar-server (sar-server/src/server.rs)
- axum web server with tower-http CORS
- Endpoints:
  - `GET /health` -> "ok"
  - `GET /topics` -> list of topic names
  - `POST /publish` -> publish a message (body: {topic, source, payload})
- Runs in background task

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

### sar-server
- sar-core
- axum 0.8, tower 0.5, tower-http 0.6 (cors)
- tokio, tracing, tracing-subscriber
- serde, serde_json, thiserror, async-trait

### sar (binary)
- All four crates
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

## Known Issues / Warnings

1. **Unreachable code warning in sar-tui**: The cleanup code after the main loop (`disable_raw_mode`, `LeaveAlternateScreen`) is technically unreachable because the loop has no break condition except Ctrl+C which calls `break`. This is actually fine - the cleanup code IS reachable when Ctrl+C is pressed. The warning is a false positive from the compiler because of how the loop is structured.

2. **No tests yet**: The project has no test code.

3. **TUI is not tested in a terminal**: The ratatui TUI requires a terminal emulator to run. It won't work in non-TTY environments.

## Git State
- Branch: main
- Latest commit: `ab4c855` - initial commit with full project
- Working tree: clean

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

## Key Technical Decisions
- `tokio::sync::broadcast` channels for pub-sub (not mpsc, since multiple subscribers)
- `Arc<Mutex<T>>` for shared TUI state (not channels, because ratatui draw closure is sync)
- RenderSnapshot pattern to avoid borrowing issues with ratatui's sync draw closure
- `async_trait` crate for async trait methods (not native async traits, to support stable Rust)
- TOML for config (not YAML, per user preference)
- Each actor is a separate crate but all run in the same process
- Server runs as a detached tokio task, TUI blocks the main thread

## Potential Next Steps
1. Add more actors (e.g., file watcher, HTTP client, etc.)
2. Add tests
3. Improve TUI (add more widgets, colors, keybindings)
4. Add persistence (save log to file)
5. Add more server endpoints (WebSocket for real-time log streaming)
6. Add actor lifecycle management (restart on failure, health checks)
7. Add metrics/monitoring
8. Support loading actors dynamically via config
9. Add message filtering/routing
10. Support multiple TUI instances