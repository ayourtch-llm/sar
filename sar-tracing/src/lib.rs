use std::sync::Arc;

use sar_core::bus::SarBus;
use sar_core::message::Message;
use tracing::field::Visit;
use tracing::Event;
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::registry::LookupSpan;

pub struct BusLayer {
    bus: Arc<SarBus>,
    topic: String,
}

impl BusLayer {
    pub fn new(bus: Arc<SarBus>, topic: String) -> Self {
        Self { bus, topic }
    }
}

impl<S> Layer<S> for BusLayer
where
    for<'a> S: LookupSpan<'a> + tracing::Subscriber,
{
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let mut visitor = BusVisitor { msg: String::new() };
        event.record(&mut visitor);
        
        let meta = event.metadata();
        let level = meta.level().as_str().to_uppercase();
        let target = meta.target();
        let span = meta.name();
        
        let text = format!("[{}] {}::{} {}", level, target, span, visitor.msg);
        
        let msg = Message::new(
            &self.topic,
            "tracing",
            text,
        );
        
        let bus = self.bus.clone();
        tokio::spawn(async move {
            if let Err(e) = bus.publish("sar-tracing", msg).await {
                eprintln!("Failed to publish tracing message: {}", e);
            }
        });
    }
}

struct BusVisitor {
    msg: String,
}

impl Visit for BusVisitor {
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "message" {
            self.msg = value.to_string();
        } else {
            if !self.msg.is_empty() {
                self.msg.push(' ');
            }
            self.msg.push_str(&format!("{}={}", field.name(), value));
        }
    }

    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.msg = format!("{:?}", value);
        } else {
            if !self.msg.is_empty() {
                self.msg.push(' ');
            }
            self.msg.push_str(&format!("{}={:?}", field.name(), value));
        }
    }
}