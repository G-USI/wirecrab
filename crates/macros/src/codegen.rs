//! Codegen for the Application impl — generates typed methods per operation.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use wirecrab_spec::{Action, Channel, Document, Message, Operation};

/// Generate the full `impl` block for the user's struct.
///
/// Produces:
/// - Schema constants for each unique message (so users don't reconstruct `Schema`)
/// - One method per `send` operation (typed publisher)
/// - One method per `receive` operation (typed subscriber factory)
pub fn generate_impl(type_name: &proc_macro2::Ident, doc: &Document) -> TokenStream {
    let mut methods = Vec::new();
    let mut schema_consts: Vec<TokenStream> = Vec::new();
    let mut seen_schemas: std::collections::HashSet<String> = std::collections::HashSet::new();

    for item in &doc.operations {
        let op = &item.item;
        let method_name = operation_method_name(&item.key, &op.action);
        let message_type = first_message_type(op);

        if let Some((msg, name_fallback)) = first_message(op) {
            if let Some(payload) = &msg.payload {
                let const_name = schema_const_name(msg, name_fallback.as_deref());
                if seen_schemas.insert(const_name.clone()) {
                    schema_consts.push(generate_schema_const(&const_name, payload));
                }
            }
        }

        match op.action {
            Action::Send => {
                methods.push(generate_send_method(&method_name, op));
            }
            Action::Receive => {
                methods.push(generate_receive_method(&method_name, &message_type, op));
            }
        }
    }

    quote! {
        impl #type_name {
            #(#schema_consts)*
            #(#methods)*
        }
    }
}

/// Generate a `pub fn schema_<name>() -> Schema` accessor.
fn generate_schema_const(name: &str, schema: &wirecrab_spec::Schema) -> TokenStream {
    let fn_name = format_ident!("schema_{}", name.to_lowercase());
    let format = &schema.format;
    let source = &schema.source;
    quote! {
        /// Schema accessor for this message, sourced from the AsyncAPI spec.
        pub fn #fn_name() -> wirecrab_kernel::document::Schema {
            wirecrab_kernel::document::Schema {
                format: ::std::string::String::from(#format),
                source: ::std::string::String::from(#source),
            }
        }
    }
}

/// Generate a send operation method.
///
/// Creates a `Publisher` pre-configured with the channel from the spec.
/// The user calls this, then `publisher.send(&payload)`.
fn generate_send_method(
    method_name: &proc_macro2::Ident,
    op: &Operation,
) -> TokenStream {
    let channel = channel_to_tokens(&op.channel);

    quote! {
        /// Create a typed publisher for this send operation.
        /// The caller provides the codec and a sender from their wire.
        pub fn #method_name<C>(
            codec: C,
            sender: Box<dyn wirecrab_kernel::wire::Sender>,
        ) -> wirecrab_kernel::endpoint::publisher::Publisher<C>
        where
            C: wirecrab_kernel::codec::Codec,
        {
            wirecrab_kernel::endpoint::publisher::Publisher::new(
                codec,
                #channel,
                sender,
            )
        }
    }
}

/// Generate a receive operation method.
///
/// Creates a typed `Subscriber` with the payload type from the spec.
fn generate_receive_method(
    method_name: &proc_macro2::Ident,
    message_type: &TokenStream,
    op: &Operation,
) -> TokenStream {
    let _channel = channel_to_tokens(&op.channel);

    quote! {
        /// Create a typed subscriber for this receive operation.
        /// The caller provides the codec, handler, and a receiver from their wire.
        pub fn #method_name<C, H>(
            codec: C,
            handler: H,
            receiver: Box<dyn wirecrab_kernel::wire::Receiver>,
        ) -> wirecrab_kernel::endpoint::subscriber::Subscriber<C, H, #message_type>
        where
            C: wirecrab_kernel::codec::Codec + 'static,
            H: wirecrab_kernel::endpoint::handler::Handler<#message_type> + 'static,
        {
            wirecrab_kernel::endpoint::subscriber::Subscriber::new(
                codec,
                handler,
                receiver,
            )
        }
    }
}

/// Convert an operation key like `/application/ping` to a method name like
/// `publisher_application_ping` or `subscriber_application_ping`.
fn operation_method_name(op_key: &str, action: &Action) -> proc_macro2::Ident {
    let prefix = match action {
        Action::Send => "publisher",
        Action::Receive => "subscriber",
    };

    let clean: String = op_key
        .trim_matches('/')
        .chars()
        .map(|c| match c {
            '/' | '-' | '.' => '_',
            c => c,
        })
        .collect();

    format_ident!("{}_{}", prefix, clean)
}

/// Determine the message type name for the first message in an operation.
/// Falls back to `serde_json::Value` if no messages are defined.
fn first_message_type(op: &Operation) -> TokenStream {
    if let Some((msg, name_fallback)) = first_message(op) {
        let name = msg.name.as_deref().or(name_fallback.as_deref());
        if let Some(name) = name {
            let ident = format_ident!("{}", name);
            return quote! { #ident };
        }
    }
    quote! { serde_json::Value }
}

/// Find the first message for an operation (operation-level first, then channel).
/// Returns the message and an optional name fallback (channel key) when
/// `message.name` is unset.
fn first_message(op: &Operation) -> Option<(&Message, Option<String>)> {
    if let Some(m) = op.messages.first() {
        return Some((m, m.name.clone()));
    }
    op.channel
        .messages
        .first()
        .map(|item| (&item.item, item.item.name.clone().or_else(|| Some(item.key.clone()))))
}

/// Build the schema accessor fn name from a message.
fn schema_const_name(msg: &Message, fallback: Option<&str>) -> String {
    let base = msg
        .name
        .clone()
        .or_else(|| fallback.map(|s| s.to_string()))
        .unwrap_or_else(|| "Message".to_string());
    base.to_lowercase()
}

/// Generate `wirecrab_kernel::document::Channel` token stream from IR.
fn channel_to_tokens(ch: &Channel) -> TokenStream {
    let address = match &ch.address {
        Some(addr) => quote! { Some(::std::string::String::from(#addr)) },
        None => quote! { None },
    };
    let title = ch
        .title
        .as_ref()
        .map(|s| quote! { Some(::std::string::String::from(#s)) })
        .unwrap_or(quote! { None });
    let summary = ch
        .summary
        .as_ref()
        .map(|s| quote! { Some(::std::string::String::from(#s)) })
        .unwrap_or(quote! { None });
    let description = ch
        .description
        .as_ref()
        .map(|s| quote! { Some(::std::string::String::from(#s)) })
        .unwrap_or(quote! { None });

    quote! {
        wirecrab_kernel::document::Channel {
            address: #address,
            title: #title,
            summary: #summary,
            description: #description,
            messages: ::std::vec::Vec::new(),
            parameters: ::std::vec::Vec::new(),
            tags: ::std::vec::Vec::new(),
            external_docs: ::std::option::Option::None,
        }
    }
}
