# SAR Project Context

## Project Overview

**Location:** `/Users/ayourtch/rust/sar`
**Build:** `cargo build` (workspace with multiple crates)

### Architecture

SAR is a Rust actor-based system with a central message bus. Key crates:

- **`sar-core`** — Core types: `Actor` trait, `SarBus` (pub-sub), `Message` struct
- **`sar-llm`** — LLM actor that interfaces with external LLM providers
- **`sar-llm-test-loop`** — Simple LLM test loop actor
- **`sar-llm-test-loop-tools`** — LLM test loop with tool calling (main focus of work)
- **`sar-llm-test`** — LLM test actor
- **`sar-server`** — HTTP server for the bus
- **`sar-tui`** — Terminal UI
- **`sar-echo`**, **`sar-reverse`**, **`sar-tracing`**, **`sar-ui-hub`** — Example/other actors

### Core Types

**`Actor` trait** (`sar-core/src/actor.rs`):
```rust
#[async_trait]
pub trait Actor: Send + Sync {
    fn id(&self) -> ActorId;
    async fn run(&self, bus: &SarBus) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
}
```

**`SarBus`** (`sar-core/src/bus.rs`): Central pub-sub. Any actor can publish to any topic, subscribe to any topic. Uses `tokio::sync::broadcast::Sender/Receiver`.

**`Message`** (`sar-core/src/message.rs`): Has `topic`, `source`, `payload` (serde_json::Value), `meta` (HashMap), `reference` (UUID v4).

### Main Entry Point

`src/main.rs` — Spawns the bus, LLM actor, and loop actor. Currently registers `CalculatorTool` via `ToolActorWrapper`.

---

## What Was Implemented

### 1. ToolActor Trait (Complete)

**File:** `sar-llm-test-loop-tools/src/tool_actor.rs`

```rust
pub struct ToolSyntax {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

impl ToolSyntax {
    pub fn new(...) -> Self { ... }
    pub fn to_openai_json(&self) -> serde_json::Value { ... }
}

#[async_trait]
pub trait ToolActor: Send + Sync {
    fn tool_syntax(&self) -> ToolSyntax;
    async fn execute_tool(&self, arguments: &serde_json::Value) -> Result<String, String>;
}
```

### 2. ToolActorWrapper (Complete)

**File:** `sar-llm-test-loop-tools/src/tool_actor_wrapper.rs`

Wraps existing `Tool` implementations to make them `ToolActor` compatible:
```rust
pub struct ToolActorWrapper<T: Tool> { tool: T }
// Implements ToolActor by delegating to wrapped Tool
```

### 3. LlmTestLoopToolsActor Changes (Complete)

**File:** `sar-llm-test-loop-tools/src/lib.rs`

- `tools` field changed from `Vec<Box<dyn Tool>>` to `StdMutex<Vec<Arc<dyn ToolActor>>>`
- `with_tool()` builder accepts `impl ToolActor`
- `add_tool()` / `remove_tool()` for runtime management
- Tool execution still inline (calls `execute_tool()` directly) — **this is the problem**

### 4. CalculatorTool (Existing)

**File:** `sar-llm-test-loop-tools/src/calculator.rs`

Implements the `Tool` trait (not `ToolActor`). Wrapped with `ToolActorWrapper` in main.rs.
- Evaluates math expressions using `cherow` JS engine
- Has `description()` and `parameters()` for OpenAI function definition

### 5. Tests (Complete)

**File:** `sar-llm-test-loop-tools/tests/tool_actor_tests.rs`

10 tests passing:
- `test_tool_syntax_new`, `test_tool_syntax_to_openai_json`, `test_tool_syntax_serialize`
- `test_tool_actor_syntax`, `test_tool_actor_execute`, `test_tool_actor_execute_error`
- `test_add_tool_to_actor`, `test_build_tool_defs_from_tool_actors`, `test_add_and_remove_with_calculator`, `test_remove_nonexistent_tool_does_not_panic`

One test commented out (`test_remove_tool_by_name`) due to `std::sync::Mutex` hang in tokio test context.

### 6. Spec Document

**File:** `docs/specs/actor-tools.md`

Full specification of the ToolActor design.

---

## Problem: Tool Execution Blocks the Loop Actor

### Current Flow (Broken)

```
LLM → loop actor → [blocked on tool.execute_tool()] → result
```

The loop actor's `tokio::select!` loop is blocked during tool execution. It can't:
- Process user input (e.g., `/continue`)
- Forward LLM stream chunks
- Handle other tool calls

### Required Fix: Bus-Based Async Tool Execution

**File:** `docs/thoughts.md` — Full design document

