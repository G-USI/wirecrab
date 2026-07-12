//! JSON Schema → Rust struct codegen.
//!
//! Converts the `source` string of a `Schema` into a Rust struct definition
//! suitable for use as a typed payload in the kernel.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use serde_json::Value;

/// Generate a struct definition from a message name and its JSON Schema source.
///
/// For schemas that are not `type: "object"`, falls back to `serde_json::Value`.
pub fn generate_struct(message_name: &str, schema_source: &str) -> TokenStream {
    let name_ident = format_ident!("{}", message_name);

    let schema: Value = match serde_json::from_str(schema_source) {
        Ok(v) => v,
        Err(_) => {
            return quote! {
                pub type #name_ident = serde_json::Value;
            };
        }
    };

    // Only generate a struct for object schemas; everything else becomes Value.
    if schema.get("type").and_then(|v| v.as_str()) != Some("object") {
        return quote! {
            pub type #name_ident = serde_json::Value;
        };
    }

    let properties = schema.get("properties");
    let required: Vec<&str> = schema
        .get("required")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();

    let fields = match properties {
        Some(Value::Object(props)) => {
            let mut fields = Vec::new();
            for (field_name, field_schema) in props {
                let field_ident = sanitize_ident(field_name);
                let field_type = json_type_to_rust(field_schema);
                let is_required = required.contains(&field_name.as_str());

                if is_required {
                    fields.push(quote! {
                        pub #field_ident: #field_type
                    });
                } else {
                    fields.push(quote! {
                        pub #field_ident: Option<#field_type>
                    });
                }
            }
            fields
        }
        _ => Vec::new(),
    };

    if fields.is_empty() {
        quote! {
            #[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
            pub struct #name_ident {}
        }
    } else {
        quote! {
            #[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
            pub struct #name_ident {
                #(#fields,)*
            }
        }
    }
}

/// Map a JSON Schema type descriptor to a Rust type.
fn json_type_to_rust(schema: &Value) -> TokenStream {
    let ty = schema.get("type").and_then(|v| v.as_str());

    match ty {
        Some("string") => quote! { String },
        Some("integer") => quote! { i64 },
        Some("number") => quote! { f64 },
        Some("boolean") => quote! { bool },
        Some("array") => {
            let item_type = schema
                .get("items")
                .map(json_type_to_rust)
                .unwrap_or_else(|| quote! { serde_json::Value });
            quote! { Vec<#item_type> }
        }
        Some("object") => {
            // Nested object — inline as serde_json::Value for now.
            // Typegen for nested structs is a future enhancement.
            quote! { serde_json::Value }
        }
        _ => quote! { serde_json::Value },
    }
}

/// Convert a JSON property name to a valid Rust identifier.
fn sanitize_ident(name: &str) -> proc_macro2::Ident {
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            '-' | '.' => '_',
            c => c,
        })
        .collect();
    format_ident!("{}", cleaned)
}
