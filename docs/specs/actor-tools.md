# Actor Tools Specification

## Overview

This spec defines the `ToolActor` trait that allows tools to be added/removed at runtime from the `LlmTestLoopToolsActor`, while still supporting the existing `Tool` trait for inline tool execution.

## Goals

1. Define a `ToolActor` trait that extends `Actor` with tool metadata and execution
2. Support runtime add/remove of tools via `Mutex<Vec<Box<dyn ToolActor>>>`
3. Allow the loop actor to query tool syntax directly (no bus hop) for building LLM tool definitions
4. Allow tool execution to be async via the bus for fault isolation and concurrency
5. Maintain backward compatibility with the existing `Tool` trait and `CalculatorTool`

## Design

### `ToolSyntax` Struct

A serializable struct representing a tool's OpenAI-compatible function definition:

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolSyntax {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}
```

This is the structured equivalent of the JSON currently built inline in `format_dump()` and `send_conversation()`.

### `ToolActor` Trait

```rust
#[async_trait]
pub trait ToolActor: Actor {
    fn tool_syntax(&self) -> ToolSyntax;
    async fn execute_tool(&self, arguments: &serde_json::Value) -> Result<String, String>;
}
```

- `tool_syntax()` — returns the tool's metadata for sending to the LLM. Called synchronously (cheap, no bus hop).
- `execute_tool()` — executes the tool logic. Called asynchronously (can be routed over bus later).

### `LlmTestLoopToolsActor` Changes

The actor holds tools as `Mutex<Vec<Box<dyn ToolActor>>>` instead of `Vec<Box<dyn Tool>>`:

```rust
pub struct LlmTestLoopToolsActor {
    // ... existing fields ...
    pub tools: Mutex<Vec<Box<dyn ToolActor>>>,
}
```

New methods:

```rust
impl LlmTestLoopToolsActor {
    pub async fn add_tool(&self, tool: impl ToolActor + 'static) {
        self.tools.lock().await.push(Box::new(tool));
    }

    pub async fn remove_tool(&self, name: &str) {
        self.tools.lock().await.retain(|t| t.tool_syntax().name != name);
    }
}
```

Existing `with_tool()` builder method updated to use `ToolActor`:

```rust
pub fn with_tool(mut self, tool: impl ToolActor + 'static) -> Self {
    self.tools = tokio::sync::Mutex::new(vec![Box::new(tool)]);
    self
}
```

Wait — the builder pattern uses `self` by value, so we can't use `Mutex` in the struct during construction. Instead, we always store as `Mutex` and the builder creates the initial vec inside the mutex.

Actually, looking at the existing code more carefully: the actor is cloned via `Arc` when spawned, and `self` is borrowed in `run()`. The `tools` field is accessed via `&self.tools` which is `Vec<Box<dyn Tool>>`. To support both the builder pattern (which takes `self` by value) and runtime add/remove, we should always store as `Mutex`:

```rust
// In struct definition:
pub tools: Mutex<Vec<Box<dyn ToolActor>>>,

// In builder:
pub fn with_tool(mut self, tool: impl ToolActor + 'static) -> Self {
    self.tools = Mutex::new(vec![Box::new(tool)]);
    self
}
```

But wait — `with_tool` is called multiple times in a chain. Each call creates a new `Mutex`. That's fine since it's constructing the actor.

### Backward Compatibility: `ToolActor` for `CalculatorTool`

The existing `CalculatorTool` implements `Tool`. We provide a wrapper or a separate actor type. Two options:

**Option A: Wrapper type**

```rust
pub struct ToolActorWrapper<T: Tool> {
    tool: T,
    actor_id: String,
}

impl<T: Tool> Actor for ToolActorWrapper<T> {
    fn id(&self) -> ActorId { self.actor_id.clone() }
    async fn run(&self, _bus: &SarBus) -> Result<(), ...> { Ok(()) }
}

impl<T: Tool> ToolActor for ToolActorWrapper<T> {
    fn tool_syntax(&self) -> ToolSyntax {
        ToolSyntax {
            name: self.tool.name().to_string(),
            description: self.tool.description().to_string(),
            parameters: self.tool.parameters().clone(),
        }
    }
    async fn execute_tool(&self, args: &Value) -> Result<String, String> {
        self.tool.execute(args).await
    }
}
```

**Option B: Implement `ToolActor` directly on `CalculatorTool`**

This requires `CalculatorTool` to implement `Actor` (which needs an `id()` and `run()`). Since `CalculatorTool` is stateless, `run()` would just be a no-op infinite loop or return immediately.

Option A is cleaner — it keeps `CalculatorTool` as a pure `Tool` implementation and wraps it for the actor system. But it adds indirection.

**Decision: Option A (wrapper)** — it's cleaner separation of concerns. The `Tool` trait stays as-is for pure tool logic. `ToolActor` is for actors that expose tool capabilities.

### Tool Execution Flow (Current vs Future)

**Current (inline):**
```
loop actor receives tool call → find_tool(name) → tool.execute(args) → result
```

**Future (async bus):**
```
loop actor receives tool call → publish tool call message to bus → tool actor receives → execute → publish result → loop actor receives result
```

This spec only implements the infrastructure for both. The execution stays inline for now (via `execute_tool()` called directly), but the trait is designed so that `execute_tool()` can later be invoked via bus messages by a separate tool actor.

## Files to Modify/Create

1. **`sar-llm-test-loop-tools/src/tool_actor.rs`** (new) — `ToolSyntax` struct and `ToolActor` trait
2. **`sar-llm-test-loop-tools/src/lib.rs`** — update `LlmTestLoopToolsActor` to use `ToolActor`
3. **`sar-llm-test-loop-tools/src/calculator.rs`** — add `ToolActorWrapper` or implement `ToolActor`
4. **`sar-llm-test-loop-tools/src/main.rs`** — update to use new API
5. **`src/main.rs`** — update tool registration
6. **`sar-llm-test-loop-tools/tests/tool_actor_tests.rs`** (new) — tests

## Testing Strategy (TDD)

1. **Red:** Write tests for `ToolSyntax` (serialization, fields)
2. **Green:** Implement `ToolSyntax`
3. **Red:** Write tests for `ToolActor` trait with a mock tool
4. **Green:** Implement `ToolActor` trait
5. **Red:** Write tests for runtime `add_tool` / `remove_tool`
6. **Green:** Implement `Mutex<Vec<Box<dyn ToolActor>>>` with add/remove
7. **Red:** Write integration test: loop actor builds tool defs from `ToolActor`s
8. **Green:** Update `LlmTestLoopToolsActor` to use `ToolActor`
9. **Code review** of all changes