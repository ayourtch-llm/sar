# Async Tool Execution Design

## Problem

Currently, the loop actor executes tools inline in its `tokio::select!` loop:

```rust
// Current (broken):
match tool.execute_tool(&args).await {
    Ok(result) => { ... }
}
```

This blocks the entire select! loop for the duration of the tool execution. The loop actor can't:
- Process user input (e.g., `/continue`)
- Forward LLM stream chunks
- Handle other tool calls

This is especially problematic for:
- **Sleep tools** (need to be interruptible)
- **Web fetch tools** (network latency)
- **Any long-running tool**

## Solution: Bus-based Tool Execution

### Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                        LLM Test Loop Actor                      │
│                                                                 │
│  tokio::select! {                                               │
│    input_rx.recv()  → user messages, /continue, /dump           │
│    out_rx.recv()    → LLM full responses                        │
│    stream_rx.recv() → LLM stream chunks                         │
│    tool_calls_rx.recv() → LLM tool calls                        │
│    tool_results_rx.recv() → tool results from tool actors       │
│  }                                                              │
│                                                                 │
│  When tool call received:                                        │
│    1. Parse tool call (name, args, tool_call_id)               │
│    2. Publish to bus: "tool:{name}:execute" with args           │
│    3. Store pending tool_call_id → tool_call_id                 │
│    4. Continue select! loop (not blocked!)                      │
│                                                                 │
│  When tool result received (from tool_results_rx):              │
│    1. Match result to pending tool_call_id                      │
│    2. Add to conversation history                               │
│    3. Check if all tool calls resolved                          │
│    4. If all done → send conversation back to LLM              │
└─────────────────────────────────────────────────────────────────┘
         ▲                                    │
         │                                    ▼
┌─────────────────────────────────────────────────────────────────┐
│                     Tool Actor Tasks                            │
│                                                                 │
│  ┌──────────────────────┐    ┌──────────────────────┐          │
│  │  Calculator Actor     │    │     Sleep Actor       │          │
│  │                      │    │                       │          │
│  │  subscribes to:      │    │  subscribes to:       │          │
│  │  "tool:calculator:   │    │  "tool:sleep:execute" │          │
│  │   execute"           │    │                       │          │
│  │                      │    │                       │          │
│  │  subscribes to:      │    │  subscribes to:       │          │
│  │  "tool:calculator:   │    │  "tool:sleep:cancel"  │          │
│  │   cancel" (optional) │    │                       │          │
│  │                      │    │                       │          │
│  │  on execute:         │    │  on execute:          │          │
│  │    execute logic     │    │    tokio::select! {    │          │
│  │    publish result    │    │      sleep(delay) →   │          │
│  │                      │    │        publish result │          │
│  │                      │    │      cancel →         │          │
│  │                      │    │        publish error  │          │
│  │                      │    │    }                  │          │
│  └──────────────────────┘    └──────────────────────┘          │
└─────────────────────────────────────────────────────────────────┘
```

### Message Flow

1. **User sends message** → loop actor sends to LLM
2. **LLM returns tool call** → loop actor publishes to `tool:{name}:execute`
3. **Tool actor receives** → executes, publishes result to `tool:{name}:result`
4. **Loop actor receives result** → adds to conversation, sends back to LLM

### Bus Topics

| Topic | Direction | Purpose |
|-------|-----------|---------|
| `tool:{name}:execute` | loop → tool | Execute tool with arguments |
| `tool:{name}:result` | tool → loop | Tool execution result |
| `tool:{name}:cancel` | loop → tool | Cancel long-running tool |

### Message Format

**Execute message:**
```json
{
  "tool_call_id": "call_abc123",
  "tool_name": "calculator",
  "arguments": { "expression": "2+2" }
}
```

**Result message:**
```json
{
  "tool_call_id": "call_abc123",
  "success": true,
  "result": "4",
  "error": null
}
```

**Cancel message:**
```json
{
  "tool_call_id": "call_abc123"
}
```

### Loop Actor Changes

**New fields:**
```rust
pub struct LlmTestLoopToolsActor {
    // ... existing fields ...
    
