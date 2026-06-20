use crate::utils::structs::*;

pub mod channel;
pub mod common;
pub mod message;
pub mod operation;

pub use channel::*;
pub use common::*;
pub use message::*;
pub use operation::*;

#[derive(Debug, Clone)]
pub struct Document {
    pub operations: BTreeMap<String, operation::Operation>,
}
