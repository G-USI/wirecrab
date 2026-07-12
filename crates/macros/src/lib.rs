#![forbid(unsafe_code)]

//! `#[asyncapi("spec.yaml")]` proc macro.
//!
//! Reads an AsyncAPI 3 spec at compile time, generates typed message structs
//! and typed publisher/subscriber factory methods.

mod codegen;
mod typegen;

use proc_macro::TokenStream;
use proc_macro2::Span;
use quote::quote;
use syn::{parse_macro_input, ItemStruct, LitStr};

use wirecrab_spec::Document;

#[proc_macro_attribute]
pub fn asyncapi(attr: TokenStream, item: TokenStream) -> TokenStream {
    // Parse the spec path from the attribute: #[asyncapi("path/to/spec.yaml")]
    let spec_path = parse_macro_input!(attr as LitStr).value();

    // Parse the struct the attribute is applied to
    let input = parse_macro_input!(item as ItemStruct);
    let struct_name = input.ident.clone();

    // Read and parse the spec file
    let manifest_dir =
        std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| String::from("."));

    let full_path = if std::path::Path::new(&spec_path).is_absolute() {
        spec_path.clone()
    } else {
        format!("{manifest_dir}/{spec_path}")
    };

    let document = match wirecrab_spec::parse(full_path.clone()) {
        Ok(doc) => doc,
        Err(e) => {
            let msg = format!("Failed to parse AsyncAPI spec '{full_path}': {e}");
            return syn::Error::new(Span::call_site(), msg)
                .to_compile_error()
                .into();
        }
    };

    // Collect all message schemas (deduplicated)
    let schemas = collect_message_schemas(&document);

    // Generate typed structs via typify
    let struct_defs = match typegen::generate_types(&schemas) {
        Ok(tokens) => tokens,
        Err(e) => return e.to_compile_error().into(),
    };

    // Generate application impl
    let impl_block = codegen::generate_impl(&struct_name, &document);

    quote! {
        #input
        #struct_defs
        #impl_block
    }
    .into()
}

/// Collect deduplicated `(name, schema_source)` pairs for all messages.
fn collect_message_schemas(doc: &Document) -> Vec<(String, String)> {
    let mut schemas: Vec<(String, String)> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    // Collect messages from operations
    for item in &doc.operations {
        for msg in &item.item.messages {
            if let Some(name) = &msg.name {
                if seen.insert(name.clone()) {
                    if let Some(payload) = &msg.payload {
                        schemas.push((name.clone(), payload.source.clone()));
                    }
                }
            }
        }
        // Also check channel messages — use key as fallback for name
        for ch_msg in &item.item.channel.messages {
            let name = ch_msg.item.name.as_deref().unwrap_or(&ch_msg.key);
            if seen.insert(name.to_string()) {
                if let Some(payload) = &ch_msg.item.payload {
                    schemas.push((name.to_string(), payload.source.clone()));
                }
            }
        }
    }

    // Collect messages from components
    if let Some(components) = &doc.components {
        for item in &components.messages {
            let name = item.item.name.as_deref().unwrap_or(&item.key);
            if seen.insert(name.to_string()) {
                if let Some(payload) = &item.item.payload {
                    schemas.push((name.to_string(), payload.source.clone()));
                }
            }
        }
    }

    schemas
}
