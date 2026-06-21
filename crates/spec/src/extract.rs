use crate::SpecError;
use kernel::document::{
    Action, AddressParameter, Channel, CorrelationId, Document, ExternalDocs, Message, Operation,
    OperationReply, ReplyAddress, Schema, Tag,
};
use kernel::prelude::*;
use serde_json::Value;

pub fn extract_document(value: &Value) -> Result<Document, SpecError> {
    let mut operations = BTreeMap::new();

    if let Some(ops) = value.get("operations").and_then(|v| v.as_object()) {
        for (key, op_val) in ops {
            let op = extract_operation(key, op_val)?;
            operations.insert(key.clone(), op);
        }
    }

    Ok(Document { operations })
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
        .and_then(|v| extract_channel("", v))?;

    let messages = extract_message_list(value.get("messages"))?;

    let reply = value
        .get("reply")
        .and_then(|v| if v.is_null() { None } else { Some(v) })
        .map(extract_operation_reply)
        .transpose()?;

    Ok(Operation {
        action,
        key: key.to_string(),
        channel,
        messages,
        reply,
        title: get_str(value, "title"),
        summary: get_str(value, "summary"),
        description: get_str(value, "description"),
        tags: extract_tags(value.get("tags")),
        external_docs: extract_external_docs(value.get("externalDocs")),
    })
}

fn extract_channel(key: &str, value: &Value) -> Result<Channel, SpecError> {
    let mut messages = BTreeMap::new();
    if let Some(msgs) = value.get("messages").and_then(|v| v.as_object()) {
        for (msg_key, msg_val) in msgs {
            let msg = extract_message(msg_key, msg_val)?;
            messages.insert(msg_key.clone(), msg);
        }
    }

    let mut parameters = BTreeMap::new();
    if let Some(params) = value.get("parameters").and_then(|v| v.as_object()) {
        for (param_key, param_val) in params {
            let param = extract_address_parameter(param_key, param_val)?;
            parameters.insert(param_key.clone(), param);
        }
    }

    Ok(Channel {
        key: key.to_string(),
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

fn extract_message(key: &str, value: &Value) -> Result<Message, SpecError> {
    Ok(Message {
        key: key.to_string(),
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

fn extract_address_parameter(key: &str, value: &Value) -> Result<AddressParameter, SpecError> {
    Ok(AddressParameter {
        key: key.to_string(),
        location: get_str(value, "location").unwrap_or_default(),
        description: get_str(value, "description"),
    })
}

fn extract_operation_reply(value: &Value) -> Result<OperationReply, SpecError> {
    let channel = value
        .get("channel")
        .map(|v| extract_channel("", v))
        .transpose()?;

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

fn extract_message_list(value: Option<&Value>) -> Result<Vec<Message>, SpecError> {
    match value {
        Some(Value::Array(arr)) => arr.iter().map(|v| extract_message("", v)).collect(),
        _ => Ok(Vec::new()),
    }
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

    /// Regression baseline for the streetlights-kafka fixture.
    ///
    /// Asserts the CURRENT IR shape: `Document { operations: BTreeMap<String, Operation> }`
    /// with `Operation.key`, `Channel.key`, and `Message.key` fields populated.
    /// These assertions WILL break after the planned IR refactor (T3) — that is expected,
    /// T3 will update them to match the new shape.
    #[test]
    fn extract_streetlights_kafka() {
        let document = parse(fixture("streetlights-kafka-asyncapi.yml"))
            .expect("streetlights-kafka fixture must parse");

        assert_eq!(document.operations.len(), 4);

        assert!(document.operations.contains_key("receiveLightMeasurement"));
        assert!(document.operations.contains_key("turnOn"));
        assert!(document.operations.contains_key("turnOff"));
        assert!(document.operations.contains_key("dimLight"));

        let recv = document
            .operations
            .get("receiveLightMeasurement")
            .expect("receiveLightMeasurement must exist");
        let turn_on = document
            .operations
            .get("turnOn")
            .expect("turnOn must exist");

        assert!(matches!(recv.action, Action::Receive));
        assert_eq!(recv.key, "receiveLightMeasurement");
        assert!(matches!(turn_on.action, Action::Send));
        assert_eq!(turn_on.key, "turnOn");

        assert_eq!(
            recv.channel.address.as_deref(),
            Some("smartylighting.streetlights.1.0.event.{streetlightId}.lighting.measured")
        );
        assert_eq!(
            turn_on.channel.address.as_deref(),
            Some("smartylighting.streetlights.1.0.action.{streetlightId}.turn.on")
        );

        let recv_msg = recv
            .channel
            .messages
            .get("lightMeasured")
            .expect("channel.messages[\"lightMeasured\"] must exist after $ref resolution");

        assert_eq!(recv_msg.key, "lightMeasured");
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
        assert_eq!(recv.messages[0].key, "");
    }

    /// Regression baseline for the simple-asyncapi fixture.
    #[test]
    fn extract_simple_asyncapi() {
        let document =
            parse(fixture("simple-asyncapi.yml")).expect("simple-asyncapi fixture must parse");

        assert_eq!(document.operations.len(), 1);
        let op = document
            .operations
            .get("sendUserSignedup")
            .expect("sendUserSignedup must exist");

        assert!(matches!(op.action, Action::Send));
        assert_eq!(op.key, "sendUserSignedup");

        assert_eq!(op.channel.address.as_deref(), Some("user/signedup"));

        assert_eq!(op.channel.messages.len(), 1);
        let msg = op
            .channel
            .messages
            .get("UserSignedUp")
            .expect("UserSignedUp message must exist");

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

        let op = document
            .operations
            .get("sayHello")
            .expect("sayHello must exist");

        assert!(matches!(op.action, Action::Send));
        assert_eq!(op.key, "sayHello");

        assert_eq!(op.channel.address.as_deref(), Some("greet/hello"));

        assert_eq!(op.channel.messages.len(), 1);
        assert!(op.channel.messages.contains_key("hello"));

        assert_eq!(op.messages.len(), 1);
        assert!(op.messages[0].payload.is_some());
        assert_eq!(
            op.messages[0].content_type.as_deref(),
            Some("application/json")
        );
        assert_eq!(op.channel.messages["hello"].key, "hello");
    }
}
