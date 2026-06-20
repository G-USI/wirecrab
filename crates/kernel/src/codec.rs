use crate::utils::structs::*;
use serde::de::DeserializeOwned;
use serde::ser::Serialize;

/// A codec encodes/decodes messages to/from bytes, with schema validation.
///
/// The codec binds to ONE message's schema at construction time.
/// Validation is inherent to decode — invalid messages return Err.
///
/// The generic type parameters allow callers to decode directly into a typed
/// struct (`codec.decode::<MyPayload>(bytes)`) rather than going through
/// `serde_json::Value`. For dynamic/schema-free decoding, use
/// `codec.decode::<serde_json::Value>(bytes)`.
pub trait Codec: Send + Sync {
    /// Encode a serializable value into bytes (with schema validation).
    fn encode<D>(&self, value: &D) -> Result<Vec<u8>, AnyhowError>
    where
        D: Serialize + ?Sized;

    /// Decode bytes into a deserializable type (with schema validation).
    /// Returns Err if bytes are invalid JSON OR fail schema validation.
    fn decode<D>(&self, bytes: &[u8]) -> Result<D, AnyhowError>
    where
        D: DeserializeOwned;

    /// Extract a field from raw message bytes using a location string.
    /// Location format: `$message.payload#/path/to/field` (AsyncAPI parameter expression)
    /// or a plain JSON pointer like `/path/to/field`.
    /// Returns the field value as a string.
    fn extract_field(&self, bytes: &[u8], location: &str) -> Result<String, AnyhowError>;
}
