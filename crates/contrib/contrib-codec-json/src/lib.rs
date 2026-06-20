#![forbid(unsafe_code)]
//! JSON codec with JSON Schema validation.
//!
//! [`JsonCodec`] binds to one message's [`Schema`] at construction time and
//! performs validation on every [`encode`](Codec::encode) and
//! [`decode`](Codec::decode) call.

use anyhow::{anyhow, bail, Result};
use serde_json::Value;
use wirecrab_kernel::codec::Codec;
use wirecrab_kernel::document::Schema;

/// JSON codec that validates payloads against a JSON Schema.
///
/// Construct one [`JsonCodec`] per message schema. The schema source may be
/// provided as JSON or YAML (detected via [`Schema::format`]).
pub struct JsonCodec {
    validator: jsonschema::Validator,
}

impl JsonCodec {
    /// Create a new [`JsonCodec`] from a message's [`Schema`].
    ///
    /// If [`Schema::format`] contains `"yaml"`, the [`Schema::source`] is
    /// parsed as YAML and converted to a [`Value`] before being compiled into
    /// a [`jsonschema::Validator`]; otherwise the source is parsed as JSON.
    ///
    /// # Errors
    ///
    /// Returns an error if the schema source cannot be parsed or if the
    /// resulting schema cannot be compiled.
    pub fn new(schema: &Schema) -> Result<Self> {
        let schema_value = if schema.format.contains("yaml") {
            let yaml: serde_yaml::Value = serde_yaml::from_str(&schema.source)
                .map_err(|e| anyhow!("Failed to parse YAML schema: {e}"))?;
            serde_json::to_value(&yaml)
                .map_err(|e| anyhow!("Failed to convert YAML to JSON: {e}"))?
        } else {
            serde_json::from_str(&schema.source)
                .map_err(|e| anyhow!("Failed to parse JSON schema: {e}"))?
        };

        let validator = jsonschema::validator_for(&schema_value)
            .map_err(|e| anyhow!("Failed to compile schema: {e}"))?;

        Ok(Self { validator })
    }

    /// Validate `value` against the bound schema, returning the first error
    /// (if any) formatted as a string for embedding into an [`anyhow::Error`].
    fn validate_or_first_error(&self, value: &Value) -> Result<()> {
        if self.validator.is_valid(value) {
            return Ok(());
        }
        let errors: Vec<_> = self.validator.iter_errors(value).collect();
        let first = errors
            .first()
            .map(|e| format!("- {e} (instance path: {})", e.instance_path()))
            .unwrap_or_else(|| "unknown validation error".to_string());
        bail!("Schema validation failed: {first}");
    }
}

impl Codec for JsonCodec {
    fn encode(&self, value: &Value) -> Result<Vec<u8>> {
        self.validate_or_first_error(value)?;
        serde_json::to_vec(value).map_err(|e| anyhow!("Failed to serialize: {e}"))
    }

    fn decode(&self, bytes: &[u8]) -> Result<Value> {
        let value: Value =
            serde_json::from_slice(bytes).map_err(|e| anyhow!("Failed to parse JSON: {e}"))?;
        self.validate_or_first_error(&value)?;
        Ok(value)
    }

