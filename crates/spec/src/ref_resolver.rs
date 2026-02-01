use std::{fs, path::PathBuf};

use kernel::prelude::*;
use serde_yaml::Value;
use thiserror::Error;

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
pub struct DocumentRef {
    pub path: DocLocation,
    pub addr: Shared<String>,
}

#[derive(Default)]
pub struct RefResolver {
    docs: std::cell::RefCell<BTreeMap<DocLocation, Shared<Value>>>,
}

impl RefResolver {
    pub fn resolve(&mut self, DocumentRef { path, addr: _ }: DocumentRef) -> RefResult {
        if !self.docs.borrow().contains_key(&path) {
            let doc = Self::resolve_doc(&path)?;
            self.docs.borrow_mut().insert(path.clone(), doc);
        }
        Ok(self.docs.borrow()[&path].clone())
    }

    fn resolve_doc(loc: &DocLocation) -> RefResult {
        match loc {
            DocLocation::File(path) => Self::resolve_doc_fs(path),
            DocLocation::Url(url) => Self::resolve_doc_http(url),
        }
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