    // Track pending tool calls: tool_call_id → tool_name
    pending_tool_calls: Mutex<HashMap<String, String>>,
    
    // Number of tool calls in current batch (to know when all resolved)
    pending_count: Mutex<usize>,
    
    // Topic for tool results
    tool_results_topic: String,
}
```

**New receive branch:**
```rust
let mut tool_results_rx = bus.subscribe(&self.id(), &self.tool_results_topic).await?;

// In select!:
result = tool_results_rx.recv() => {
    match result {
        Ok(msg) => {
            // Parse result
            let result: ToolResult = serde_json::from_value(msg.payload)?;
            
            // Store result
            {
                let mut pending = self.pending_tool_calls.lock().unwrap();
                pending.insert(result.tool_call_id, result);
            }
            
            // Check if all done
            {
                let mut count = self.pending_count.lock().unwrap();
                *count -= 1;
                if *count == 0 {
                    // All tool calls resolved, send to LLM
                    self.send_conversation(bus, &messages, &tool_defs).await?;
                }
            }
        }
        // ... error handling ...
    }
}
```

**Tool call handling:**
```rust
// When receiving tool calls from LLM:
let mut tool_call_ids = Vec::new();
for tc in &tool_calls {
    let tool_call_id = tc["id"].as_str().unwrap_or("");
    let func_name = tc["function"]["name"].as_str().unwrap_or("");
    let func_args = tc["function"]["arguments"].as_str().unwrap_or("");
    
    // Publish to tool actor
    let execute_msg = Message::new(
        &format!("tool:{}:execute", func_name),
        &self.id(),
        serde_json::json!({
            "tool_call_id": tool_call_id,
            "tool_name": func_name,
            "arguments": serde_json::from_str(func_args).unwrap_or(serde_json::Value::Null),
        }),
    );
    bus.publish(&self.id(), execute_msg).await?;
    
    tool_call_ids.push(tool_call_id);
}

// Track pending
{
    let mut count = self.pending_count.lock().unwrap();
    *count = tool_calls.len();
}
```

### Tool Actor Trait

```rust
#[async_trait]
pub trait ToolActor: Send + Sync {
    fn tool_syntax(&self) -> ToolSyntax;
    
    /// Execute tool. Called by the tool actor's own task, not the loop actor.
    async fn execute_tool(&self, arguments: &serde_json::Value) -> Result<String, String>;
    
    /// Optional: handle cancellation (for long-running tools)
    fn supports_cancel(&self) -> bool { false }
}
```

### Tool Actor Base Implementation

A base actor that handles the bus subscription loop:

```rust
#[async_trait]
pub trait ToolActor: Send + Sync {
    fn tool_syntax(&self) -> ToolSyntax;
    async fn execute_tool(&self, arguments: &serde_json::Value) -> Result<String, String>;
    fn supports_cancel(&self) -> bool { false }
}

pub struct ToolActorRunner {
    actor: Arc<dyn ToolActor>,
    tool_name: String,
}

impl ToolActorRunner {
    pub fn new(actor: impl ToolActor + 'static) -> Self {
        let name = actor.tool_syntax().name.clone();
        Self {
            actor: Arc::new(actor),
            tool_name: name,
        }
    }
    
    pub async fn run(&self, bus: &SarBus) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let execute_topic = format!("tool:{}:execute", self.tool_name);
        let result_topic = format!("tool:{}:result", self.tool_name);
        let cancel_topic = format!("tool:{}:cancel", self.tool_name);
        
        let mut execute_rx = bus.subscribe("tool-actor", &execute_topic).await?;
        let mut cancel_rx = if self.actor.supports_cancel() {
            Some(bus.subscribe("tool-actor", &cancel_topic).await?)
        } else {
            None
        };
        
        info!("Tool actor '{}' listening on '{}'", self.tool_name, execute_topic);
        
