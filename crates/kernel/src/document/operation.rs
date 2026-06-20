use super::channel::Channel;
use super::common::*;
use super::message::Message;
use crate::utils::structs::*;

#[derive(Debug, Clone)]
pub struct OperationReply {
    pub address: Option<String>,
    pub channel: Option<Channel>,
    pub messages: Vec<Message>,
}

#[derive(Debug, Clone)]
pub struct Operation {
    pub action: Action,
    pub title: Option<String>,
    pub summary: Option<String>,
    pub description: Option<String>,
    pub channel: Channel,
    pub messages: Vec<Message>,
    pub reply: Option<OperationReply>,
    pub tags: Vec<Tag>,
    pub external_docs: Option<ExternalDocs>,
    pub key: String,
}
