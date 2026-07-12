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

/// Collect `(name, payload_source)` pairs from the deduplicated
/// `Document::messages` IR field. The spec crate already handles dedup
/// and collision detection — this is a pure projection.
fn collect_message_schemas(doc: &Document) -> Vec<(String, String)> {
    doc.messages
        .iter()
        .filter_map(|item| {
            // Skip synthesized anonymous-message keys (format `<op>#<n>`).
            // typify needs a real name to bind the struct to.
            if item.key.contains('#') {
                return None;
            }
            let payload = item.item.payload.as_ref()?;
            Some((item.key.clone(), payload.source.clone()))
        })
        .collect()
}
