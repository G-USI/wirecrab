use std::{collections::HashMap, fs, path::PathBuf};

use kernel::prelude::*;
use serde_json::Value;
use serde_yaml;
use thiserror::Error;
use url::Url;

#[derive(Debug, Error)]
#[error("Invalid JSON Pointer")]
pub struct DocAddressParseError;

impl From<DocAddressParseError> for RefError {
    fn from(_: DocAddressParseError) -> Self {
        RefError::Http("Invalid JSON Pointer".to_string())
    }
}

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

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum DocLocation {
    File(PathBuf),
    Url(Url),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
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
            .enumerate()
            .map(|(index, part)| {
                if part.is_empty() {
                    if index == 0 {
                        return Ok(None);
                    }
                    return Err(DocAddressParseError);
                }
                if part.contains('~') && !part.contains("~0") && !part.contains("~1") {
                    return Err(DocAddressParseError);
                }
                Ok(Some(part.replace("~1", "/").replace("~0", "~")))
            })
            .collect::<Result<Vec<Option<String>>, _>>()?
            .into_iter()
            .flatten()
            .collect();

        Ok(DocAddress(parts))
    }
}

impl TryFrom<&str> for DocAddress {
    type Error = DocAddressParseError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DocumentRef {
    pub location: Shared<DocLocation>,
    pub addr: Shared<DocAddress>,
}

#[derive(Default)]
pub struct RefResolver {
    docs: std::cell::RefCell<HashMap<DocLocation, Shared<Value>>>,
    subtrees: std::cell::RefCell<HashMap<(DocLocation, DocAddress), Shared<Value>>>,
}

impl RefResolver {
    pub fn resolve(&self, doc_ref: DocumentRef) -> RefResult {
        let cache_key = ((*doc_ref.location).clone(), (*doc_ref.addr).clone());

        if self.subtrees.borrow().contains_key(&cache_key) {
            return Ok(self.subtrees.borrow()[&cache_key].clone());
        }

        if self.docs.borrow().contains_key(&*doc_ref.location) {
            let doc = self.docs.borrow()[&*doc_ref.location].clone();
            return self.traverse_and_clone(&doc, &doc_ref.addr, cache_key);
        }

        let full_doc = self.resolve_doc(&doc_ref.location)?;
        self.docs
            .borrow_mut()
            .insert((*doc_ref.location).clone(), full_doc.clone());
        self.traverse_and_clone(&full_doc, &doc_ref.addr, cache_key)
    }

    pub fn resolve_ref(&self, current_file: &str, ref_str: &str) -> RefResult {
        let (location_str, address_str) = Self::parse_ref(ref_str, current_file)?;
        let location = Self::resolve_location(location_str);

        let address_str_for_parse = if address_str.starts_with('#') {
            address_str.to_string()
        } else {
            format!("#{}", address_str)
        };

        let address = DocAddress::try_from(address_str_for_parse.as_str())
            .map_err(|e| RefError::Http(e.to_string()))?;

        let doc_ref = DocumentRef {
            location: Shared::new(location),
            addr: Shared::new(address),
        };
        self.resolve(doc_ref)
    }

    pub fn resolve_recursive(&self, value: &Value, current_file: &str) -> Result<Value, RefError> {
        self.resolve_recursive_with_stack(
            value,
            current_file,
            &mut std::collections::HashSet::new(),
        )
    }

    fn resolve_recursive_with_stack(
        &self,
        value: &Value,
        current_file: &str,
        visited: &mut std::collections::HashSet<String>,
    ) -> Result<Value, RefError> {
        match value {
            Value::Object(map) => {
                if let Some(ref_str) = map.get("$ref").and_then(|v| v.as_str()) {
                    let ref_key = format!("{}#{}", current_file, ref_str);

                    if visited.contains(&ref_key) {
                        return Err(RefError::Http(format!(
                            "Circular reference detected: {}",
                            ref_str
                        )));
                    }

                    visited.insert(ref_key.clone());
                    let resolved = (*self.resolve_ref(current_file, ref_str)?.clone()).clone();

                    self.resolve_recursive_with_stack(&resolved, current_file, visited)
                } else {
                    let mut new_map = serde_json::Map::new();
                    for (key, val) in map {
                        if key != "$ref" {
                            let resolved =
                                self.resolve_recursive_with_stack(val, current_file, visited)?;
                            new_map.insert(key.clone(), resolved);
                        }
                    }
                    Ok(Value::Object(new_map))
                }
            }
            Value::Array(arr) => {
                let mut new_arr = Vec::new();
                for item in arr {
                    new_arr.push(self.resolve_recursive_with_stack(item, current_file, visited)?);
                }
                Ok(Value::Array(new_arr))
            }
            _ => Ok(value.clone()),
        }
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
                Value::Object(map) => map
                    .get(key)
                    .ok_or_else(|| RefError::Http(format!("Key not found: {}", key)))?,
                Value::Array(seq) => {
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

    fn resolve_doc_http(loc: &Url) -> RefResult {
        let response = ureq::get(loc.as_str())
            .call()
            .map_err(|e| RefError::Http(e.to_string()))?;
        let content = response
            .into_string()
            .map_err(|e| RefError::Http(e.to_string()))?;
        let value: Value = serde_yaml::from_str(&content)?;
        Ok(Shared::new(value))
    }

    fn parse_ref<'a>(
        ref_str: &'a str,
        current_file: &'a str,
    ) -> Result<(&'a str, &'a str), RefError> {
        if let Some((loc, addr)) = ref_str.split_once('#') {
            let location = if loc.is_empty() { current_file } else { loc };
            Ok((location, addr))
        } else {
            Err(RefError::Http("Invalid ref format".to_string()))
        }
    }

    fn resolve_location(location_str: &str) -> DocLocation {
        Url::parse(location_str)
            .map(DocLocation::Url)
            .unwrap_or_else(|_| DocLocation::File(PathBuf::from(location_str)))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests;
