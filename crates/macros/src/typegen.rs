//! JSON Schema → Rust struct codegen via typify.
//!
//! Collects all message schemas from the document and feeds them into a single
//! `typify::TypeSpace` so cross-references between types resolve correctly.

use proc_macro2::TokenStream;
use schemars::schema::Schema;
use typify_impl::{TypeSpace, TypeSpaceSettings};

/// Generate all Rust type definitions for a set of message schemas.
///
/// Each entry is `(name, schema_source)` where `schema_source` is the raw
/// JSON Schema string from the AsyncAPI spec. Returns the combined
/// `TokenStream` of all generated struct/enum definitions.
pub fn generate_types(
    schemas: &[(String, String)],
) -> Result<TokenStream, syn::Error> {
    let mut settings = TypeSpaceSettings::default();
    settings.with_derive("Debug".to_string());
    settings.with_derive("Clone".to_string());
    settings.with_derive("PartialEq".to_string());

    let mut type_space = TypeSpace::new(&settings);

    for (name, source) in schemas {
        let schema_json: serde_json::Value = serde_json::from_str(source)
            .map_err(|e| {
                syn::Error::new(
                    proc_macro2::Span::call_site(),
                    format!("Failed to parse schema for '{name}': {e}"),
                )
            })?;

        // Wrap in a named schema so typify uses our name
        let mut named = schema_json;
        if let Some(obj) = named.as_object_mut() {
            obj.insert("title".to_string(), serde_json::Value::String(name.clone()));
        }

        let schema: Schema = serde_json::from_value(named)
            .map_err(|e| {
                syn::Error::new(
                    proc_macro2::Span::call_site(),
                    format!("Failed to convert schema for '{name}': {e}"),
                )
            })?;

        type_space
            .add_type(&schema)
            .map_err(|e| {
                syn::Error::new(
                    proc_macro2::Span::call_site(),
                    format!("typify failed on '{name}': {e}"),
                )
            })?;
    }

    Ok(type_space.to_stream())
}
