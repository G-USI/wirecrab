use crate::SpecError;
use kernel::document::{
    Action, AddressParameter, Channel, Components, CorrelationId, Document, ExternalDocs, Item,
    Message, Operation, OperationReply, ReplyAddress, Schema, Tag,
};
use kernel::prelude::*;
use serde_json::Value;

pub fn extract_document(value: &Value) -> Result<Document, SpecError> {
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

    Ok(Document {
        operations,
        components,
    })
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
    /// Calls `extract_document` directly on an inline `serde_json::Value` (bypassing
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

        let document = extract_document(&value)
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
}
