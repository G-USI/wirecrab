use crate::utils::structs::*;

#[derive(Debug, Clone)]
pub struct Schema {
    pub format: String,
    pub source: String,
}

#[derive(Debug, Clone)]
pub struct Tag {
    pub name: String,
    pub description: Option<String>,
    pub external_docs: Option<ExternalDocs>,
}

#[derive(Debug, Clone)]
pub struct ExternalDocs {
    pub description: Option<String>,
    pub url: String,
}

#[derive(Debug, Clone)]
pub enum Action {
    Send,
    Receive,
}

#[derive(Debug, Clone)]
pub struct CorrelationId {
    pub description: Option<String>,
    pub location: String,
}
