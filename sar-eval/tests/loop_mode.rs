use sar_core::{actor::ActorAnnouncement, bus::SarBus, config::LlmConfig};
use sar_tool_actors::{ToolActor, ToolActorRunner, ToolSyntax};
use serde_json::{json, Value};
use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};

struct FixtureTool {
    name: String,
    calls: Arc<AtomicUsize>,
}
#[async_trait::async_trait]
impl ToolActor for FixtureTool {
    fn tool_syntax(&self) -> ToolSyntax {
        ToolSyntax::new(
            self.name.clone(),
            "Fixture research tool".into(),
            json!({"type":"object","properties":{"question":{"type":"string"}},"required":["question"]}),
        )
    }
    async fn execute_tool(&self, _: &Value) -> Result<String, String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(json!({"answer":"fixture answer","sources":["https://example.org"],"status":"final_answer","steps":2,"searches":1,"fetches":0,"prompt_tokens":20,"completion_tokens":10,"wall_ms":1,"trace_path":"fixture","live_tavily_calls":0,"live_tavily_calls_total":0}).to_string())
    }
}

#[tokio::test]
async fn outer_sar_loop_exposes_only_search_and_uses_bus_results() -> anyhow::Result<()> {
    let app = axum::Router::new().route("/chat/completions",axum::routing::post(|axum::Json(body):axum::Json<Value>| async move {
        assert_eq!(body["tools"].as_array().unwrap().len(),1);
        assert_eq!(body["tools"][0]["function"]["name"],"search");
        assert_eq!(body["model"],"outer-fixture");
        let searched = body["messages"].as_array().unwrap().iter().any(|m|m["role"]=="tool");
        let delta = if searched { json!({"content":"fixture answer"}) } else { json!({"tool_calls":[{"index":0,"id":"call1","function":{"name":"search","arguments":"{\"question\":\"fixture\"}"}}]}) };
        ([ ("content-type","text/event-stream") ],format!("data: {}\n\ndata: [DONE]\n\n",json!({"choices":[{"delta":delta}]})))
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let http = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let bus = SarBus::new();
    bus.register_announcement(ActorAnnouncement {
        id: "sar-eval".into(),
        subscriptions: vec![],
        publications: vec![],
    })
    .await;
    let calls = Arc::new(AtomicUsize::new(0));
    let runner = ToolActorRunner::new(FixtureTool {
        name: "search".into(),
        calls: calls.clone(),
    });
    let b = bus.clone();
    let runner_task = tokio::spawn(async move {
        runner.run(&b).await.unwrap();
    });
    let tools: Vec<Arc<dyn ToolActor>> = vec![
        Arc::new(FixtureTool {
            name: "search".into(),
            calls: calls.clone(),
        }),
        Arc::new(FixtureTool {
            name: "unwanted".into(),
            calls: calls.clone(),
        }),
    ];
    let config = LlmConfig {
        model: "outer-fixture".into(),
        base_url: format!("http://{address}"),
        api_key: "fixture-key".into(),
        max_tokens: 100,
        ..Default::default()
    };
    let result = sar_eval::loop_mode::run(
        &bus,
        tools,
        config,
        "fixture question",
        Duration::from_secs(5),
    )
    .await?;
    assert_eq!(result["answer"], "fixture answer");
    assert_eq!(result["status"], "final_answer");
    assert_eq!(result["searches"], 1);
    assert_eq!(result["prompt_tokens"], 20);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(result["inner_results"].as_array().unwrap().len(), 1);
    runner_task.abort();
    http.abort();
    Ok(())
}
