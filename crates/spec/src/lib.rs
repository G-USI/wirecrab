//
// SPDX-License-Identifier: Apache-2.0

//! AsyncAPI spec parser and validator.
//!
//! This crate provides parsing and validation of AsyncAPI 3.0/3.1 specifications.
//! It operates on [`kernel::document`] types from wirecrab-kernel crate.

pub mod ast;
pub mod resolver;
pub use kernel::document::*;
use serde_json::Value;

use kernel::prelude::*;

use crate::resolver::{RefError, RefResolver};

#[derive(Debug, ThisError)]
pub enum SpecError {
    #[error("Failed to resolve and populate yaml spec: {0}")]
    RefResolvingFailure(#[from] RefError),
    #[error("Failed to deserialize spec into Document: {0}")]
    DeserializeError(#[from] serde_json::Error),
}

pub type SpecParseResult = Result<Value, SpecError>;

pub fn parse(path: String) -> SpecParseResult {
    let resolver = RefResolver::default();

    // -- Parse root yaml file
    let root_value: Value = (*resolver.resolve_ref(&path, "#/")?.clone()).clone();

    // -- Recursively resolve all $refs in document
    let resolved_document: Value = resolver.resolve_recursive(&root_value, &path)?;

    // -- Validate resolved document against AsyncAPI spec schema
    // TODO: Add jsonschema validation

    // -- Return resolved Value
    Ok(resolved_document)
}
