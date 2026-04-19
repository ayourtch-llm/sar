# Problem: MCP Server Connection Dies Immediately

## Summary

The MCP server (`mcp-find-files`) starts, initializes, and discovers its tools successfully. However, the `RunningService` is dropped immediately after `spawn()` returns, killing the stdio transport **before any tool calls can execute**. When the LLM tries to call `find_files`, the MCP client gets "Transport closed" / "input stream terminated" errors.

## Log Evidence

```
17:44:29.972919Z  INFO sar: Spawning MCP server 'find_files'
17:44:29.978105Z  INFO rmcp::service: Service initialized as client  ← MCP handshake OK
17:44:29.978275Z  INFO sar_tool_mcp: MCP server 'find_files' started, discovering tools...
17:44:29.978785Z  INFO sar_tool_mcp: MCP server 'find_files' exposed 1 tools  ← Tool discovery OK
17:44:29.978823Z  INFO sar: MCP server 'find_files' discovered find_files tools
17:44:29.978876Z  DEBUG rmcp::service: RunningService dropped without explicit close()  ← ❌ DROPPED
17:44:29.978967Z  INFO rmcp::service: task cancelled
17:44:29.979043Z  INFO sar: Adding MCP tool to LLM loop: find_files
17:44:29.979722Z  INFO rmcp::transport::child_process: Child exited gracefully exit status: 0  ← Process dies
17:44:29.979834Z  INFO rmcp::service: serve finished quit_reason=Cancelled
```

The entire lifecycle from spawn to process death takes **~6ms**. The tool actor subscribes to `tool:find_files:execute` at line 274, but by then the transport is already dead.

## Root Cause

`RunningService` from `rmcp` contains a `DropGuard` (via `tokio_util::sync::DropGuard`) that **cancels the service** when the struct is dropped. The `RunningService` struct owns the background task that polls the transport. Without it being kept alive, the transport closes and the child process exits.

### The `RunningService` Structure (rmcp 0.16)

```rust
pub struct RunningService<R: ServiceRole, S: Service<R>> {
    service: Arc<S>,
    peer: Peer<R>,
    handle: Option<tokio::task::JoinHandle<QuitReason>>,
    cancellation_token: CancellationToken,
    dg: DropGuard,  // ← This cancels the service on drop
}
```

The `DropGuard` is initialized with `cancellation_token.clone().drop_guard()`. When `RunningService` is dropped, the `DropGuard` fires, calling `cancellation_token.cancel()`, which cancels the background task that's polling the transport.

### Why `serve_client()` Starts a Background Task

`serve_client()` returns a `RunningService` that already has a background `JoinHandle` running. This handle polls the transport for messages. If this handle is cancelled (via the `DropGuard`), the transport closes, and the child process receives EOF on stdin and exits.

## Solution: One Actor Per MCP Server

Each MCP server gets its own `McpServerActor` that owns a `JoinHandle` running `running_service.waiting()`. The actor is spawned via `bus.spawn_actor()` and stays alive for the lifetime of the program.

### Architecture

```
McpServerRunner::spawn(bus)
  ├─ TokioChildProcess::builder(cmd).spawn()  ← start child process
  ├─ serve_client(handler, transport)         ← initialize MCP client
  ├─ Peer::clone()                            ← clone peer for tool actors
  ├─ Peer::list_all_tools()                   ← discover tools
  ├─ tokio::spawn(async { rs.waiting().await })  ← keep service loop alive
  ├─ McpServerActor { peer, tools, _service_handle }
  ├─ bus.spawn_actor(actor)                   ← spawn actor (keeps it alive)
  └─ return McpServerHandle { peer, tools }   ← lightweight handle for tool_actors()
```

### Key Design Decisions

1. **`McpServerActor`** — The actor owns `_service_handle: JoinHandle<()>` which runs `running_service.waiting()`. As long as this handle is alive, the service loop keeps polling the transport.

2. **`McpServerHandle`** — A lightweight handle returned after spawning. Contains `peer` and `tools` info but NOT the service handle. Used by `main.rs` to get `tool_actors()` for the LLM loop.

3. **`McpToolActor`** — Each tool is a `ToolActor` that holds `Arc<Mutex<Peer>>`. Multiple tool actors share the same peer.

4. **Tool runners** — Spawned inside `McpServerActor::run()`, they subscribe to `tool:{name}:execute` and call tools via `Peer::call_tool()`.

## Files Involved

- `sar-tool-mcp/src/lib.rs` — `McpServerRunner`, `McpServerActor`, `McpServerHandle`, `McpToolActor`
- `src/main.rs` — Orchestrates MCP server spawning, collects tool actors for LLM loop
- `sar-llm-test-loop-tools/src/lib.rs` — Consumes tool results, sends tool calls
- `sar-llm/src/lib.rs` — Publishes tool calls to `llm:0:tool_calls` topic

## Related Issues

- Tool calls from LLM arrive but `McpToolActor::execute_tool()` fails with "Transport closed"
- The `mcp-find-files` child process exits with status 0 (graceful shutdown due to transport close)
- The `DropGuard` debug log message confirms the `RunningService` is being dropped: "RunningService dropped without explicit close()"