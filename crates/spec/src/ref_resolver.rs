use std::{fs, path::PathBuf};

use kernel::prelude::*;
use serde_yaml::Value;
use thiserror::Error;

#[derive(Debug, Error)]
#[error("Invalid JSON Pointer")]
pub struct DocAddressParseError;

#[derive(Debug, Error)]
pub enum RefError {
    #[error("Failed to read file: {0}")]
    Io(#[from] std::io::Error),
    #[error("Failed to parse YAML: {0}")]
    Parse(#[from] serde_yaml::Error),
    #[error("HTTP request failed: {0}")]
    Http(String),
}

pub type RefResult = Result<Shared<Value>, RefError>;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum DocLocation {
    File(PathBuf),
    Url(String),
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct DocAddress(Vec<String>);

pub struct DocAddressIter<'a> {
    inner: std::slice::Iter<'a, String>,
}

impl<'a> Iterator for DocAddressIter<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next().map(|s| s.as_str())
    }
}

impl DocAddress {
    pub fn iter(&self) -> DocAddressIter<'_> {
        DocAddressIter {
            inner: self.0.iter(),
        }
    }

    fn parse(value: &str) -> Result<Self, DocAddressParseError> {
        if !value.starts_with("#/") {
            return Err(DocAddressParseError);
        }

        let parts: Vec<String> = value
            .split('/')
            .skip(1)
            .map(|part| {
                if part.is_empty() {
                    return Err(DocAddressParseError);
                }
                if part.contains('~') && !part.contains("~0") && !part.contains("~1") {
                    return Err(DocAddressParseError);
                }
                Ok(part.replace("~1", "/").replace("~0", "~"))
            })
            .collect::<Result<_, _>>()?;

        Ok(DocAddress(parts))
    }
}

impl TryFrom<&str> for DocAddress {
    type Error = DocAddressParseError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct DocumentRef {
    pub location: Shared<DocLocation>,
    pub addr: Shared<DocAddress>,
}

#[derive(Default)]
pub struct RefResolver {
    docs: std::cell::RefCell<BTreeMap<DocLocation, Shared<Value>>>,
    subtrees: std::cell::RefCell<BTreeMap<(DocLocation, DocAddress), Shared<Value>>>,
}

impl RefResolver {
    pub fn resolve(&self, doc_ref: DocumentRef) -> RefResult {
        let cache_key = ((*doc_ref.location).clone(), (*doc_ref.addr).clone());

        if self.subtrees.borrow().contains_key(&cache_key) {
            return Ok(self.subtrees.borrow()[&cache_key].clone());
        }

        if self.docs.borrow().contains_key(&*doc_ref.location) {
            let doc = self.docs.borrow()[&*doc_ref.location].clone();
            return self.traverse_and_clone(&doc, &*doc_ref.addr, cache_key);
        }

        let full_doc = self.resolve_doc(&*doc_ref.location)?;
        self.docs
            .borrow_mut()
            .insert((*doc_ref.location).clone(), full_doc.clone());
        self.traverse_and_clone(&full_doc, &*doc_ref.addr, cache_key)
    }

    fn traverse_and_clone(
        &self,
        doc: &Value,
        addr: &DocAddress,
        cache_key: (DocLocation, DocAddress),
    ) -> RefResult {
        let mut current = doc;

        for key in addr.iter() {
            current = match current {
                Value::Mapping(map) => map
                    .get(&Value::String(key.to_string()))
                    .ok_or_else(|| RefError::Http(format!("Key not found: {}", key)))?,
                Value::Sequence(seq) => {
                    let index: usize = key
                        .parse()
                        .map_err(|_| RefError::Http(format!("Invalid index: {}", key)))?;
                    seq.get(index)
                        .ok_or_else(|| RefError::Http(format!("Index out of bounds: {}", key)))?
                }
                _ => {
                    return Err(RefError::Http(format!(
                        "Cannot traverse into: {:?}",
                        current
                    )));
                }
            };
        }

        let subtree = Shared::new(current.clone());
        self.subtrees
            .borrow_mut()
            .insert(cache_key, subtree.clone());
        Ok(subtree)
    }

    fn resolve_doc(&self, loc: &DocLocation) -> RefResult {
        if !self.docs.borrow().contains_key(loc) {
            let doc = match loc {
                DocLocation::File(path) => Self::resolve_doc_fs(path)?,
                DocLocation::Url(url) => Self::resolve_doc_http(url)?,
            };
            self.docs.borrow_mut().insert(loc.clone(), doc);
        }
        Ok(self.docs.borrow()[loc].clone())
    }

    fn resolve_doc_fs(loc: &PathBuf) -> RefResult {
        let content = fs::read_to_string(loc)?;
        let value: Value = serde_yaml::from_str(&content)?;
        Ok(Shared::new(value))
    }

    fn resolve_doc_http(loc: &str) -> RefResult {
        let response = ureq::get(loc)
            .call()
            .map_err(|e| RefError::Http(e.to_string()))?;
        let content = response
            .into_string()
            .map_err(|e| RefError::Http(e.to_string()))?;
        let value: Value = serde_yaml::from_str(&content)?;
        Ok(Shared::new(value))
    }
}