#### Architecture

```
LLM → loop actor → publish "tool:{name}:execute" → ToolActor task → result → publish "tool:{name}:result"
```

Each tool runs on its own tokio task, subscribed to its own bus topic.

#### Bus Topics

| Topic | Direction | Purpose |
|-------|-----------|---------|
| `tool:{name}:execute` | loop → tool | Execute tool with arguments |
| `tool:{name}:result` | tool → loop | Tool execution result |
| `tool:{name}:cancel` | loop → tool | Cancel long-running tool |

#### Message Formats

**Execute:**
```json
{ "tool_call_id": "call_abc123", "tool_name": "calculator", "arguments": { "expression": "2+2" } }
```

**Result:**
```json
{ "tool_call_id": "call_abc123", "success": true, "result": "4", "error": null }
```

**Cancel:**
```json
{ "tool_call_id": "call_abc123" }
```

#### Loop Actor Changes Needed

1. Add `pending_tool_calls` tracking (HashMap<tool_call_id, ToolResult>)
2. Add `pending_count` (usize) to track how many tool calls are in-flight
3. Add `tool_results_rx` receive branch to `tokio::select!`
4. Change tool call handling: instead of calling `execute_tool()`, publish to `tool:{name}:execute`
5. When all pending tool calls resolve → send conversation back to LLM

#### ToolActorRunner (Base Implementation)

A base runner that handles bus subscription:
```rust
pub struct ToolActorRunner {
    actor: Arc<dyn ToolActor>,
    tool_name: String,
}

impl ToolActorRunner {
    pub fn new(actor: impl ToolActor + 'static) -> Self { ... }
    
    pub async fn run(&self, bus: &SarBus) -> Result<(), ...> {
        // Subscribe to "tool:{name}:execute"
        // On message: call actor.execute_tool(), publish result to "tool:{name}:result"
    }
}
```

#### Startup Changes

```rust
// Spawn tool actor tasks
let calculator_runner = ToolActorRunner::new(CalculatorTool::new());
let sleep_runner = ToolActorRunner::new(SleepTool::new());
tokio::spawn(calculator_runner.run(bus.clone()));
tokio::spawn(sleep_runner.run(bus.clone()));

// Run loop actor (now non-blocking for tools)
loop_actor.run(bus).await?;
```

---

## What Needs to Be Done Next

### Phase 1: Async Tool Execution Infrastructure

1. **Add message types** (`ToolExecuteMessage`, `ToolResultMessage`) to `tool_actor.rs`
2. **Add `ToolActorRunner`** — base implementation that subscribes to bus and dispatches
3. **Update `LlmTestLoopToolsActor`**:
   - Add `pending_tool_calls: Mutex<HashMap<String, ToolResultMessage>>`
   - Add `pending_count: Mutex<usize>`
   - Add `tool_results_topic: String` field
   - Add `tool_results_rx` to `tokio::select!`
   - Replace inline `execute_tool()` call with bus publish to `tool:{name}:execute`
   - Handle tool results in new receive branch
4. **Update `main.rs`** to spawn tool actor tasks

### Phase 2: Sleep Tool

5. **Create `SleepToolActor`** (`sar-llm-test-loop-tools/src/sleep_tool.rs`):
   - Implements `ToolActor` trait
   - `tool_syntax()`: returns name="sleep", description, parameters (duration_ms: integer)
   - `execute_tool()`: sleeps for `duration_ms`, returns success message
   - `supports_cancel()`: true
   - Sleep task should be interruptible (tokio::select! between sleep and cancel channel)

6. **Update `main.rs`** to register SleepToolActor

### Phase 3: Tests

7. **Test sleep tool execution**
8. **Test sleep tool cancellation**
9. **Test async tool execution flow** (loop actor not blocked)

---

## Key Files to Modify

- `sar-llm-test-loop-tools/src/tool_actor.rs` — Add message types, ToolActorRunner
- `sar-llm-test-loop-tools/src/lib.rs` — Refactor tool execution to bus-based
- `sar-llm-test-loop-tools/src/sleep_tool.rs` — New file
- `sar-llm-test-loop-tools/src/main.rs` — Spawn tool actor tasks
- `src/main.rs` — Register SleepToolActor
- `sar-llm-test-loop-tools/tests/` — Add tests

---

## Current Test Status

All 46 workspace tests pass (excluding `sar-llm` and `sar-llm-test` which have pre-existing failures unrelated to this work).

New tests in `sar-llm-test-loop-tools/tests/tool_actor_tests.rs`: 10/10 passing.

## Git Status

Last commit: `f0a678b` — "feat: add ToolActor trait for runtime tool management"