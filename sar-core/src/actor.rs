use tokio::task::JoinHandle;

use crate::bus::SarBus;

pub type ActorId = String;

#[async_trait::async_trait]
pub trait Actor: Send + Sized {
    fn id(&self) -> ActorId;

    async fn run(&self, bus: &SarBus) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
}

pub struct ActorJoinHandle {
    id: ActorId,
    handle: JoinHandle<()>,
}

impl ActorJoinHandle {
    pub fn new(id: ActorId, handle: JoinHandle<()>) -> Self {
        Self { id, handle }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub async fn stop(self) {
        self.handle.abort();
    }

    pub async fn wait(self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.handle
            .await
            .map_err(|e| Box::new(std::io::Error::new(std::io::ErrorKind::Other, format!("Task panicked: {}", e))) as Box<dyn std::error::Error + Send + Sync>)?;
        Ok(())
    }
}