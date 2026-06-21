use crate::utils::structs::*;

/// A keyed wrapper carrying an item alongside its original map key.
///
/// Replaces the old `BTreeMap<String, T>` + `T { key: String }` pattern:
/// the key now lives on the `Item`, not duplicated inside the item.
#[derive(Debug, Clone)]
pub struct Item<T> {
    pub key: String,
    pub item: T,
}

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