        loop {
            tokio::select! {
                result = execute_rx.recv() => {
                    match result {
                        Ok(msg) => {
                            let exec_msg: ToolExecuteMessage = serde_json::from_value(msg.payload)?;
                            let result = self.actor.execute_tool(&exec_msg.arguments).await;
                            
                            let result_msg = ToolResultMessage {
                                tool_call_id: exec_msg.tool_call_id,
                                success: result.is_ok(),
                                result: result.unwrap_or_else(|e| e),
                                error: result.err(),
                            };
                            
                            let msg = Message::new(&result_topic, "tool-actor", result_msg);
                            bus.publish("tool-actor", msg).await?;
                        }
                        // ... error handling ...
                    }
                }
                Some(_) = cancel_rx.as_mut().map(|r| r.recv()) => {
                    // Handle cancellation
                }
            }
        }
    }
}
```

### Sleep Tool Example

```rust
pub struct SleepTool {
    duration_ms: u64,
}

#[async_trait]
impl ToolActor for SleepTool {
    fn tool_syntax(&self) -> ToolSyntax {
        ToolSyntax::new(
            "sleep".to_string(),
            "Sleep for a specified duration in milliseconds".to_string(),
            json!({
                "type": "object",
                "properties": {
                    "duration_ms": {
                        "type": "integer",
                        "description": "Duration in milliseconds"
                    }
                },
                "required": ["duration_ms"]
            }),
        )
    }
    
    async fn execute_tool(&self, arguments: &serde_json::Value) -> Result<String, String> {
        let duration_ms = arguments["duration_ms"].as_u64().ok_or("missing duration_ms")?;
        
        // In the runner, this would be wrapped with cancel support
        tokio::time::sleep(Duration::from_millis(duration_ms)).await;
        Ok(format!("Slept for {}ms", duration_ms))
    }
    
    fn supports_cancel(&self) -> bool { true }
}
```

### Startup Changes

In `main.rs`, each tool actor runs as its own task:

```rust
// Create loop actor
let loop_actor = LlmTestLoopToolsActor::new(...)
    .with_tool(CalculatorTool::new())
    .with_tool(SleepTool::new());

// Create tool actor runners (each runs on its own task)
let calculator_runner = ToolActorRunner::new(CalculatorTool::new());
let sleep_runner = ToolActorRunner::new(SleepTool::new());

// Spawn tool actor tasks
tokio::spawn(calculator_runner.run(bus.clone()));
tokio::spawn(sleep_runner.run(bus.clone()));

// Run loop actor
loop_actor.run(bus).await?;
```

## Implementation Plan

### Phase 1: Infrastructure
1. ✅ `ToolSyntax` struct (done)
2. ✅ `ToolActor` trait (done)
3. `ToolExecuteMessage` / `ToolResultMessage` structs
4. `ToolActorRunner` base implementation
5. `LlmTestLoopToolsActor` changes:
   - Add `pending_tool_calls` tracking
   - Add `tool_results_rx` receive branch
   - Change tool execution to publish via bus

### Phase 2: Sleep Tool
6. `SleepToolActor` with cancel support
7. Tests for sleep tool

### Phase 3: Integration
8. Update `main.rs` to spawn tool actor tasks
9. Integration tests

## Open Questions

1. **Should the loop actor know about tool result topics, or should tool actors publish to a fixed topic?**
   - Option A: `tool:{name}:result` (loop actor needs to know naming convention)
   - Option B: Fixed `tool:results` topic with tool_name in message (loop actor dispatches by name)
   - **Decision: Option A** - simpler, follows existing topic naming convention

2. **How to handle tool actor lifecycle?**
   - Tool actors run as long as the system runs
   - No dynamic tool actor registration/deregistration (tools are static)

3. **Should we support tool execution timeouts?**
   - Yes, add `timeout_ms` parameter to execute message
   - Loop actor publishes cancel after timeout

4. **Error handling for tool actors that crash?**
   - If a tool actor task panics, the loop actor receives no result
   - Need timeout mechanism to detect hung tools
   - **Decision: Add timeout to tool execution**