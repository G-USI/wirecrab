//
// SPDX-License-Identifier: Apache-2.0

//! YAML/JSON AST parsing for AsyncAPI specs.

use thiserror::Error;

/// Error types for AST parsing.
#[derive(Debug, Error)]
pub enum SpecError {
    #[error("YAML/JSON parsing error: {0}")]
    ParseError(#[from] serde_yaml::Error),
}

/// Parse AsyncAPI spec string into YAML AST.
///
/// Returns a generic `serde_yaml::Value` (the AST) rather than
/// typed document structure. This allows for post-processing
/// and inspection before conversion to concrete types.
///
/// # Arguments
///
/// * `yaml` - YAML or JSON string to parse
///
/// # Returns
///
/// * `Ok(serde_yaml::Value)` - Parsed AST structure
/// * `Err(SpecError)` - Parsing error
///
/// # Examples
///
/// ```no_run
/// use wirecrab_spec::ast::parse_yaml_ast;
///
/// let yaml = "asyncapi: 3.1.0\ninfo:\n  title: Test\n  version: 1.0.0";
/// let ast = parse_yaml_ast(yaml).unwrap();
/// ```
pub fn parse_yaml_ast(yaml: &str) -> Result<serde_yaml::Value, SpecError> {
    Ok(serde_yaml::from_str(yaml)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_yaml() {
        let yaml = r#"
asyncapi: 3.1.0
info:
  title: Test API
  version: 1.0.0
"#;

        let ast = parse_yaml_ast(yaml).unwrap();

        assert!(ast.is_mapping());
        let map = ast.as_mapping().unwrap();

        assert_eq!(
            map.get(&serde_yaml::Value::String("asyncapi".to_string()))
                .unwrap(),
            &serde_yaml::Value::String("3.1.0".to_string())
        );
    }

    #[test]
    fn test_parse_with_components() {
        let yaml = r#"
asyncapi: 3.1.0
info:
  title: Test API
  version: 1.0.0
components:
  messages:
    UserSignedUp:
      name: UserSignedUp
      title: User Signed Up
"#;

        let ast = parse_yaml_ast(yaml).unwrap();
        let map = ast.as_mapping().unwrap();

        let components = map
            .get(&serde_yaml::Value::String("components".to_string()))
            .unwrap();
        let components_map = components.as_mapping().unwrap();

        assert!(components_map.contains_key(&serde_yaml::Value::String("messages".to_string())));
    }

    #[test]
    fn test_parse_invalid_yaml() {
        // Invalid YAML: duplicate key
        let invalid_yaml = "asyncapi: 3.1.0\nasyncapi: 2.0.0";

        let result = parse_yaml_ast(invalid_yaml);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_with_extensions() {
        let yaml = r#"
asyncapi: 3.1.0
info:
  title: Test API
  version: 1.0.0
  x-custom-field: custom value
"#;

        let ast = parse_yaml_ast(yaml).unwrap();
        let map = ast.as_mapping().unwrap();

        let info = map
            .get(&serde_yaml::Value::String("info".to_string()))
            .unwrap();
        let info_map = info.as_mapping().unwrap();

        assert!(info_map.contains_key(&serde_yaml::Value::String("x-custom-field".to_string())));
    }

    #[test]
    fn test_parse_real_example() {
        let yaml = r#"
asyncapi: 3.1.0
info:
  title: Account Service
  version: 1.0.0
  description: This service is in charge of processing user signups
channels:
  userSignedup:
    address: user/signedup
    messages:
      UserSignedUp:
        $ref: '#/components/messages/UserSignedUp'
"#;

        let ast = parse_yaml_ast(yaml).unwrap();
        let map = ast.as_mapping().unwrap();

        // Verify asyncapi version
        assert_eq!(
            map.get(&serde_yaml::Value::String("asyncapi".to_string()))
                .unwrap(),
            &serde_yaml::Value::String("3.1.0".to_string())
        );

        // Verify channels exist
        assert!(map.contains_key(&serde_yaml::Value::String("channels".to_string())));

        // Verify info structure
        let info = map
            .get(&serde_yaml::Value::String("info".to_string()))
            .unwrap();
        let info_map = info.as_mapping().unwrap();
        assert!(info_map.contains_key(&serde_yaml::Value::String("title".to_string())));
    }
}
