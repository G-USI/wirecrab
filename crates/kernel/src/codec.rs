use crate::utils::structs::*;

/// A codec encodes/decodes messages to/from bytes, with schema validation.
///
/// The codec binds to ONE message's schema at construction time.
/// Validation is inherent to decode — invalid messages return Err.
pub trait Codec: Send + Sync {
    /// Encode a JSON value into bytes (with schema validation).
    fn encode(&self, value: &serde_json::Value) -> Result<Vec<u8>, AnyhowError>;

    /// Decode bytes into a JSON value (with schema validation).
    /// Returns Err if bytes are invalid JSON OR fail schema validation.
    fn decode(&self, bytes: &[u8]) -> Result<serde_json::Value, AnyhowError>;

    /// Extract a field from raw message bytes using a location string.
    /// Location format: `$message.payload#/path/to/field` (AsyncAPI parameter expression)
    /// or a plain JSON pointer like `/path/to/field`.
    /// Returns the field value as a string.
    fn extract_field(&self, bytes: &[u8], location: &str) -> Result<String, AnyhowError>;
}
