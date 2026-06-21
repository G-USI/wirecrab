use super::common::*;
use crate::utils::structs::*;

#[derive(Debug, Clone)]
pub struct Message {
    pub name: Option<String>,
    pub title: Option<String>,
    pub summary: Option<String>,
    pub description: Option<String>,
    pub content_type: Option<String>,
    pub headers: Option<Schema>,
    pub payload: Option<Schema>,
    pub deprecated: bool,
    pub correlation_id: Option<CorrelationId>,
    pub tags: Vec<Tag>,
    pub external_docs: Option<ExternalDocs>,
}
