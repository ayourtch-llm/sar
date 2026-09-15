use sar_llm_test_loop_tools::LlmTestLoopToolsActor;
use sar_tool_actors::{ToolActor, ToolActorRunner, ToolExecuteMessage, ToolResultMessage, ToolSyntax};
use sar_tool_calculator::CalculatorTool;
use sar_tool_sleep::SleepTool;
use serde_json::json;

// ============================================================
// Red tests for ToolSyntax
// ============================================================

#[test]
fn test_tool_syntax_new() {
    let syntax = ToolSyntax::new(
        "calculator".to_string(),
        "Evaluate mathematical expressions".to_string(),
        json!({
            "type": "object",
            "properties": {
                "expression": {
                    "type": "string",
                    "description": "The mathematical expression to evaluate"
                }
            },
            "required": ["expression"]
        }),
    );

    assert_eq!(syntax.name, "calculator");
    assert_eq!(syntax.description, "Evaluate mathematical expressions");
    assert_eq!(syntax.parameters["type"], "object");
}

#[test]
fn test_tool_syntax_to_openai_json() {
    let syntax = ToolSyntax::new(
        "weather".to_string(),
        "Get weather info".to_string(),
        json!({"type": "object", "properties": {}}),
    );

    let json = syntax.to_openai_json();

    assert_eq!(json["type"], "function");
    assert_eq!(json["function"]["name"], "weather");
    assert_eq!(json["function"]["description"], "Get weather info");
    assert_eq!(json["function"]["parameters"]["type"], "object");
}

#[test]
fn test_tool_syntax_serialize() {
    let syntax = ToolSyntax::new(
        "test".to_string(),
        "A test tool".to_string(),
        json!({"type": "object"}),
    );

    let serialized = serde_json::to_string(&syntax).unwrap();
    let deserialized: ToolSyntax = serde_json::from_str(&serialized).unwrap();

    assert_eq!(deserialized.name, "test");
    assert_eq!(deserialized.description, "A test tool");
}

// ============================================================
// Red tests for ToolActor trait
// ============================================================

struct MockToolActor {
    syntax: ToolSyntax,
    execute_result: Result<String, String>,
}

impl MockToolActor {
    fn new(name: &str) -> Self {
        Self {
            syntax: ToolSyntax::new(
                name.to_string(),
                format!("Mock tool: {}", name),
                json!({"type": "object", "properties": {}}),
            ),
            execute_result: Ok("mock result".to_string()),
        }
    }
}

#[async_trait::async_trait]
impl ToolActor for MockToolActor {
    fn tool_syntax(&self) -> ToolSyntax {
        self.syntax.clone()
    }

    async fn execute_tool(&self, _arguments: &serde_json::Value) -> Result<String, String> {
        self.execute_result.clone()
    }
}

#[tokio::test]
async fn test_tool_actor_syntax() {
    let tool = MockToolActor::new("test");
    let syntax = tool.tool_syntax();

    assert_eq!(syntax.name, "test");
    assert_eq!(syntax.description, "Mock tool: test");
}

#[tokio::test]
async fn test_tool_actor_execute() {
    let tool = MockToolActor::new("test");
    let result = tool.execute_tool(&json!({})).await;

    assert_eq!(result.unwrap(), "mock result");
}

#[tokio::test]
async fn test_tool_actor_execute_error() {
    let mut tool = MockToolActor::new("failing");
    tool.execute_result = Err("something went wrong".to_string());

    let result = tool.execute_tool(&json!({})).await;

    assert!(result.is_err());
    assert_eq!(result.unwrap_err(), "something went wrong");
}

// ============================================================
// Red tests for runtime add/remove tools
// ============================================================

#[tokio::test]
async fn test_add_tool_to_actor() {
    let actor = LlmTestLoopToolsActor::new(
        0,
        "test:in".to_string(),
        "llm:0:in".to_string(),
        "llm:0:out".to_string(),
        "llm:0:stream".to_string(),
        "llm:0:tool_calls".to_string(),
        "test:stream".to_string(),
    );

    let mock = MockToolActor::new("added-tool");
    actor.add_tool_arc(std::sync::Arc::new(mock)).await;

    let tools = actor.tools.lock().unwrap();
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].tool_syntax().name, "added-tool");
}

