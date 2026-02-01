//
// SPDX-License-Identifier: Apache-2.0

//! YAML/JSON AST parsing for AsyncAPI specs.

use thiserror::Error;

/// Error types for AST parsing.
#[derive(Debug, Error)]
pub enum SpecError {
    #[error("YAML/JSON parsing error: {0}")]
    ParseError(#[from] serde_yaml::Error),
    #[error("File I/O error: {0}")]
    IoError(#[from] std::io::Error),
}

/// Parse AsyncAPI spec string into YAML AST.
///
/// Returns a generic `serde_json::Value` (the AST) rather than
/// typed document structure. This allows for post-processing
/// and inspection before conversion to concrete types.
///
/// # Arguments
///
/// * `yaml` - YAML or JSON string to parse
///
/// # Returns
///
/// * `Ok(serde_json::Value)` - Parsed AST structure
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
pub fn parse_yaml_ast(yaml: &str) -> Result<serde_json::Value, SpecError> {
    Ok(serde_yaml::from_str(yaml)?)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    const SIMPLE_EXAMPLE: &str =
        include_str!("../../../submodules/asyncapi-spec/examples/simple-asyncapi.yml");
    const ANYOF_EXAMPLE: &str =
        include_str!("../../../submodules/asyncapi-spec/examples/anyof-asyncapi.yml");
    const STREETLIGHTS_KAFKA: &str =
        include_str!("../../../submodules/asyncapi-spec/examples/streetlights-kafka-asyncapi.yml");
    const RPC_SERVER: &str =
        include_str!("../../../submodules/asyncapi-spec/examples/rpc-server-asyncapi.yml");
    const INVALID_YAML: &str = "asyncapi: 3.1.0\n  invalid indentation: true";

    #[test]
    fn test_parse_simple_example() {
        let ast = parse_yaml_ast(SIMPLE_EXAMPLE).unwrap();

        assert!(ast.is_object());
        let map = ast.as_object().unwrap();

        assert_eq!(
            map.get("asyncapi").unwrap(),
            &serde_json::Value::String("3.1.0".to_string())
        );
    }

    #[test]
    fn test_parse_with_components() {
        let ast = parse_yaml_ast(ANYOF_EXAMPLE).unwrap();
        let map = ast.as_object().unwrap();

        let components = map.get("components").unwrap();
        let components_map = components.as_object().unwrap();

        assert!(components_map.contains_key("messages"));
    }

    #[test]
    fn test_parse_streetlights_kafka() {
        let ast = parse_yaml_ast(STREETLIGHTS_KAFKA).unwrap();
        let map = ast.as_object().unwrap();

        assert_eq!(
            map.get("asyncapi").unwrap(),
            &serde_json::Value::String("3.1.0".to_string())
        );

        assert!(map.contains_key("channels"));
    }

    #[test]
    fn test_parse_rpc_server() {
        let ast = parse_yaml_ast(RPC_SERVER).unwrap();
        let map = ast.as_object().unwrap();

        assert!(map.contains_key("id"));
        assert!(map.contains_key("channels"));
    }

    #[test]
    fn test_parse_invalid_yaml() {
        let result = parse_yaml_ast(INVALID_YAML);
        assert!(result.is_err());
    }
}
