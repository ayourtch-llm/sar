use anyhow::{bail, Context, Result};
use clap::Parser;
use sar_core::{actor::ActorAnnouncement, bus::SarBus, config::Config, message::Message};
use sar_eval::{grade, markdown, metric, totals, Question, Row};
use sar_tool_actors::{ToolExecuteMessage, ToolResultMessage};
use sar_tool_mcp::McpServerRunner;
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

#[derive(Parser)]
#[command(about = "Headless SAR research/evaluation harness")]
struct Args {
    #[arg(long)]
    config: PathBuf,
    #[arg(long)]
    eval: PathBuf,
    #[arg(long)]
    output: PathBuf,
    #[arg(long, default_value = "direct", value_parser = ["direct", "loop"])]
    mode: String,
    #[arg(long, default_value = "llm_search")]
    server: String,
    #[arg(long, default_value_t = 1200)]
    question_timeout_secs: u64,
    #[arg(long, default_value_t = 24)]
    max_steps: usize,
}
#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "warn".into()),
        )
        .init();
    let args = Args::parse();
    if args.question_timeout_secs == 0 || args.max_steps == 0 {
        bail!("limits must be positive");
    }
    let config = Config::from_file(&args.config)?;
    let server = config
        .mcp_servers
        .get(&args.server)
        .context("missing configured MCP server")?
        .clone();
    let questions: Vec<Question> = serde_json::from_slice(&std::fs::read(&args.eval)?)?;
    let mut ids = std::collections::HashSet::new();
    for q in &questions {
        if q.id.is_empty()
            || !ids.insert(&q.id)
            || q.ground_truth.trim().is_empty()
            || q.source_url.is_empty()
        {
            bail!("invalid/duplicate evaluation question");
        }
    }
    let mut rows = Vec::new();
    for q in questions {
        // A fresh bus and server per question also isolates timed-out calls.
        let bus = SarBus::new();
        bus.register_announcement(ActorAnnouncement {
            id: "sar-eval".into(),
            subscriptions: vec!["tool:results".into()],
            publications: vec!["tool:search:execute".into()],
        })
        .await;
        let mut rx = bus.subscribe("sar-eval", "tool:results").await?;
        let handle = tokio::time::timeout(
            Duration::from_secs(30),
            McpServerRunner::new(args.server.clone(), server.clone()).spawn(&bus),
        )
        .await?
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        if !handle.tool_names().iter().any(|n| n == "search") {
            handle.shutdown().await;
            bail!("MCP server does not expose search");
        }
        // Wait for the real bus subscription, not a fixed startup sleep.
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if bus
                    .list_topic_info()
                    .await
                    .iter()
                    .any(|t| t.name == "tool:search:execute" && !t.subscribers.is_empty())
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .context("MCP tool runner did not subscribe")?;
        let call_id = uuid::Uuid::new_v4().to_string();
        let question = if q.format_hint.is_empty() {
            q.question.clone()
        } else {
            format!(
                "{}\nReturn only the short answer in final_answer.answer. Answer format: {}",
                q.question, q.format_hint
            )
        };
        let result = if args.mode == "loop" {
            eprintln!("question={} started mode=loop", q.id);
            sar_eval::loop_mode::run(
                &bus,
                handle.tool_actors(),
                config.llm.clone(),
                &question,
                Duration::from_secs(args.question_timeout_secs),
            )
            .await?
        } else {
            let request = ToolExecuteMessage {
                tool_call_id: call_id.clone(),
                tool_name: "search".into(),
                arguments: json!({"question":question,"max_steps":args.max_steps}),
            };
            let started = Instant::now();
            eprintln!("question={} started mode={}", q.id, args.mode);
            bus.publish(
                "sar-eval",
                Message::new(
                    "tool:search:execute",
                    "sar-eval",
                    serde_json::to_value(request)?,
                ),
            )
            .await?;
            let outcome =
                tokio::time::timeout(Duration::from_secs(args.question_timeout_secs), async {
                    loop {
                        let msg = rx.recv().await?;
                        let response: ToolResultMessage = serde_json::from_value(msg.payload)?;
                        if response.tool_call_id == call_id {
                            return Ok::<_, anyhow::Error>(response);
                        }
                    }
                })
                .await;
            let result: Value = match outcome {
            Ok(Ok(r)) if r.success => serde_json::from_str(&r.result).unwrap_or_else(|_|json!({"status":"invalid_result","answer":"","error":"search result was not JSON"})),
            Ok(Ok(r)) => json!({"status":"mcp_error","answer":"","error":r.error}),
            Ok(Err(e)) => json!({"status":"bus_error","answer":"","error":e.to_string()}),
            Err(_) => json!({"status":"harness_timeout","answer":"","wall_ms":started.elapsed().as_millis()}),
        };
            result
        };
        handle.shutdown().await;
        let answer_matches = grade(&q, result["answer"].as_str().unwrap_or(""));
        let search_requirement_met = !q.search_required || metric(&result, "searches") > 0;
        let correct =
            answer_matches && search_requirement_met && result["status"] == "final_answer";
        eprintln!(
            "question={} correct={} status={} live_tavily_calls_total={}",
            q.id,
            correct,
            result["status"],
            metric(&result, "live_tavily_calls_total")
        );
        rows.push(Row {
            id: q.id,
            question: q.question,
            ground_truth: q.ground_truth,
            correct,
            answer_matches,
            search_requirement_met,
            result,
        });
        write_results(&args, &rows)?;
    }
    println!("{}", markdown(&rows));
    Ok(())
}
fn write_results(args: &Args, rows: &[Row]) -> Result<()> {
    if let Some(parent) = args.output.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    let output = json!({"mode":args.mode,"config_path":args.config,"eval_path":args.eval,"max_steps":args.max_steps,"question_timeout_secs":args.question_timeout_secs,"rows":rows,"totals":totals(rows)});
    let tmp = args.output.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(&output)?)?;
    std::fs::rename(tmp, &args.output)?;
    std::fs::write(args.output.with_extension("md"), markdown(rows))?;
    Ok(())
}
