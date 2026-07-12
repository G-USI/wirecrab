use crate::ref_resolver::ResolvedDocument;
use crate::SpecError;
use kernel::document::{
    Action, AddressParameter, Channel, Components, CorrelationId, Document, ExternalDocs, Item,
    Message, Operation, OperationReply, ReplyAddress, Schema, Tag,
};
use kernel::prelude::*;
use serde_json::Value;
use std::collections::HashMap;

/// Extract a codegen-ready `Document` IR from a resolved AsyncAPI spec.
///
/// The returned `Document` carries:
/// - `operations`: as resolved from the spec
/// - `messages`: deduplicated by name + payload structure; collisions
///   (same name, different payload) produce `SpecError::ExtractionFailed`
/// - `schemas`: every `components/schemas/*` entry (escape hatch; the
///   macros/CLI layer decides which are actually referenced)
/// - `components`: raw components section for escape-hatch access
pub fn extract_document(resolved: &ResolvedDocument) -> Result<Document, SpecError> {
    let value = &resolved.value;
    let mut operations = Vec::new();

    if let Some(ops) = value.get("operations").and_then(|v| v.as_object()) {
        for (key, op_val) in ops {
            let op = extract_operation(key, op_val)?;
            operations.push(Item {
                key: key.clone(),
                item: op,
            });
        }
    }

    let components = extract_components(value);

    // Build the deduplicated message list. Walk operations + their channels,
    // collect messages, dedupe by (name, payload.source). If two messages
    // share a name but have different payloads, that's a collision — error.
    let messages = collect_unique_messages(&operations)?;

    // Pull every named schema from components/schemas/* verbatim.
    let schemas: Vec<Item<Schema>> = components
        .as_ref()
        .map(|c| c.schemas.clone())
        .unwrap_or_default();

    Ok(Document {
        operations,
        messages,
        schemas,
        components,
    })
}

/// Walk operations and their channels, deduplicating messages.
///
/// Naming precedence (spec-extract policy):
///   1. `message.name` if set
///   2. `message.title` if set
///   3. The containing key (channel key, or component key)
///
/// Dedup policy:
/// - Messages that resolve to a canonical name: dedupe by name. Two
///   messages sharing a name MUST have the same payload source, else
///   this is an ambiguous spec and we error.
/// - Messages that don't resolve (no name, no title, no key, e.g. an
///   unnamed entry in an operation's `messages` array): always included
///   verbatim. No dedup, no collision check. Their `Item.key` is
///   synthesized as `<op>#<index>`.
fn collect_unique_messages(operations: &[Item<Operation>]) -> Result<Vec<Item<Message>>, SpecError> {
    let mut result: Vec<Item<Message>> = Vec::new();
    let mut named_index: HashMap<String, usize> = HashMap::new();
    let mut anon_counter: usize = 0;

    for op in operations {
        // Operation-level messages: list, no key
        for msg in &op.item.messages {
            let name = canonical_name(msg, None);
            register_message(
                msg,
                name,
                &op.key,
                &mut result,
                &mut named_index,
                &mut anon_counter,
            )?;
        }
        // Channel-level messages: keyed map
        for item in &op.item.channel.messages {
            let name = canonical_name(&item.item, Some(&item.key));
            register_message(
                &item.item,
                name,
                &op.key,
                &mut result,
                &mut named_index,
                &mut anon_counter,
            )?;
        }
    }

    Ok(result)
}

/// Resolve the canonical name for a message per the precedence policy.
fn canonical_name(msg: &Message, fallback_key: Option<&str>) -> Option<String> {
    msg.name
        .clone()
        .or_else(|| msg.title.clone())
        .or_else(|| fallback_key.map(|s| s.to_string()))
}

