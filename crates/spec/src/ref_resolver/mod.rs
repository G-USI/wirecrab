use std::{collections::HashMap, fs, path::PathBuf};

use kernel::prelude::*;
use serde_json::Value;
use serde_yaml;
use thiserror::Error;
use url::Url;

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

/// Provenance of a resolved subtree: the JSON Reference (RFC) string for
/// where this content originated in the source spec.
///
/// Format follows JSON Reference: `"<file>#/<json-pointer>"` for cross-file
/// refs, or `"#/<json-pointer>"` for same-document refs. The path is a
/// JSON Pointer per RFC 6901.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProvenanceEntry {
    /// Fully-qualified origin: `#/components/schemas/User` (same doc) or
    /// `file:///abs/path.yaml#/User` (cross-file).
    pub origin: String,
}

/// Sidecar registry mapping resolved-tree JSON Pointer → origin in source spec.
///
/// Built during `$ref` expansion. Every visited subtree (objects, arrays,
/// primitives) is recorded. Keys are JSON Pointers into the resolved tree
/// (e.g., `"#/operations/0/messages/0/payload/properties/user"`).
///
/// Discarded after extract; never leaks into the typed `Document` IR.
#[derive(Debug, Default, Clone)]
pub struct ProvenanceRegistry {
    entries: HashMap<String, ProvenanceEntry>,
}

impl ProvenanceRegistry {
    pub fn record(&mut self, resolved_path: String, entry: ProvenanceEntry) {
        self.entries.insert(resolved_path, entry);
    }

    pub fn lookup(&self, resolved_path: &str) -> Option<&ProvenanceEntry> {
        self.entries.get(resolved_path)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&String, &ProvenanceEntry)> {
        self.entries.iter()
    }
}

/// Result of `$ref` expansion: clean resolved JSON + provenance sidecar.
#[derive(Debug, Clone)]
pub struct ResolvedDocument {
    pub value: Value,
    pub provenance: ProvenanceRegistry,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum DocLocation {
    File(PathBuf),
    Url(Url),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DocAddress(Vec<String>);

impl DocAddress {
    pub fn iter(&self) -> impl Iterator<Item = &str> {
        self.0.iter().map(|s| s.as_str())
    }

    fn parse(value: &str) -> Result<Self, RefError> {
        if !value.starts_with("#/") {
            return Err(RefError::Http("Invalid JSON Pointer".to_string()));
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
                    return Err(RefError::Http("Invalid JSON Pointer".to_string()));
                }
                if part.contains('~') && !part.contains("~0") && !part.contains("~1") {
                    return Err(RefError::Http("Invalid JSON Pointer".to_string()));
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
    type Error = RefError;

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
        let location = Self::resolve_location(&location_str);

        let address = if address_str.starts_with('#') {
            DocAddress::try_from(address_str.as_str())
        } else {
            DocAddress::try_from(format!("#{}", address_str).as_str())
        }?;

        let doc_ref = DocumentRef {
            location: Shared::new(location),
            addr: Shared::new(address),
        };
        self.resolve(doc_ref)
    }

    pub fn resolve_recursive(
        &self,
        value: &Value,
        current_file: &str,
    ) -> Result<ResolvedDocument, RefError> {
        let mut provenance = ProvenanceRegistry::default();
        let value = self.resolve_recursive_with_stack(
            value,
            current_file,
            "#",
            "#",
            &mut std::collections::HashSet::new(),
            &mut provenance,
        )?;
        Ok(ResolvedDocument { value, provenance })
    }

    #[allow(clippy::too_many_arguments)]
    fn resolve_recursive_with_stack(
        &self,
        value: &Value,
        current_file: &str,
        output_path: &str,
        origin_path: &str,
        visited: &mut std::collections::HashSet<String>,
        provenance: &mut ProvenanceRegistry,
    ) -> Result<Value, RefError> {
        provenance.record(
            output_path.to_string(),
            ProvenanceEntry {
                origin: origin_path.to_string(),
            },
        );

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

                    let (resolved_loc, resolved_addr) = Self::parse_ref(ref_str, current_file)?;
                    let next_file: &str = if resolved_loc.is_empty() {
                        current_file
                    } else {
                        &resolved_loc
                    };

                    // The inlined content's origin is the ref target.
                    // parse_ref returns the address part as `resolved_addr`,
                    // which may be empty if the ref points to a whole doc;
                    // normalize to "#" for the root.
                    let new_origin_path = if resolved_addr.is_empty() {
                        "#".to_string()
                    } else {
                        resolved_addr
                    };

                    let result = self.resolve_recursive_with_stack(
                        &resolved,
                        next_file,
                        output_path,
                        &new_origin_path,
                        visited,
                        provenance,
                    );
                    visited.remove(&ref_key);
                    result
                } else {
                    let mut new_map = serde_json::Map::new();
                    for (key, val) in map {
                        if key != "$ref" {
                            let child_output = json_pointer_child(output_path, key);
                            let child_origin = json_pointer_child(origin_path, key);
                            let resolved = self.resolve_recursive_with_stack(
                                val,
                                current_file,
                                &child_output,
                                &child_origin,
                                visited,
                                provenance,
                            )?;
                            new_map.insert(key.clone(), resolved);
                        }
                    }
                    Ok(Value::Object(new_map))
                }
            }
            Value::Array(arr) => {
                let mut new_arr = Vec::new();
                for (i, item) in arr.iter().enumerate() {
                    let child_output = json_pointer_index(output_path, i);
                    let child_origin = json_pointer_index(origin_path, i);
                    let resolved = self.resolve_recursive_with_stack(
                        item,
                        current_file,
                        &child_output,
                        &child_origin,
                        visited,
                        provenance,
                    )?;
                    new_arr.push(resolved);
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

    fn parse_ref(ref_str: &str, current_file: &str) -> Result<(String, String), RefError> {
        let (loc, addr) = ref_str
            .split_once('#')
            .ok_or_else(|| RefError::Http("Invalid ref format".to_string()))?;

        let location = if loc.is_empty() {
            current_file.to_string()
        } else if Url::parse(loc).is_ok() && (loc.starts_with("http") || loc.starts_with("https"))
            || std::path::Path::new(loc).is_absolute()
        {
            loc.to_string()
        } else {
            let parent = std::path::Path::new(current_file)
                .parent()
                .unwrap_or(std::path::Path::new("."));
            parent.join(loc).to_string_lossy().into_owned()
        };

        Ok((location, addr.to_string()))
    }

    fn resolve_location(location_str: &str) -> DocLocation {
        Url::parse(location_str)
            .map(DocLocation::Url)
            .unwrap_or_else(|_| DocLocation::File(PathBuf::from(location_str)))
    }
}

/// Build a child JSON Pointer by appending `/key` (RFC 6901 escaped).
fn json_pointer_child(parent: &str, key: &str) -> String {
    let escaped = key.replace('~', "~0").replace('/', "~1");
    format!("{parent}/{escaped}")
}

/// Build a child JSON Pointer by appending `/index`.
fn json_pointer_index(parent: &str, index: usize) -> String {
    format!("{parent}/{index}")
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests;