// TODO: investigate hang with std::sync::Mutex in tests
// #[tokio::test]
// async fn test_remove_tool_by_name() {
//     use sar_llm_test_loop_tools::ToolActorWrapper;
//     use sar_llm_test_loop_tools::calculator::CalculatorTool;
//
//     let actor = LlmTestLoopToolsActor::new(
//         0,
//         "test:in".to_string(),
//         "llm:0:in".to_string(),
//         "llm:0:out".to_string(),
//         "llm:0:stream".to_string(),
//         "llm:0:tool_calls".to_string(),
//         "test:stream".to_string(),
//     );
//
//     actor.add_tool(ToolActorWrapper::new(CalculatorTool::new())).await;
//
//     let tools = actor.tools.lock().unwrap();
//     assert_eq!(tools.len(), 1);
//
//     actor.remove_tool("calculator").await;
//
//     let tools = actor.tools.lock().unwrap();
//     assert_eq!(tools.len(), 0);
// }

#[tokio::test]
async fn test_remove_nonexistent_tool_does_not_panic() {
    let actor = LlmTestLoopToolsActor::new(
        0,
        "test:in".to_string(),
        "llm:0:in".to_string(),
        "llm:0:out".to_string(),
        "llm:0:stream".to_string(),
        "llm:0:tool_calls".to_string(),
        "test:stream".to_string(),
    );

    actor.add_tool_arc(std::sync::Arc::new(MockToolActor::new("only-one"))).await;
    actor.remove_tool("does-not-exist").await;

    let tools = actor.tools.lock().unwrap();
    assert_eq!(tools.len(), 1);
}

#[tokio::test]
async fn test_build_tool_defs_from_tool_actors() {
    let actor = LlmTestLoopToolsActor::new(
        0,
        "test:in".to_string(),
        "llm:0:in".to_string(),
        "llm:0:out".to_string(),
        "llm:0:stream".to_string(),
        "llm:0:tool_calls".to_string(),
        "test:stream".to_string(),
    )
    .with_tool(MockToolActor::new("tool-a"))
    .with_tool(MockToolActor::new("tool-b"));

    let tools = actor.tools.lock().unwrap();
    let defs: Vec<serde_json::Value> = tools.iter()
        .map(|t| t.tool_syntax().to_openai_json())
        .collect();

    assert_eq!(defs.len(), 2);
    assert_eq!(defs[0]["function"]["name"], "tool-a");
    assert_eq!(defs[1]["function"]["name"], "tool-b");
}

#[tokio::test]
async fn test_add_and_remove_with_calculator() {
    let actor = LlmTestLoopToolsActor::new(
        0,
        "test:in".to_string(),
        "llm:0:in".to_string(),
        "llm:0:out".to_string(),
        "llm:0:stream".to_string(),
        "llm:0:tool_calls".to_string(),
        "test:stream".to_string(),
    );

    actor.add_tool_arc(std::sync::Arc::new(CalculatorTool::new())).await;

    let tools = actor.tools.lock().unwrap();
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].tool_syntax().name, "calculator");
}

// ============================================================
// Tests for async tool execution infrastructure
// ============================================================

#[test]
fn test_tool_execute_message_serialize() {
    let msg = ToolExecuteMessage {
        tool_call_id: "call_abc123".to_string(),
        tool_name: "calculator".to_string(),
        arguments: json!({"expression": "2+2"}),
    };

    let serialized = serde_json::to_string(&msg).unwrap();
    let deserialized: ToolExecuteMessage = serde_json::from_str(&serialized).unwrap();

    assert_eq!(deserialized.tool_call_id, "call_abc123");
    assert_eq!(deserialized.tool_name, "calculator");
    assert_eq!(deserialized.arguments["expression"], "2+2");
}

