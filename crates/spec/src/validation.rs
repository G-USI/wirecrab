//
// SPDX-License-Identifier: Apache-2.0

//! JSON Schema validation for AsyncAPI documents.

use crate::SpecError;
use serde_json::Value;

const ASYNCAPI_3_0_0_SCHEMA: &str = include_str!("../schemas/asyncapi-3.0.0.json");
const ASYNCAPI_3_1_0_SCHEMA: &str = include_str!("../schemas/asyncapi-3.1.0.json");

/// Validates an AsyncAPI document against its spec version schema.
///
/// # Errors
///
/// Returns `SpecError` if:
/// - The document is missing `asyncapi` field
/// - The `asyncapi` version is not supported
/// - The document does not conform to the JSON schema
pub fn validate(document: &Value) -> Result<(), SpecError> {
    let schema_str = match document.get("asyncapi").and_then(|v| v.as_str()) {
        Some("3.0.0") => ASYNCAPI_3_0_0_SCHEMA,
        Some("3.1.0") => ASYNCAPI_3_1_0_SCHEMA,
        Some(unsupported) => return Err(SpecError::UnsupportedVersion(unsupported.into())),
        None => {
            return Err(SpecError::InvalidSpec("Missing 'asyncapi' field".into()));
        }
    };

    // -- Parse schema
    let schema: Value = serde_json::from_str(schema_str)
        .map_err(|e| SpecError::InvalidSchema(format!("Failed to parse schema: {}", e)))?;

    // -- Create validator
    // -- The schemas are bundled with all definitions inline, so jsonschema
    // -- should resolve references internally without network access
    let validator = jsonschema::validator_for(&schema)
        .map_err(|e| SpecError::InvalidSchema(format!("Failed to create validator: {}", e)))?;

    // -- Validate document
    if !validator.is_valid(document) {
        let errors: Vec<_> = validator.iter_errors(document).collect();
        return Err(SpecError::ValidationFailed(format!(
            "Document validation failed with {} error(s): {}",
            errors.len(),
            format_errors(errors)
        )));
    }

    Ok(())
}

