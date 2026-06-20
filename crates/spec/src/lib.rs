//
// SPDX-License-Identifier: Apache-2.0

//! AsyncAPI spec parser and validator.
//!
//! This crate provides parsing and validation of AsyncAPI 3.0/3.1 specifications.
//! It operates on [`kernel::document`] types from wirecrab-kernel crate.

pub mod ast;
pub mod extract;
pub mod resolver;
pub mod validation;
pub use kernel::document::*;
use serde_json::Value;

use kernel::prelude::*;

use crate::extract::extract_document;
use crate::resolver::{RefError, RefResolver};
use crate::validation::validate;

#[derive(Debug, ThisError)]
pub enum SpecError {
    #[error("Failed to resolve and populate yaml spec: {0}")]
    RefResolvingFailure(#[from] RefError),
    #[error("Invalid spec: {0}")]
    InvalidSpec(String),
    #[error("Invalid AsyncAPI version: {0}")]
    UnsupportedVersion(String),
    #[error("Invalid schema: {0}")]
    InvalidSchema(String),
    #[error("Document validation failed: {0}")]
    ValidationFailed(String),
    #[error("Extraction failed: {0}")]
    ExtractionFailed(String),
}

pub type SpecParseResult = Result<Document, SpecError>;

pub fn parse(path: String) -> SpecParseResult {
    let resolver = RefResolver::default();

    let root_value: Value = (*resolver.resolve_ref(&path, "#/")?.clone()).clone();

    validate(&root_value)?;

    let resolved_document: Value = resolver.resolve_recursive(&root_value, &path)?;

    let document = extract_document(&resolved_document)?;

    Ok(document)
}
