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
    pub components: Option<Components>,
}
