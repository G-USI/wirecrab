use crate::utils::structs::*;

pub mod channel;
pub mod common;
pub mod components;
pub mod message;
pub mod operation;

pub use channel::*;
pub use common::*;
pub use components::*;
pub use message::*;
pub use operation::*;

#[derive(Debug, Clone)]
pub struct Document {
    pub operations: Vec<Item<Operation>>,
    /// Deduplicated messages reachable from operations. How keys are
    /// assigned (title, name, channel key, etc.) is the spec crate's
    /// policy, not the kernel's.
    pub messages: Vec<Item<Message>>,
    /// Named schemas from `components/schemas/*`, populated by the spec
    /// crate's extract pass.
    pub schemas: Vec<Item<Schema>>,
    /// Raw components section preserved as an escape hatch.
    pub components: Option<Components>,
}
