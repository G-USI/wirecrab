use crate::document::common::Item;
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

/// Extract parameter values from encoded message bytes using a channel's parameter definitions.
///
/// Each [`crate::document::channel::AddressParameter`] has a `location` field (e.g.
/// `$message.payload#/user_id`) which is handed to [`Codec::extract_field`] to
/// pull the corresponding value out of the raw payload bytes.
///
/// Returns a map of `parameter name -> stringified value`.
pub fn extract_parameters<C: Codec>(
    codec: &C,
    payload: &[u8],
    parameters: &[Item<crate::document::channel::AddressParameter>],
) -> Result<BTreeMap<String, String>, AnyhowError> {
    let mut result: BTreeMap<String, String> = BTreeMap::new();
    for param in parameters {
        let value = codec.extract_field(payload, &param.item.location)?;
        result.insert(param.key.clone(), value);
    }
    Ok(result)
}

/// Resolve a channel address template by substituting `{param_name}` placeholders.
///
/// Example: `users/{user_id}` + `{"user_id": "42"}` → `users/42`.
///
/// Placeholders without a matching parameter are left untouched; extra
/// parameters that do not appear in the template are silently ignored.
pub fn resolve_address(
    template: &str,
    params: &BTreeMap<String, String>,
) -> Result<String, AnyhowError> {
    let mut result = String::from(template);
    for (key, value) in params {
        let placeholder = {
            let mut s = String::from("{");
            s.push_str(key);
            s.push('}');
            s
        };
        if !result.contains(&placeholder) {
            // Template doesn't have this placeholder — that's fine, skip it.
            continue;
        }
        result = result.replace(&placeholder, value);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::channel::AddressParameter;
    use serde::de::DeserializeOwned;
    use serde::ser::Serialize;
    use std::vec;

    fn s(lit: &str) -> String {
        String::from(lit)
    }

    /// Mock codec that resolves `$message.payload#/<field>` locations against
    /// a hardcoded key→value table. Only used to exercise [`extract_parameters`].
    struct TestCodec {
        values: BTreeMap<String, String>,
    }

    impl Codec for TestCodec {
        fn encode<D>(&self, _value: &D) -> Result<Vec<u8>, AnyhowError>
        where
            D: Serialize + ?Sized,
        {
            Ok(Vec::new())
        }

        fn decode<D>(&self, _bytes: &[u8]) -> Result<D, AnyhowError>
        where
            D: DeserializeOwned,
        {
            Err(anyhow::anyhow!("TestCodec::decode not implemented"))
        }

        fn extract_field(&self, _bytes: &[u8], location: &str) -> Result<String, AnyhowError> {
            // Accept `$message.payload#/foo` or bare `/foo`.
            let key = location
                .rsplit('/')
                .next()
                .ok_or_else(|| anyhow::anyhow!("invalid location: {location}"))?;
            self.values
                .get(key)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("no value for: {key}"))
        }
    }

    #[test]
    fn extract_parameters_walks_channel_params() {
        let codec = TestCodec {
            values: {
                let mut m = BTreeMap::new();
                m.insert(s("user_id"), s("42"));
                m.insert(s("org"), s("acme"));
                m
            },
        };
        let params: Vec<Item<AddressParameter>> = vec![
            Item {
                key: s("user_id"),
                item: AddressParameter {
                    description: None,
                    location: s("$message.payload#/user_id"),
                },
            },
            Item {
                key: s("org"),
                item: AddressParameter {
                    description: None,
                    location: s("$message.payload#/org"),
                },
            },
        ];

        let out = extract_parameters(&codec, b"{}", &params).expect("extract ok");
        assert_eq!(out.get("user_id").map(String::as_str), Some("42"));
        assert_eq!(out.get("org").map(String::as_str), Some("acme"));
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn resolve_address_happy_path() {
        let mut params = BTreeMap::new();
        params.insert(s("user_id"), s("42"));
        let addr = resolve_address("users/{user_id}", &params).expect("resolve ok");
        assert_eq!(addr, "users/42");
    }

    #[test]
    fn resolve_address_static_no_placeholders() {
        let params = BTreeMap::new();
        let addr = resolve_address("users/list", &params).expect("resolve ok");
        assert_eq!(addr, "users/list");
    }

    #[test]
    fn resolve_address_missing_param_leaves_placeholder() {
        // Template has {user_id} but params is empty — still Ok, placeholder stays.
        let params = BTreeMap::new();
        let addr = resolve_address("users/{user_id}", &params).expect("resolve ok");
        assert_eq!(addr, "users/{user_id}");
    }

    #[test]
    fn resolve_address_extra_param_ignored() {
        // params carries a key that does not appear in the template → ignored.
        let mut params = BTreeMap::new();
        params.insert(s("unused"), s("x"));
        let addr = resolve_address("users/list", &params).expect("resolve ok");
        assert_eq!(addr, "users/list");
    }

    #[test]
    fn resolve_address_multiple_placeholders() {
        let mut params = BTreeMap::new();
        params.insert(s("org"), s("acme"));
        params.insert(s("user_id"), s("7"));
        let addr = resolve_address("orgs/{org}/users/{user_id}", &params).expect("resolve ok");
        assert_eq!(addr, "orgs/acme/users/7");
    }
}
