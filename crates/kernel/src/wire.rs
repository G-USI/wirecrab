use crate::utils::structs::*;

#[async_trait]
pub trait Sender: Lifecycle {
    async fn send_batch(
        &mut self,
        messages: &[WireMessage],
        address_override: Option<&str>,
    ) -> Result<(), AnyhowError>;
}

#[async_trait]
pub trait Receiver: Lifecycle {
    async fn receive(&mut self) -> Result<IncomingMessage, AnyhowError>;
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
    pub reply_to: Option<String>,
}

/// Delivery semantics for a channel.
/// WorkQueue: competing consumers (shared queue, first-to-poll wins).
/// PubSub: fan-out (each consumer gets its own copy).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DeliveryMode {
    #[default]
    WorkQueue,
    PubSub,
}

/// Configuration for creating Sender/Receiver endpoints
#[derive(Debug, Clone, Default)]
pub struct ChannelConfig {
    pub address: String,
    pub delivery: DeliveryMode,
}

impl ChannelConfig {
    /// Create a new ChannelConfig with the given address and delivery mode.
    pub fn new(address: String, delivery: DeliveryMode) -> Self {
        Self { address, delivery }
    }
}

/// Acknowledgment state of an incoming message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AckState {
    #[default]
    Pending,
    Acked,
    Nacked,
    Rejected,
}

/// A message received from a wire, with acknowledgment state.
/// ack()/nack()/reject() are synchronous flag setters (no re-delivery, no side effects).
/// This matches the Python asyncapi-python in-memory broker semantics.
#[derive(Debug, Clone)]
pub struct IncomingMessage {
    /// The original wire message fields
    pub headers: BTreeMap<String, String>,
    pub payload: Vec<u8>,
    pub correlation_id: Option<String>,
    pub content_type: Option<String>,
    pub reply_to: Option<String>,
    /// Current acknowledgment state
    ack_state: AckState,
}

impl IncomingMessage {
    /// Create a new IncomingMessage from a WireMessage (ack_state = Pending)
    pub fn from_wire(msg: WireMessage) -> Self {
        Self {
            headers: msg.headers,
            payload: msg.payload,
            correlation_id: msg.correlation_id,
            content_type: msg.content_type,
            reply_to: msg.reply_to,
            ack_state: AckState::Pending,
        }
    }

    /// Acknowledge the message (marks as successfully processed).
    pub fn ack(&mut self) {
        self.ack_state = AckState::Acked;
    }

    /// Negatively acknowledge the message (marks as failed processing).
    pub fn nack(&mut self) {
        self.ack_state = AckState::Nacked;
    }

    /// Reject the message (marks as unprocessable).
    pub fn reject(&mut self) {
        self.ack_state = AckState::Rejected;
    }

    /// Returns true if the message has been acknowledged.
    pub fn is_acknowledged(&self) -> bool {
        self.ack_state == AckState::Acked
    }

    /// Returns true if the message has been nacked.
    pub fn is_nacked(&self) -> bool {
        self.ack_state == AckState::Nacked
    }

    /// Returns true if the message has been rejected.
    pub fn is_rejected(&self) -> bool {
        self.ack_state == AckState::Rejected
    }

    /// Returns true if the message has not yet been acknowledged, nacked, or rejected.
    pub fn is_pending(&self) -> bool {
        self.ack_state == AckState::Pending
    }

    /// Returns the current acknowledgment state.
    pub fn ack_state(&self) -> AckState {
        self.ack_state
    }
}
