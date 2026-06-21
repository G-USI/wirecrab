use super::common::*;
use super::message::Message;
use crate::utils::structs::*;

#[derive(Debug, Clone)]
pub struct AddressParameter {
    pub description: Option<String>,
    pub location: String,
}

#[derive(Debug, Clone)]
pub struct Channel {
    pub address: Option<String>,
    pub title: Option<String>,
    pub summary: Option<String>,
    pub description: Option<String>,
    pub messages: Vec<Item<Message>>,
    pub parameters: Vec<Item<AddressParameter>>,
    pub tags: Vec<Tag>,
    pub external_docs: Option<ExternalDocs>,
}
