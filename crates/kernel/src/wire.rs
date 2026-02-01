use crate::utils::structs::*;

#[async_trait]
pub trait Sender: Lifecycle {
    async fn send_batch(&mut self, messages: &[WireMessage]) -> Result<(), AnyhowError>;
}

#[async_trait]
pub trait Receiver: Lifecycle {
    async fn receive(&mut self) -> Result<WireMessage, AnyhowError>;
}

#[async_trait]
pub trait Wire: Lifecycle {
    async fn new_sender(&mut self, config: &ChannelConfig) -> Result<Box<dyn Sender>, AnyhowError>;
    async fn new_receiver(
        &mut self,
        config: &ChannelConfig,
    ) -> Result<Box<dyn Receiver>, AnyhowError>;
}

#[async_trait]
pub trait Lifecycle: ThreadSafe {
    async fn start(&mut self) -> Result<(), AnyhowError>;
    async fn stop(&mut self) -> Result<(), AnyhowError>;
}

#[derive(Debug, Clone)]
pub struct WireMessage {
    pub headers: BTreeMap<String, String>,
    pub payload: Vec<u8>,
    pub correlation_id: Option<String>,
    pub content_type: Option<String>,
}

/// Configuration for creating Sender/Receiver endpoints
pub struct ChannelConfig;