    fn extract_field(&self, bytes: &[u8], location: &str) -> Result<String> {
        let value: Value =
            serde_json::from_slice(bytes).map_err(|e| anyhow!("Failed to parse JSON: {e}"))?;

        // Location may be:
        //   * `$message.payload#/path/to/field` (AsyncAPI parameter expression)
        //   * `/path/to/field`                  (plain JSON pointer)
        //   * `fieldName`                       (treated as `/fieldName`)
        let pointer = location
            .strip_prefix("$message.payload#")
            .map(str::to_owned)
            .unwrap_or_else(|| location.to_owned());

        let full_pointer = if pointer.starts_with('/') {
            pointer
        } else {
            format!("/{pointer}")
        };

        match value.pointer(&full_pointer) {
            Some(Value::String(s)) => Ok(s.clone()),
            Some(other) => {
                serde_json::to_string(other).map_err(|e| anyhow!("Failed to serialize field: {e}"))
            }
            None => bail!("Field not found at pointer: {full_pointer}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wirecrab_kernel::document::Schema;

    /// Schema with `event` field that must equal `"ping"`.
    const EVENT_SCHEMA: &str =
        r#"{"type":"object","properties":{"event":{"type":"string","const":"ping"}}}"#;

    fn make_codec(source: &str, format: &str) -> JsonCodec {
        JsonCodec::new(&Schema {
            format: format.to_string(),
            source: source.to_string(),
        })
        .expect("schema should compile")
    }

    #[test]
    fn encode_valid_payload() {
        let codec = make_codec(EVENT_SCHEMA, "application/json");
        let value = json!({"event": "ping"});
        let bytes = codec.encode(&value).expect("valid payload should encode");
        assert_eq!(bytes, br#"{"event":"ping"}"#);
    }

    #[test]
    fn encode_invalid_const() {
        let codec = make_codec(EVENT_SCHEMA, "application/json");
        let value = json!({"event": "pong"});
        let err = codec.encode(&value).expect_err("invalid const should fail");
        assert!(
            format!("{err}").to_lowercase().contains("validation"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn decode_valid_bytes() {
        let codec = make_codec(EVENT_SCHEMA, "application/json");
        let bytes = br#"{"event":"ping"}"#;
        let value = codec.decode(bytes).expect("valid bytes should decode");
        assert_eq!(value, json!({"event": "ping"}));
    }

    #[test]
    fn decode_invalid_json() {
        let codec = make_codec(EVENT_SCHEMA, "application/json");
        let err = codec
            .decode(b"not json")
            .expect_err("invalid JSON should fail");
        assert!(
            format!("{err}").to_lowercase().contains("parse"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn decode_schema_violation() {
        let codec = make_codec(EVENT_SCHEMA, "application/json");
        let err = codec
            .decode(br#"{"event":"pong"}"#)
            .expect_err("schema violation should fail");
        assert!(
            format!("{err}").to_lowercase().contains("validation"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn extract_field_simple() {
        let codec = make_codec(EVENT_SCHEMA, "application/json");
        let bytes = br#"{"event":"ping"}"#;
        let field = codec
            .extract_field(bytes, "$message.payload#/event")
            .expect("existing field should be extractable");
        assert_eq!(field, "ping");
    }

    #[test]
    fn extract_field_nested() {
        let schema = r#"{
            "type":"object",
            "properties":{"user":{"type":"object","properties":{"email":{"type":"string"}}}}
        }"#;
        let codec = make_codec(schema, "application/json");
        let bytes = br#"{"user":{"email":"a@b.com"}}"#;
        let field = codec
            .extract_field(bytes, "$message.payload#/user/email")
            .expect("nested field should be extractable");
        assert_eq!(field, "a@b.com");
    }

    #[test]
    fn extract_field_missing() {
        let codec = make_codec(EVENT_SCHEMA, "application/json");
        let bytes = br#"{"event":"ping"}"#;
        let err = codec
            .extract_field(bytes, "$message.payload#/missing")
            .expect_err("missing field should error");
        assert!(
            format!("{err}").contains("not found"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn yaml_schema_source() {
        let yaml_source = "\
type: object
properties:
  event:
    type: string
    const: ping
";
        let codec = make_codec(yaml_source, "application/vnd.yaml");

        let value = codec
            .decode(br#"{"event":"ping"}"#)
            .expect("YAML-backed codec should decode valid payload");
        assert_eq!(value, json!({"event": "ping"}));

        let err = codec
            .decode(br#"{"event":"pong"}"#)
            .expect_err("schema violation should fail");
        assert!(
            format!("{err}").to_lowercase().contains("validation"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn extract_field_non_string_returns_json_string() {
        let schema = r#"{"type":"object","properties":{"count":{"type":"integer"}}}"#;
        let codec = make_codec(schema, "application/json");
        let bytes = br#"{"count":42}"#;
        let field = codec
            .extract_field(bytes, "$message.payload#/count")
            .expect("integer field should be extractable");
        assert_eq!(field, "42");
    }

    #[test]
    fn extract_field_plain_pointer() {
        let codec = make_codec(EVENT_SCHEMA, "application/json");
        let bytes = br#"{"event":"ping"}"#;
        let field = codec
            .extract_field(bytes, "/event")
            .expect("plain JSON pointer should work");
        assert_eq!(field, "ping");
    }

    #[test]
    fn extract_field_bare_name() {
        let codec = make_codec(EVENT_SCHEMA, "application/json");
        let bytes = br#"{"event":"ping"}"#;
        let field = codec
            .extract_field(bytes, "event")
            .expect("bare field name should work");
        assert_eq!(field, "ping");
    }
}
