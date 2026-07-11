use crate::rules::ValidationIssue;
use serde_json::Value;

pub fn check_parameter_location(document: &Value) -> Vec<ValidationIssue> {
    let mut issues = Vec::new();

    let Some(channels) = document.get("channels").and_then(|v| v.as_object()) else {
        return issues;
    };

    for (channel_key, channel_def) in channels {
        let Some(channel_obj) = channel_def.as_object() else {
            continue;
        };

        let Some(parameters) = channel_obj.get("parameters").and_then(|v| v.as_object()) else {
            continue;
        };

        let messages = match channel_obj.get("messages").and_then(|v| v.as_object()) {
            Some(m) => m,
            None => continue,
        };

        for (param_name, param_def) in parameters {
            let Some(location) = param_def.get("location").and_then(|v| v.as_str()) else {
                continue;
            };

            let Some(path) = location.strip_prefix("$message.payload#/") else {
                continue;
            };

            for (msg_name, msg_def) in messages {
                if !is_json_message(msg_def) {
                    continue;
                }

                let Some(payload) = msg_def.get("payload") else {
                    issues.push(ValidationIssue {
                        message: format!(
                            "parameter '{param_name}' location path '{path}' not found \
                             in message '{msg_name}': message has no payload"
                        ),
                        path: format!("$.channels.{channel_key}.parameters.{param_name}.location"),
                        rule: "parameter-location-exists-in-messages",
                    });
                    continue;
                };

                if !path_exists_in_schema(payload, path) {
                    issues.push(ValidationIssue {
                        message: format!(
                            "parameter '{param_name}' location path '{path}' not found \
                             in message '{msg_name}'"
                        ),
                        path: format!("$.channels.{channel_key}.parameters.{param_name}.location"),
                        rule: "parameter-location-exists-in-messages",
                    });
                }
            }
        }
    }

    issues
}

fn is_json_message(msg_def: &Value) -> bool {
    let content_type = msg_def.get("contentType").and_then(|v| v.as_str());
    match content_type {
        None | Some("application/json") => true,
        Some(other) => other.starts_with("application/") && other.ends_with("+json"),
    }
}

fn path_exists_in_schema(schema: &Value, path: &str) -> bool {
    if path.is_empty() {
        return true;
    }

    let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    let mut current = schema;

    for part in parts {
        let Some(obj) = current.as_object() else {
            return false;
        };

        if obj.get("type").and_then(|v| v.as_str()) != Some("object") {
            return false;
        }

        let Some(props) = obj.get("properties") else {
            return false;
        };

        match props.get(part) {
            Some(next) => current = next,
            None => return false,
        }
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn valid_path_in_all_messages() {
        let doc = json!({
            "channels": {
                "events": {
                    "address": "user.{userId}.events",
                    "parameters": {
                        "userId": {
                            "location": "$message.payload#/userId"
                        }
                    },
                    "messages": {
                        "created": {
                            "contentType": "application/json",
                            "payload": {
                                "type": "object",
                                "properties": {
                                    "userId": { "type": "string" }
                                }
                            }
                        },
                        "deleted": {
                            "contentType": "application/json",
                            "payload": {
                                "type": "object",
                                "properties": {
                                    "userId": { "type": "string" }
                                }
                            }
                        }
                    }
                }
            }
        });

        let issues = check_parameter_location(&doc);
        assert!(issues.is_empty(), "got: {issues:?}");
    }

    #[test]
    fn missing_path_in_one_message() {
        let doc = json!({
            "channels": {
                "events": {
                    "address": "user.{userId}.events",
                    "parameters": {
                        "userId": {
                            "location": "$message.payload#/userId"
                        }
                    },
                    "messages": {
                        "created": {
                            "contentType": "application/json",
                            "payload": {
                                "type": "object",
                                "properties": {
                                    "userId": { "type": "string" }
                                }
                            }
                        },
                        "deleted": {
                            "contentType": "application/json",
                            "payload": {
                                "type": "object",
                                "properties": {
                                    "id": { "type": "string" }
                                }
                            }
                        }
                    }
                }
            }
        });

        let issues = check_parameter_location(&doc);
        assert_eq!(issues.len(), 1);
        assert!(issues[0].message.contains("deleted"));
    }

    #[test]
    fn skips_non_json_messages() {
        let doc = json!({
            "channels": {
                "events": {
                    "address": "user.{userId}.events",
                    "parameters": {
                        "userId": {
                            "location": "$message.payload#/userId"
                        }
                    },
                    "messages": {
                        "protobuf": {
                            "contentType": "application/x-protobuf",
                            "payload": {
                                "schemaFormat": "application/vnd.google.protobuf;version=3",
                                "schema": "message User { string id = 1; }"
                            }
                        },
                        "json": {
                            "contentType": "application/json",
                            "payload": {
                                "type": "object",
                                "properties": {
                                    "userId": { "type": "string" }
                                }
                            }
                        }
                    }
                }
            }
        });

        let issues = check_parameter_location(&doc);
        assert!(issues.is_empty(), "protobuf should be skipped: {issues:?}");
    }

    #[test]
    fn nested_path_resolves() {
        let doc = json!({
            "channels": {
                "events": {
                    "address": "user.{region}.events",
                    "parameters": {
                        "region": {
                            "location": "$message.payload#/user/address/region"
                        }
                    },
                    "messages": {
                        "signup": {
                            "contentType": "application/json",
                            "payload": {
                                "type": "object",
                                "properties": {
                                    "user": {
                                        "type": "object",
                                        "properties": {
                                            "address": {
                                                "type": "object",
                                                "properties": {
                                                    "region": { "type": "string" }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        });

        let issues = check_parameter_location(&doc);
        assert!(issues.is_empty(), "got: {issues:?}");
    }
}