#[allow(clippy::too_many_arguments)]
fn register_message(
    msg: &Message,
    name: Option<String>,
    op_key: &str,
    result: &mut Vec<Item<Message>>,
    named_index: &mut HashMap<String, usize>,
    anon_counter: &mut usize,
) -> Result<(), SpecError> {
    let payload_sig = msg
        .payload
        .as_ref()
        .map(|p| p.source.clone())
        .unwrap_or_default();

    match name {
        Some(name) => {
            if let Some(&idx) = named_index.get(&name) {
                let existing_sig = result[idx]
                    .item
                    .payload
                    .as_ref()
                    .map(|p| p.source.clone())
                    .unwrap_or_default();
                if existing_sig != payload_sig {
                    return Err(SpecError::ExtractionFailed(format!(
                        "message name collision: '{}' refers to distinct payloads\n  existing: {}\n  new:      {}",
                        name, existing_sig, payload_sig
                    )));
                }
                // Same name + same payload → already registered, skip.
            } else {
                named_index.insert(name.clone(), result.len());
                result.push(Item {
                    key: name,
                    item: msg.clone(),
                });
            }
            Ok(())
        }
        None => {
            let key = format!("{op_key}#{anon_counter}");
            *anon_counter += 1;
            result.push(Item {
                key,
                item: msg.clone(),
            });
            Ok(())
        }
    }
}

fn extract_operation(key: &str, value: &Value) -> Result<Operation, SpecError> {
    let action = match value.get("action").and_then(|v| v.as_str()) {
        Some("send") => Action::Send,
        Some("receive") => Action::Receive,
        Some(other) => {
            return Err(SpecError::ExtractionFailed(format!(
                "operation '{key}': invalid action '{other}'"
            )));
        }
        None => {
            return Err(SpecError::ExtractionFailed(format!(
                "operation '{key}': missing 'action' field"
            )));
        }
    };

    let channel = value
        .get("channel")
        .ok_or_else(|| {
            SpecError::ExtractionFailed(format!("operation '{key}': missing 'channel' field"))
        })
        .and_then(extract_channel)?;

    let messages = extract_message_list(value.get("messages"))?;

    let reply = value
        .get("reply")
        .and_then(|v| if v.is_null() { None } else { Some(v) })
        .map(extract_operation_reply)
        .transpose()?;

    Ok(Operation {
        action,
        channel,
        messages: messages.into_iter().map(|i| i.item).collect(),
        reply,
        title: get_str(value, "title"),
        summary: get_str(value, "summary"),
        description: get_str(value, "description"),
        tags: extract_tags(value.get("tags")),
        external_docs: extract_external_docs(value.get("externalDocs")),
    })
}

fn extract_channel(value: &Value) -> Result<Channel, SpecError> {
    let mut messages = Vec::new();
    if let Some(msgs) = value.get("messages").and_then(|v| v.as_object()) {
        for (msg_key, msg_val) in msgs {
            let msg = extract_message(msg_val)?;
            messages.push(Item {
                key: msg_key.clone(),
                item: msg,
            });
        }
    }

    let mut parameters = Vec::new();
    if let Some(params) = value.get("parameters").and_then(|v| v.as_object()) {
        for (param_key, param_val) in params {
            let param = extract_address_parameter(param_val)?;
            parameters.push(Item {
                key: param_key.clone(),
                item: param,
            });
        }
    }

    Ok(Channel {
        address: get_str(value, "address"),
        title: get_str(value, "title"),
        summary: get_str(value, "summary"),
        description: get_str(value, "description"),
        messages,
        parameters,
        tags: extract_tags(value.get("tags")),
        external_docs: extract_external_docs(value.get("externalDocs")),
    })
}

fn extract_message(value: &Value) -> Result<Message, SpecError> {
    Ok(Message {
        name: get_str(value, "name"),
        title: get_str(value, "title"),
        summary: get_str(value, "summary"),
        description: get_str(value, "description"),
        content_type: get_str(value, "contentType"),
        headers: value.get("headers").map(extract_schema),
        payload: value.get("payload").map(extract_schema),
        deprecated: value
            .get("deprecated")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        correlation_id: value
            .get("correlationId")
            .and_then(|v| if v.is_null() { None } else { Some(v) })
            .map(extract_correlation_id),
        tags: extract_tags(value.get("tags")),
        external_docs: extract_external_docs(value.get("externalDocs")),
    })
}

fn extract_address_parameter(value: &Value) -> Result<AddressParameter, SpecError> {
    Ok(AddressParameter {
        location: get_str(value, "location").unwrap_or_default(),
        description: get_str(value, "description"),
    })
}

fn extract_operation_reply(value: &Value) -> Result<OperationReply, SpecError> {
    let channel = value.get("channel").map(extract_channel).transpose()?;

    let messages = extract_message_list(value.get("messages"))?;

    let address = value
        .get("address")
        .filter(|v| v.is_object())
        .map(|v| ReplyAddress {
            location: v
                .get("location")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            description: get_str(v, "description"),
        });

    Ok(OperationReply {
        address,
        channel,
        messages,
    })
}