fn format_errors(errors: Vec<jsonschema::ValidationError>) -> String {
    errors
        .iter()
        .map(|e| format!("- {} (instance path: {})", e, e.instance_path()))
        .collect::<Vec<_>>()
        .join("; ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn validate_minimal_valid_document() {
        let doc = json!({
            "asyncapi": "3.0.0",
            "info": {
                "title": "Test API",
                "version": "1.0.0"
            }
        });

        assert!(validate(&doc).is_ok());
    }

    #[test]
    fn validate_missing_asyncapi_field() {
        let doc = json!({
            "info": {
                "title": "Test API",
                "version": "1.0.0"
            }
        });

        assert!(matches!(validate(&doc), Err(SpecError::InvalidSpec(_))));
    }

    #[test]
    fn validate_unsupported_version() {
        let doc = json!({
            "asyncapi": "2.0.0",
            "info": {
                "title": "Test API",
                "version": "1.0.0"
            }
        });

        assert!(matches!(
            validate(&doc),
            Err(SpecError::UnsupportedVersion(_))
        ));
    }

    #[test]
    fn validate_missing_required_field() {
        let doc = json!({
            "asyncapi": "3.0.0",
            "info": {
                "title": "Test API"
            }
        });

        assert!(matches!(
            validate(&doc),
            Err(SpecError::ValidationFailed(_))
        ));
    }

    #[test]
    fn validate_valid_3_1_document() {
        let doc = json!({
            "asyncapi": "3.1.0",
            "info": {
                "title": "Test API",
                "version": "1.0.0"
            }
        });

        assert!(validate(&doc).is_ok());
    }

    #[test]
    fn validate_non_string_asyncapi_field() {
        let doc = json!({
            "asyncapi": 123,
            "info": {
                "title": "Test API",
                "version": "1.0.0"
            }
        });

        assert!(matches!(validate(&doc), Err(SpecError::InvalidSpec(_))));
    }

    #[test]
    fn validate_invalid_info_type() {
        let doc = json!({
            "asyncapi": "3.0.0",
            "info": "not an object"
        });

        assert!(matches!(
            validate(&doc),
            Err(SpecError::ValidationFailed(_))
        ));
    }

    #[test]
    fn validate_invalid_title_type() {
        let doc = json!({
            "asyncapi": "3.0.0",
            "info": {
                "title": 123,
                "version": "1.0.0"
            }
        });

        assert!(matches!(
            validate(&doc),
            Err(SpecError::ValidationFailed(_))
        ));
    }

    #[test]
    fn validate_invalid_version_type() {
        let doc = json!({
            "asyncapi": "3.0.0",
            "info": {
                "title": "Test API",
                "version": 123
            }
        });

        assert!(matches!(
            validate(&doc),
            Err(SpecError::ValidationFailed(_))
        ));
    }

    #[test]
    fn validate_with_servers() {
        let doc = json!({
            "asyncapi": "3.0.0",
            "info": {
                "title": "Test API",
                "version": "1.0.0"
            },
            "servers": {
                "production": {
                    "host": "broker.example.com",
                    "protocol": "amqp"
                }
            }
        });

        assert!(validate(&doc).is_ok());
    }

    #[test]
    fn validate_with_channels() {
        let doc = json!({
            "asyncapi": "3.0.0",
            "info": {
                "title": "Test API",
                "version": "1.0.0"
            },
            "channels": {
                "userSignedUp": {
                    "address": "user.signedup",
                    "messages": {
                        "userSignedUp": {
                            "$ref": "#/components/messages/userSignedUp"
                        }
                    }
                }
            },
            "components": {
                "messages": {
                    "userSignedUp": {
                        "payload": {
                            "type": "object",
                            "properties": {
                                "username": {"type": "string"},
                                "email": {"type": "string"}
                            }
                        }
                    }
                }
            }
        });

        assert!(validate(&doc).is_ok());
    }

    #[test]
    fn validate_with_extension_field() {
        let doc = json!({
            "asyncapi": "3.0.0",
            "info": {
                "title": "Test API",
                "version": "1.0.0"
            },
            "x-custom-field": "custom value"
        });

        assert!(validate(&doc).is_ok());
    }

    #[test]
    fn validate_with_invalid_additional_property() {
        let doc = json!({
            "asyncapi": "3.0.0",
            "info": {
                "title": "Test API",
                "version": "1.0.0"
            },
            "invalidField": "not allowed"
        });

        assert!(matches!(
            validate(&doc),
            Err(SpecError::ValidationFailed(_))
        ));
    }

    #[test]
    fn validate_empty_document() {
        let doc = json!({});

        assert!(matches!(validate(&doc), Err(SpecError::InvalidSpec(_))));
    }

    #[test]
    fn validate_with_components() {
        let doc = json!({
            "asyncapi": "3.0.0",
            "info": {
                "title": "Test API",
                "version": "1.0.0"
            },
            "components": {
                "schemas": {
                    "User": {
                        "type": "object",
                        "properties": {
                            "id": {"type": "string"},
                            "name": {"type": "string"}
                        }
                    }
                },
                "messages": {
                    "UserCreated": {
                        "payload": {
                            "$ref": "#/components/schemas/User"
                        }
                    }
                }
            }
        });

        assert!(validate(&doc).is_ok());
    }

    #[test]
    fn validate_with_operations() {
        let doc = json!({
            "asyncapi": "3.0.0",
            "info": {
                "title": "Test API",
                "version": "1.0.0"
            },
            "operations": {
                "onUserSignedUp": {
                    "action": "send",
                    "channel": {
                        "$ref": "#/channels/userSignedUp"
                    }
                }
            },
            "channels": {
                "userSignedUp": {
                    "address": "user.signedup"
                }
            }
        });

        assert!(validate(&doc).is_ok());
    }
}
