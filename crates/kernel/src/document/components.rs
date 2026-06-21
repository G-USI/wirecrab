use super::common::{Item, Schema};
use super::message::Message;
use crate::utils::structs::*;

#[derive(Debug, Clone)]
pub struct Components {
    pub messages: Vec<Item<Message>>,
    pub schemas: Vec<Item<Schema>>,
}