fn extract_message_list(value: Option<&Value>) -> Result<Vec<Item<Message>>, SpecError> {
    match value {
        Some(Value::Array(arr)) => arr
            .iter()
            .map(|v| {
                let msg = extract_message(v)?;
                let key = msg.name.clone().unwrap_or_default();
                Ok(Item { key, item: msg })
            })
            .collect(),
        _ => Ok(Vec::new()),
    }
}

fn extract_components(value: &Value) -> Option<Components> {
    let comp = value.get("components")?;

    let messages = comp
        .get("messages")
        .and_then(|v| v.as_object())
        .map(|obj| {
            obj.iter()
                .filter_map(|(k, v)| match extract_message(v) {
                    Ok(msg) => Some(Item {
                        key: k.clone(),
                        item: msg,
                    }),
                    Err(_) => None,
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let schemas = comp
        .get("schemas")
        .and_then(|v| v.as_object())
        .map(|obj| {
            obj.iter()
                .map(|(k, v)| Item {
                    key: k.clone(),
                    item: extract_schema(v),
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    Some(Components { messages, schemas })
}

fn extract_schema(value: &Value) -> Schema {
    if let Some(format) = value.get("schemaFormat").and_then(|v| v.as_str()) {
        let source = match value.get("schema") {
            Some(Value::String(s)) => s.clone(),
            Some(other) => other.to_string(),
            None => String::new(),
        };
        Schema {
            format: format.to_string(),
            source,
        }
    } else {
        Schema {
            format: "application/vnd.asyncapi".to_string(),
            source: value.to_string(),
        }
    }
}

fn extract_correlation_id(value: &Value) -> CorrelationId {
    CorrelationId {
        location: get_str(value, "location").unwrap_or_default(),
        description: get_str(value, "description"),
    }
}

fn extract_tags(value: Option<&Value>) -> Vec<Tag> {
    match value.and_then(|v| v.as_array()) {
        Some(arr) => arr
            .iter()
            .filter_map(|v| {
                Some(Tag {
                    name: v.get("name")?.as_str()?.to_string(),
                    description: get_str(v, "description"),
                    external_docs: extract_external_docs(v.get("externalDocs")),
                })
            })
            .collect(),
        None => Vec::new(),
    }
}

fn extract_external_docs(value: Option<&Value>) -> Option<ExternalDocs> {
    let v = value.and_then(|v| if v.is_null() { None } else { Some(v) })?;
    Some(ExternalDocs {
        url: get_str(v, "url").unwrap_or_default(),
        description: get_str(v, "description"),
    })
}

fn get_str(value: &Value, field: &str) -> Option<String> {
    value.get(field).and_then(|v| v.as_str()).map(String::from)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::parse;
    use kernel::document::Action;

    fn fixture(name: &str) -> String {
        format!(
            "{}/../../submodules/asyncapi-spec/examples/{name}",
            env!("CARGO_MANIFEST_DIR")
        )
    }

    fn op_by_key<'a>(document: &'a Document, key: &str) -> &'a Operation {
        document
            .operations
            .iter()
            .find(|i| i.key == key)
            .map(|i| &i.item)
            .unwrap_or_else(|| panic!("operation '{key}' must exist"))
    }

    fn msg_by_key<'a>(messages: &'a [Item<Message>], key: &str) -> &'a Message {
        messages
            .iter()
            .find(|i| i.key == key)
            .map(|i| &i.item)
            .unwrap_or_else(|| panic!("message '{key}' must exist"))
    }

    /// Regression baseline for the streetlights-kafka fixture.
    ///
    /// Asserts the refactored IR shape: `Document { operations: Vec<Item<Operation>>,
    /// components: Option<Components> }` with keys carried by `Item` wrappers.
    #[test]
    fn extract_streetlights_kafka() {
        let document = parse(fixture("streetlights-kafka-asyncapi.yml"))
            .expect("streetlights-kafka fixture must parse");

        assert_eq!(document.operations.len(), 4);

        let op_keys: Vec<&str> = document.operations.iter().map(|i| i.key.as_str()).collect();
        assert!(op_keys.contains(&"receiveLightMeasurement"));
        assert!(op_keys.contains(&"turnOn"));
        assert!(op_keys.contains(&"turnOff"));
        assert!(op_keys.contains(&"dimLight"));

        let recv = op_by_key(&document, "receiveLightMeasurement");
        let turn_on = op_by_key(&document, "turnOn");

        assert!(matches!(recv.action, Action::Receive));
        assert!(matches!(turn_on.action, Action::Send));

        assert_eq!(
            recv.channel.address.as_deref(),
            Some("smartylighting.streetlights.1.0.event.{streetlightId}.lighting.measured")
        );
        assert_eq!(
            turn_on.channel.address.as_deref(),
            Some("smartylighting.streetlights.1.0.action.{streetlightId}.turn.on")
        );

        let light_measured_item = recv
            .channel
            .messages
            .iter()
            .find(|i| i.key == "lightMeasured")
            .expect("channel.messages must contain 'lightMeasured' after $ref resolution");
        assert_eq!(light_measured_item.key, "lightMeasured");

        let recv_msg = &light_measured_item.item;
        assert_eq!(recv_msg.name.as_deref(), Some("lightMeasured"));
        assert_eq!(recv_msg.title.as_deref(), Some("Light measured"));
        assert_eq!(recv_msg.content_type.as_deref(), Some("application/json"));
        assert!(
            recv_msg.payload.is_some(),
            "payload must be Some after $ref resolution"
        );

        let payload = recv_msg.payload.as_ref().expect("payload is Some");
        assert_eq!(payload.format, "application/vnd.asyncapi");
        assert!(
            payload.source.contains("lumens"),
            "payload source must contain resolved 'lumens' field, got: {}",
            payload.source
        );

        assert_eq!(recv.messages.len(), 1);
    }

    /// Regression baseline for the simple-asyncapi fixture.
    #[test]
    fn extract_simple_asyncapi() {
        let document =
            parse(fixture("simple-asyncapi.yml")).expect("simple-asyncapi fixture must parse");

        assert_eq!(document.operations.len(), 1);
        let op = op_by_key(&document, "sendUserSignedup");

        assert!(matches!(op.action, Action::Send));

        assert_eq!(op.channel.address.as_deref(), Some("user/signedup"));

        assert_eq!(op.channel.messages.len(), 1);
        let msg = msg_by_key(&op.channel.messages, "UserSignedUp");

        assert!(msg.payload.is_some(), "payload must be Some");
        let payload = msg.payload.as_ref().expect("payload is Some");
        assert!(payload.source.contains("displayName"));
        assert!(payload.source.contains("email"));
    }

    /// Verifies the extractor does not depend on a `components` section being present.
    ///
    /// Calls `extract_document` directly on an inline `ResolvedDocument` (bypassing
    /// resolver + jsonschema validation) so the test exercises only the extract layer.
    #[test]
    fn extract_no_components_does_not_crash() {
        let value = serde_json::json!({
            "asyncapi": "3.1.0",
            "info": {
                "title": "No Components Test",
                "version": "1.0.0"
            },
            "channels": {
                "greetings": {
                    "address": "greet/hello",
                    "messages": {
                        "hello": {
                            "contentType": "application/json",
                            "payload": {
                                "type": "object",
                                "properties": {
                                    "msg": { "type": "string" }
                                }
                            }
                        }
                    }
                }
            },
            "operations": {
                "sayHello": {
                    "action": "send",
                    "channel": {
                        "address": "greet/hello",
                        "messages": {
                            "hello": {
                                "contentType": "application/json",
                                "payload": { "type": "object" }
                            }
                        }
                    },
                    "messages": [
                        {
                            "contentType": "application/json",
                            "payload": { "type": "object" }
                        }
                    ]
                }
            }
        });

        let resolved = ResolvedDocument {
            value: value.clone(),
            provenance: crate::ref_resolver::ProvenanceRegistry::default(),
        };
        let document = extract_document(&resolved)
            .expect("extract_document must succeed on a spec with no components");

        assert!(!document.operations.is_empty());
        assert_eq!(document.operations.len(), 1);
        assert!(
            document.components.is_none(),
            "no components section in input → None in output"
        );

        let op = op_by_key(&document, "sayHello");

        assert!(matches!(op.action, Action::Send));

        assert_eq!(op.channel.address.as_deref(), Some("greet/hello"));

        assert_eq!(op.channel.messages.len(), 1);
        assert!(op.channel.messages.iter().any(|i| i.key == "hello"));

        assert_eq!(op.messages.len(), 1);
        assert!(op.messages[0].payload.is_some());
        assert_eq!(
            op.messages[0].content_type.as_deref(),
            Some("application/json")
        );

        let hello_item = op
            .channel
            .messages
            .iter()
            .find(|i| i.key == "hello")
            .expect("hello message must exist");
        assert_eq!(hello_item.key, "hello");
    }

    /// Verifies that `components.messages` and `components.schemas` are extracted
    /// into `Document.components` as `Vec<Item<...>>` keyed by their component name.
    #[test]
    fn extract_components_from_streetlights() {
        let document = parse(fixture("streetlights-kafka-asyncapi.yml"))
            .expect("streetlights-kafka fixture must parse");

        let components = document
            .components
            .as_ref()
            .expect("streetlights-kafka must have components");

        assert!(
            !components.messages.is_empty(),
            "components.messages must be populated"
        );
        let msg_keys: Vec<&str> = components.messages.iter().map(|i| i.key.as_str()).collect();
        assert!(
            msg_keys.contains(&"lightMeasured"),
            "components.messages must contain 'lightMeasured', got: {msg_keys:?}"
        );

        let light_measured = msg_by_key(&components.messages, "lightMeasured");
        assert_eq!(light_measured.name.as_deref(), Some("lightMeasured"));
        assert_eq!(light_measured.title.as_deref(), Some("Light measured"));
        assert!(light_measured.payload.is_some());

        assert!(
            !components.schemas.is_empty(),
            "components.schemas must be populated"
        );
        let schema_keys: Vec<&str> = components.schemas.iter().map(|i| i.key.as_str()).collect();
        assert!(
            schema_keys.contains(&"lightMeasuredPayload"),
            "components.schemas must contain 'lightMeasuredPayload', got: {schema_keys:?}"
        );
    }

    // =========================================================================
    // Dedup + collision-rule tests
    // =========================================================================

    fn empty_provenance() -> crate::ref_resolver::ProvenanceRegistry {
        crate::ref_resolver::ProvenanceRegistry::default()
    }

    fn resolved(value: Value) -> ResolvedDocument {
        ResolvedDocument {
            value,
            provenance: empty_provenance(),
        }
    }

    /// Helper: a single-operation spec with one channel message.
    fn spec_with_op_message(op_key: &str, msg_key: &str, payload_type: &str) -> Value {
        serde_json::json!({
            "operations": {
                op_key: {
                    "action": "send",
                    "channel": {
                        "address": "test",
                        "messages": {
                            msg_key: {
                                "contentType": "application/json",
                                "payload": { "type": payload_type }
                            }
                        }
                    }
                }
            }
        })
    }

    /// Channel message key is used as canonical name when `name`/`title` are absent.
    #[test]
    fn extract_message_canonical_name_falls_back_to_channel_key() {
        let value = spec_with_op_message("sendFoo", "Foo", "object");
        let document = extract_document(&resolved(value)).expect("extract must succeed");

        assert_eq!(document.messages.len(), 1);
        assert_eq!(document.messages[0].key, "Foo");
    }

    /// Explicit `name` field takes precedence over channel key.
    #[test]
    fn extract_message_canonical_name_prefers_explicit_name() {
        let value = serde_json::json!({
            "operations": {
                "sendFoo": {
                    "action": "send",
                    "channel": {
                        "address": "test",
                        "messages": {
                            "channel_key_ignored": {
                                "name": "ExplicitName",
                                "contentType": "application/json",
                                "payload": { "type": "object" }
                            }
                        }
                    }
                }
            }
        });
        let document = extract_document(&resolved(value)).expect("extract must succeed");

        assert_eq!(document.messages.len(), 1);
        assert_eq!(document.messages[0].key, "ExplicitName");
    }

    /// `title` is used when `name` is absent and channel key is unavailable.
    #[test]
    fn extract_message_canonical_name_uses_title() {
        let value = serde_json::json!({
            "operations": {
                "sendFoo": {
                    "action": "send",
                    "channel": {
                        "address": "test",
                        "messages": {
                            "channel_key": {
                                "title": "TitledMessage",
                                "contentType": "application/json",
                                "payload": { "type": "object" }
                            }
                        }
                    }
                }
            }
        });
        let document = extract_document(&resolved(value)).expect("extract must succeed");

        assert_eq!(document.messages.len(), 1);
        assert_eq!(document.messages[0].key, "TitledMessage");
    }

    /// Same message referenced from multiple operations dedupes to one entry.
    #[test]
    fn extract_dedupes_same_message_referenced_twice() {
        let payload = serde_json::json!({"type": "object", "properties": {"n": {"type": "integer"}}});
        let value = serde_json::json!({
            "operations": {
                "sendFoo": {
                    "action": "send",
                    "channel": {
                        "address": "test",
                        "messages": {
                            "Foo": {
                                "contentType": "application/json",
                                "payload": payload
                            }
                        }
                    }
                },
                "receiveFoo": {
                    "action": "receive",
                    "channel": {
                        "address": "test",
                        "messages": {
                            "Foo": {
                                "contentType": "application/json",
                                "payload": payload
                            }
                        }
                    }
                }
            }
        });
        let document = extract_document(&resolved(value)).expect("extract must succeed");

        assert_eq!(
            document.messages.len(),
            1,
            "same name + same payload must dedupe to one entry"
        );
        assert_eq!(document.messages[0].key, "Foo");
    }

    /// Two messages with same name but DIFFERENT payloads → extraction fails.
    #[test]
    fn extract_collision_same_name_different_payload_errors() {
        let value = serde_json::json!({
            "operations": {
                "sendFoo": {
                    "action": "send",
                    "channel": {
                        "address": "test",
                        "messages": {
                            "Foo": {
                                "name": "Foo",
                                "contentType": "application/json",
                                "payload": { "type": "object", "properties": { "a": { "type": "string" } } }
                            }
                        }
                    }
                },
                "receiveFoo": {
                    "action": "receive",
                    "channel": {
                        "address": "test",
                        "messages": {
                            "Bar": {
                                "name": "Foo",
                                "contentType": "application/json",
                                "payload": { "type": "object", "properties": { "b": { "type": "integer" } } }
                            }
                        }
                    }
                }
            }
        });
        let err = extract_document(&resolved(value))
            .expect_err("collision must produce SpecError");
        let msg = err.to_string();
        assert!(
            msg.contains("collision") && msg.contains("Foo"),
            "error must name the colliding message, got: {msg}"
        );
    }

    /// Anonymous messages (no name, no title, in operation `messages` array)
    /// are included verbatim with synthesized keys; no collision check.
    #[test]
    fn extract_anonymous_messages_included_verbatim() {
        let value = serde_json::json!({
            "operations": {
                "multi": {
                    "action": "send",
                    "channel": { "address": "test" },
                    "messages": [
                        { "contentType": "application/json", "payload": { "type": "object", "properties": { "a": { "type": "string" } } } },
                        { "contentType": "application/json", "payload": { "type": "object", "properties": { "b": { "type": "integer" } } } }
                    ]
                }
            }
        });
        let document = extract_document(&resolved(value)).expect("extract must succeed");

        assert_eq!(
            document.messages.len(),
            2,
            "two distinct anonymous messages must both appear"
        );
        assert!(
            document.messages[0].key.starts_with("multi#"),
            "anonymous key must be synthesized from op key, got: {}",
            document.messages[0].key
        );
        assert!(document.messages[1].key != document.messages[0].key);
    }

    /// `components/schemas/*` flows into `document.schemas` verbatim.
    #[test]
    fn extract_populates_document_schemas_from_components() {
        let value = serde_json::json!({
            "operations": {
                "sendFoo": {
                    "action": "send",
                    "channel": { "address": "test" }
                }
            },
            "components": {
                "schemas": {
                    "User": { "type": "object", "properties": { "id": { "type": "string" } } },
                    "Admin": { "type": "object", "properties": { "perms": { "type": "array", "items": { "type": "string" } } } }
                }
            }
        });
        let document = extract_document(&resolved(value)).expect("extract must succeed");

        let schema_keys: Vec<&str> =
            document.schemas.iter().map(|i| i.key.as_str()).collect();
        assert!(schema_keys.contains(&"User"), "schemas must include User: {schema_keys:?}");
        assert!(schema_keys.contains(&"Admin"), "schemas must include Admin: {schema_keys:?}");
    }
}