#[test]
fn test_tool_result_message_serialize() {
    let msg = ToolResultMessage {
        tool_call_id: "call_abc123".to_string(),
        tool_name: "calculator".to_string(),
        success: true,
        result: "4".to_string(),
        error: None,
    };

    let serialized = serde_json::to_string(&msg).unwrap();
    let deserialized: ToolResultMessage = serde_json::from_str(&serialized).unwrap();

    assert_eq!(deserialized.tool_call_id, "call_abc123");
    assert_eq!(deserialized.tool_name, "calculator");
    assert!(deserialized.success);
    assert_eq!(deserialized.result, "4");
    assert!(deserialized.error.is_none());
}

#[test]
fn test_tool_result_message_error() {
    let msg = ToolResultMessage {
        tool_call_id: "call_xyz".to_string(),
        tool_name: "sleep".to_string(),
        success: false,
        result: String::new(),
        error: Some("Cancelled".to_string()),
    };

    let serialized = serde_json::to_string(&msg).unwrap();
    let deserialized: ToolResultMessage = serde_json::from_str(&serialized).unwrap();

    assert!(!deserialized.success);
    assert_eq!(deserialized.error, Some("Cancelled".to_string()));
}

#[test]
fn test_sleep_tool_syntax() {
    let tool = SleepTool::new();
    let syntax = tool.tool_syntax();

    assert_eq!(syntax.name, "sleep");
    assert_eq!(syntax.description, "Sleep for a specified duration in milliseconds. Useful for testing tool cancellation.");
    assert_eq!(syntax.parameters["required"][0], "duration_ms");
}

#[test]
fn test_sleep_tool_supports_cancel() {
    let tool = SleepTool::new();
    assert!(tool.supports_cancel());
}

#[tokio::test]
async fn test_sleep_tool_execute() {
    let tool = SleepTool::new();
    let result = tool.execute_tool(&json!({"duration_ms": 10})).await;

    assert!(result.is_ok());
    assert_eq!(result.unwrap(), "Slept for 10ms");
}

#[tokio::test]
async fn test_sleep_tool_execute_zero() {
    let tool = SleepTool::new();
    let result = tool.execute_tool(&json!({"duration_ms": 0})).await;

    assert!(result.is_ok());
    assert_eq!(result.unwrap(), "Slept for 0ms");
}

#[tokio::test]
async fn test_sleep_tool_execute_missing_arg() {
    let tool = SleepTool::new();
    let result = tool.execute_tool(&json!({})).await;

    assert!(result.is_err());
    assert!(result.unwrap_err().contains("duration_ms"));
}

#[test]
fn test_tool_actor_runner_creation() {
    let runner = ToolActorRunner::new(SleepTool::new());
    // The runner is created successfully - we can't easily test the run() method
    // in isolation without a bus, but we can verify the runner exists
    let _ = runner;
}

#[test]
fn test_tool_actor_runner_with_calculator() {
    let runner = ToolActorRunner::new(CalculatorTool::new());
    let _ = runner;
}

async fn wait_for_subscription(bus: &sar_core::SarBus, actor_id: &str, topic: &str) {
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            if bus
                .list_actors()
                .await
                .iter()
                .any(|a| a.id == actor_id && a.subscriptions.iter().any(|s| s == topic))
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn test_unknown_tool_batch_returns_errors_without_dispatch() {
    use sar_core::actor::{Actor, ActorAnnouncement};
    use sar_core::{Message, SarBus};

    let bus = SarBus::new();
    bus.register_announcement(ActorAnnouncement {
        id: "test".into(),
        subscriptions: vec![],
        publications: vec![],
    })
    .await;
    let actor = LlmTestLoopToolsActor::new(
        0,
        "test:in".into(),
        "llm:in".into(),
        "llm:out".into(),
        "llm:stream".into(),
        "llm:calls".into(),
        "test:stream".into(),
    );
    let actor_id = actor.id();
    let mut requests = bus.subscribe("test", "llm:in").await.unwrap();
    let mut results = bus.subscribe("test", "tool:results").await.unwrap();
    let mut executions = bus.subscribe("test", "tool:missing:execute").await.unwrap();
    let handle = bus.spawn_actor(actor).await.unwrap();
    wait_for_subscription(&bus, &actor_id, "llm-test-tools:0:grammar").await;

    let calls: Vec<_> = ["unknown_1", "unknown_2"]
        .iter()
        .map(|id| {
            json!({
                "id": id, "type": "function", "function": {"name": "missing", "arguments": "{}"}
            })
        })
        .collect();
    bus.publish("test", Message::new("llm:calls", "test", json!(calls)))
        .await
        .unwrap();
    let request = tokio::time::timeout(std::time::Duration::from_secs(2), requests.recv())
        .await
        .unwrap()
        .unwrap();
    let tool_messages: Vec<_> = request.payload["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|m| m["role"] == "tool")
        .collect();
    assert_eq!(tool_messages.len(), 2);
    for id in ["unknown_1", "unknown_2"] {
        assert!(
            tool_messages.iter().any(|m| m["tool_call_id"] == id
                && m["content"].as_str().unwrap().contains("Unknown tool"))
        );
        let result: ToolResultMessage =
            serde_json::from_value(results.try_recv().unwrap().payload).unwrap();
        assert!(!result.success);
        assert_eq!(result.tool_name, "missing");
    }
    assert!(executions.try_recv().is_err());
    assert!(requests.try_recv().is_err());
    handle.stop().await;
}

struct CancellableTool {
    started: std::sync::Arc<tokio::sync::Notify>,
    dropped: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

#[async_trait::async_trait]
impl ToolActor for CancellableTool {
    fn tool_syntax(&self) -> ToolSyntax {
        ToolSyntax::new("gated".into(), "Cancellation regression".into(), json!({}))
    }

    fn supports_cancel(&self) -> bool {
        true
    }

    async fn execute_tool(&self, _: &serde_json::Value) -> Result<String, String> {
        struct DropFlag(std::sync::Arc<std::sync::atomic::AtomicBool>);
        impl Drop for DropFlag {
            fn drop(&mut self) {
                self.0.store(true, std::sync::atomic::Ordering::SeqCst);
            }
        }
        let _guard = DropFlag(self.dropped.clone());
        self.started.notify_one();
        std::future::pending().await
    }
}

#[tokio::test]
async fn test_tool_control_filters_call_id_and_awaits_cancel() {
    use sar_core::actor::ActorAnnouncement;
    use sar_core::{Message, SarBus};
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    let bus = SarBus::new();
    bus.register_announcement(ActorAnnouncement {
        id: "test".into(),
        subscriptions: vec![],
        publications: vec![],
    })
    .await;
    let started = Arc::new(tokio::sync::Notify::new());
    let dropped = Arc::new(AtomicBool::new(false));
    let runner = ToolActorRunner::new(CancellableTool {
        started: started.clone(),
        dropped: dropped.clone(),
    });
    let mut results = bus.subscribe("test", "tool:results").await.unwrap();
    let runner_bus = bus.clone();
    let task = tokio::spawn(async move { runner.run(&runner_bus).await.unwrap() });
    wait_for_subscription(&bus, "tool-gated", "user:control").await;
    bus.publish(
        "test",
        Message::new(
            "tool:gated:execute",
            "test",
            json!({"tool_call_id": "active", "tool_name": "gated", "arguments": {}}),
        ),
    )
    .await
    .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(2), started.notified())
        .await
        .unwrap();
    bus.publish(
        "test",
        Message::new(
            "user:control",
            "test",
            json!({"type": "continue", "tool_call_id": "other"}),
        ),
    )
    .await
    .unwrap();
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(50), results.recv())
            .await
            .is_err()
    );
    assert!(!dropped.load(Ordering::SeqCst));
    bus.publish(
        "test",
        Message::new(
            "user:control",
            "test",
            json!({"type": "continue", "tool_call_id": "active", "reason": "test"}),
        ),
    )
    .await
    .unwrap();
    let result = tokio::time::timeout(std::time::Duration::from_secs(2), results.recv())
        .await
        .unwrap()
        .unwrap();
    let result: ToolResultMessage = serde_json::from_value(result.payload).unwrap();
    assert!(!result.success);
    assert_eq!(result.tool_call_id, "active");
    assert!(result.error.unwrap().contains("Cancelled"));
    assert!(
        dropped.load(Ordering::SeqCst),
        "execution must finish before cancellation is published"
    );
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
}
