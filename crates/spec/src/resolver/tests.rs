use super::*;
use std::io::Write;
use tempfile::NamedTempFile;

fn create_test_yaml_in(dir: &std::path::Path, name: &str, content: &str) -> std::path::PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, content).unwrap();
    path
}

#[test]
fn docaddress_valid_root() {
    let addr = DocAddress::try_from("#/").unwrap();
    assert_eq!(addr.0, Vec::<String>::new());
}

#[test]
fn docaddress_valid_single_segment() {
    let addr = DocAddress::try_from("#/components").unwrap();
    assert_eq!(addr.0, vec!["components".to_string()]);
}

#[test]
fn docaddress_valid_multiple_segments() {
    let addr = DocAddress::try_from("#/components/schemas/User").unwrap();
    assert_eq!(
        addr.0,
        vec![
            "components".to_string(),
            "schemas".to_string(),
            "User".to_string()
        ]
    );
}

#[test]
fn docaddress_valid_with_slash_escaped() {
    let addr = DocAddress::try_from("#/a~1b").unwrap();
    assert_eq!(addr.0, vec!["a/b".to_string()]);
}

#[test]
fn docaddress_valid_with_tilde_escaped() {
    let addr = DocAddress::try_from("#/a~0b").unwrap();
    assert_eq!(addr.0, vec!["a~b".to_string()]);
}

#[test]
fn docaddress_valid_mixed_escapes() {
    let addr = DocAddress::try_from("#/a~1b~0c").unwrap();
    assert_eq!(addr.0, vec!["a/b~c".to_string()]);
}

#[test]
fn docaddress_invalid_missing_hash_slash() {
    assert!(DocAddress::try_from("").is_err());
    assert!(DocAddress::try_from("#").is_err());
    assert!(DocAddress::try_from("components").is_err());
    assert!(DocAddress::try_from("/components").is_err());
}

#[test]
fn docaddress_invalid_empty_segment() {
    assert!(DocAddress::try_from("#//").is_err());
    assert!(DocAddress::try_from("#/a//b").is_err());
    assert!(DocAddress::try_from("#/a/").is_err());
}

#[test]
fn docaddress_invalid_tilde_alone() {
    assert!(DocAddress::try_from("#/~").is_err());
    assert!(DocAddress::try_from("#/~2").is_err());
    assert!(DocAddress::try_from("#/a~b").is_err());
}

#[test]
fn docaddress_iter_empty() {
    let addr = DocAddress::try_from("#/").unwrap();
    let iter = addr.iter();
    assert_eq!(iter.count(), 0);
}

#[test]
fn docaddress_iter_single() {
    let addr = DocAddress::try_from("#/components").unwrap();
    let iter = addr.iter();
    let parts: Vec<&str> = iter.collect();
    assert_eq!(parts, vec!["components"]);
}

#[test]
fn docaddress_iter_multiple() {
    let addr = DocAddress::try_from("#/components/schemas/User").unwrap();
    let iter = addr.iter();
    let parts: Vec<&str> = iter.collect();
    assert_eq!(parts, vec!["components", "schemas", "User"]);
}

fn create_test_yaml(content: &str) -> NamedTempFile {
    let mut file = NamedTempFile::new().unwrap();
    file.write_all(content.as_bytes()).unwrap();
    file
}

#[test]
fn refresolver_load_simple_yaml() {
    let yaml = r#"
asyncapi: 3.0.0
info:
  title: Test API
  version: 1.0.0
"#;
    let file = create_test_yaml(yaml);

    let resolver = RefResolver::default();
    let doc_ref = DocumentRef {
        location: Shared::new(DocLocation::File(file.path().to_path_buf())),
        addr: Shared::new(DocAddress::try_from("#/").unwrap()),
    };

    let result = resolver.resolve(doc_ref);
    assert!(result.is_ok());

    let doc = result.unwrap();
    let doc_value = doc.as_ref();
    assert!(doc_value.is_object());
    assert_eq!(doc_value["asyncapi"], "3.0.0");
    assert_eq!(doc_value["info"]["title"], "Test API");
    assert_eq!(doc_value["info"]["version"], "1.0.0");
}

#[test]
fn refresolver_circular_dependency_detection() {
    let resolver = RefResolver::default();

    // Create a file with circular dependency
    let yaml_content = r#"
components:
  schemas:
    User:
      type: object
      properties:
        friend:
          $ref: '#/components/schemas/Admin'
    Admin:
      type: object
      properties:
        manager:
          $ref: '#/components/schemas/User'
"#;
    let file_path = "circular-test.yaml";
    std::fs::write(file_path, yaml_content).unwrap();

    // Get the root document first
    let root_doc = resolver.resolve_ref(file_path, "#/").unwrap();
    let root_value = root_doc.as_ref().clone();

    // Then try to resolve recursively (this should detect circular dependency)
    let result = resolver.resolve_recursive(&root_value, file_path);

    // Clean up
    std::fs::remove_file(file_path).ok();

    assert!(result.is_err());
    let error = result.unwrap_err();
    assert!(error.to_string().contains("Circular reference detected"));
}

#[test]
fn refresolver_traverse_nested_mapping() {
    let yaml = r#"
asyncapi: 3.0.0
info:
  title: Test API
  version: 1.0.0
components:
  schemas:
    User:
      type: object
      properties:
        name:
          type: string
"#;
    let file = create_test_yaml(yaml);

    let resolver = RefResolver::default();
    let doc_ref = DocumentRef {
        location: Shared::new(DocLocation::File(file.path().to_path_buf())),
        addr: Shared::new(
            DocAddress::try_from("#/components/schemas/User/properties/name").unwrap(),
        ),
    };

    let result = resolver.resolve(doc_ref);
    assert!(result.is_ok());

    let doc = result.unwrap();
    let doc_value = doc.as_ref();
    assert_eq!(
        doc_value.get("type"),
        Some(&Value::String("string".to_string()))
    );
}

#[test]
fn refresolver_traverse_sequence() {
    let yaml = r#"
servers:
  - url: amqp://localhost
    protocol: amqp
  - url: http://localhost:8080
    protocol: http
"#;
    let file = create_test_yaml(yaml);

    let resolver = RefResolver::default();
    let doc_ref = DocumentRef {
        location: Shared::new(DocLocation::File(file.path().to_path_buf())),
        addr: Shared::new(DocAddress::try_from("#/servers/0").unwrap()),
    };

    let result = resolver.resolve(doc_ref);
    assert!(result.is_ok());

    let doc = result.unwrap();
    let doc_value = doc.as_ref();
    assert_eq!(
        doc_value.get("protocol"),
        Some(&Value::String("amqp".to_string()))
    );
}

#[test]
fn refresolver_traverse_sequence_second_item() {
    let yaml = r#"
servers:
  - url: amqp://localhost
    protocol: amqp
  - url: http://localhost:8080
    protocol: http
"#;
    let file = create_test_yaml(yaml);

    let resolver = RefResolver::default();
    let doc_ref = DocumentRef {
        location: Shared::new(DocLocation::File(file.path().to_path_buf())),
        addr: Shared::new(DocAddress::try_from("#/servers/1").unwrap()),
    };

    let result = resolver.resolve(doc_ref);
    assert!(result.is_ok());

    let doc = result.unwrap();
    let doc_value = doc.as_ref();
    assert_eq!(
        doc_value.get("protocol"),
        Some(&Value::String("http".to_string()))
    );
}

#[test]
fn refresolver_error_missing_key() {
    let yaml = r#"
info:
  title: Test API
"#;
    let file = create_test_yaml(yaml);

    let resolver = RefResolver::default();
    let doc_ref = DocumentRef {
        location: Shared::new(DocLocation::File(file.path().to_path_buf())),
        addr: Shared::new(DocAddress::try_from("#/missing/key").unwrap()),
    };

    let result = resolver.resolve(doc_ref);
    assert!(result.is_err());
}

#[test]
fn refresolver_error_index_out_of_bounds() {
    let yaml = r#"
servers:
  - url: amqp://localhost
"#;
    let file = create_test_yaml(yaml);

    let resolver = RefResolver::default();
    let doc_ref = DocumentRef {
        location: Shared::new(DocLocation::File(file.path().to_path_buf())),
        addr: Shared::new(DocAddress::try_from("#/servers/5").unwrap()),
    };

    let result = resolver.resolve(doc_ref);
    assert!(result.is_err());
}

#[test]
fn refresolver_error_invalid_index() {
    let yaml = r#"
servers:
  - url: amqp://localhost
"#;
    let file = create_test_yaml(yaml);

    let resolver = RefResolver::default();
    let doc_ref = DocumentRef {
        location: Shared::new(DocLocation::File(file.path().to_path_buf())),
        addr: Shared::new(DocAddress::try_from("#/servers/not_a_number").unwrap()),
    };

    let result = resolver.resolve(doc_ref);
    assert!(result.is_err());
}

#[test]
fn refresolver_subtree_cache_hit() {
    let yaml = r#"
components:
  schemas:
    User:
      type: object
    Admin:
      type: object
"#;
    let file = create_test_yaml(yaml);

    let resolver = RefResolver::default();
    let location = DocLocation::File(file.path().to_path_buf());
    let addr = DocAddress::try_from("#/components/schemas/User").unwrap();

    let doc_ref = DocumentRef {
        location: Shared::new(location.clone()),
        addr: Shared::new(addr.clone()),
    };
    let result1 = resolver.resolve(doc_ref.clone());
    assert!(result1.is_ok());

    let result2 = resolver.resolve(doc_ref.clone());
    assert!(result2.is_ok());

    assert_eq!(result1.unwrap().as_ref(), result2.unwrap().as_ref());
}

#[test]
fn refresolver_doc_cache_reuse() {
    let yaml = r#"
components:
  schemas:
    User:
      type: object
    Admin:
      type: string
"#;
    let file = create_test_yaml(yaml);

    let resolver = RefResolver::default();
    let location = DocLocation::File(file.path().to_path_buf());

    let doc_ref1 = DocumentRef {
        location: Shared::new(location.clone()),
        addr: Shared::new(DocAddress::try_from("#/components/schemas/User").unwrap()),
    };
    let result1 = resolver.resolve(doc_ref1);
    assert!(result1.is_ok());

    let doc_ref2 = DocumentRef {
        location: Shared::new(location.clone()),
        addr: Shared::new(DocAddress::try_from("#/components/schemas/Admin").unwrap()),
    };
    let result2 = resolver.resolve(doc_ref2);
    assert!(result2.is_ok());

    assert_ne!(result1.unwrap().as_ref(), result2.unwrap().as_ref());
}

#[test]
fn refresolver_caches_different_subtrees_separately() {
    let yaml = r#"
components:
  schemas:
    User:
      type: object
    Admin:
      type: object
"#;
    let file = create_test_yaml(yaml);

    let resolver = RefResolver::default();
    let location = DocLocation::File(file.path().to_path_buf());

    let doc_ref1 = DocumentRef {
        location: Shared::new(location.clone()),
        addr: Shared::new(DocAddress::try_from("#/components/schemas/User").unwrap()),
    };
    let result1 = resolver.resolve(doc_ref1);
    assert!(result1.is_ok());

    let doc_ref2 = DocumentRef {
        location: Shared::new(location.clone()),
        addr: Shared::new(DocAddress::try_from("#/components/schemas/Admin").unwrap()),
    };
    let result2 = resolver.resolve(doc_ref2);
    assert!(result2.is_ok());

    let subtrees = resolver.subtrees.borrow();
    assert_eq!(subtrees.len(), 2);

    let keys: Vec<_> = subtrees.keys().collect();
    assert_ne!(keys[0], keys[1]);
}

#[test]
fn refresolver_with_slash_escaped_in_path() {
    let yaml = r#"
a/b:
  type: object
  properties:
    name:
      type: string
"#;
    let file = create_test_yaml(yaml);

    let resolver = RefResolver::default();
    let doc_ref = DocumentRef {
        location: Shared::new(DocLocation::File(file.path().to_path_buf())),
        addr: Shared::new(DocAddress::try_from("#/a~1b/type").unwrap()),
    };

    let result = resolver.resolve(doc_ref);
    assert!(result.is_ok());

    let doc = result.unwrap();
    let doc_value = doc.as_ref();
    assert_eq!(doc_value, &Value::String("object".to_string()));
}

#[test]
fn refresolver_diamond_reference_resolves() {
    let yaml = r#"
channels:
  userSignedup:
    address: user/signedup
    messages:
      UserSignedUp:
        $ref: '#/components/messages/UserSignedUp'
operations:
  sendUserSignedup:
    action: send
    channel:
      $ref: '#/channels/userSignedup'
    messages:
      - $ref: '#/components/messages/UserSignedUp'
components:
  messages:
    UserSignedUp:
      payload:
        type: object
        properties:
          email:
            type: string
"#;
    let file = create_test_yaml(yaml);

    let resolver = RefResolver::default();
    let root = resolver
        .resolve_ref(file.path().to_str().unwrap(), "#/")
        .unwrap();
    let root_value = root.as_ref().clone();

    let result = resolver.resolve_recursive(&root_value, file.path().to_str().unwrap());

    assert!(
        result.is_ok(),
        "diamond reference should not be flagged circular: {:?}",
        result.err()
    );

    let resolved = result.unwrap();
    let op_msg_payload = &resolved["operations"]["sendUserSignedup"]["messages"][0]["payload"];
    assert_eq!(op_msg_payload["properties"]["email"]["type"], "string");
    let chan_msg_payload =
        &resolved["channels"]["userSignedup"]["messages"]["UserSignedUp"]["payload"];
    assert_eq!(chan_msg_payload["properties"]["email"]["type"], "string");
}

#[test]
fn refresolver_cycle_direct_self_reference_errors() {
    let yaml = r#"
components:
  schemas:
    Node:
      type: object
      properties:
        parent:
          $ref: '#/components/schemas/Node'
"#;
    let file = create_test_yaml(yaml);

    let resolver = RefResolver::default();
    let root = resolver
        .resolve_ref(file.path().to_str().unwrap(), "#/")
        .unwrap();
    let root_value = root.as_ref().clone();

    let result = resolver.resolve_recursive(&root_value, file.path().to_str().unwrap());

    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("Circular reference detected"), "got: {err}");
}

#[test]
fn refresolver_with_tilde_escaped_in_path() {
    let yaml = r#"
a~b:
  type: object
  properties:
    name:
      type: string
"#;
    let file = create_test_yaml(yaml);

    let resolver = RefResolver::default();
    let doc_ref = DocumentRef {
        location: Shared::new(DocLocation::File(file.path().to_path_buf())),
        addr: Shared::new(DocAddress::try_from("#/a~0b/type").unwrap()),
    };

    let result = resolver.resolve(doc_ref);
    assert!(result.is_ok());

    let doc = result.unwrap();
    let doc_value = doc.as_ref();
    assert_eq!(doc_value, &Value::String("object".to_string()));
}

#[test]
fn refresolver_cross_file_relative_ref_from_different_cwd() {
    let dir = tempfile::tempdir().unwrap();

    create_test_yaml_in(
        dir.path(),
        "schemas.yaml",
        r#"
components:
  schemas:
    UserId:
      type: string
      format: uuid
    User:
      type: object
      properties:
        id:
          $ref: '#/components/schemas/UserId'
        email:
          type: string
      required: [id, email]
"#,
    );

    create_test_yaml_in(
        dir.path(),
        "messages.yaml",
        r#"
components:
  messages:
    userSignedUp:
      contentType: application/json
      payload:
        $ref: 'schemas.yaml#/components/schemas/User'
"#,
    );

    let spec = create_test_yaml_in(
        dir.path(),
        "spec.yaml",
        r#"
asyncapi: 3.1.0
info:
  title: Cross-file Test
  version: 1.0.0
channels:
  events:
    address: events
    messages:
      userSignedUp:
        $ref: 'messages.yaml#/components/messages/userSignedUp'
"#,
    );

    let original_cwd = std::env::current_dir().unwrap();
    std::env::set_current_dir("/tmp").unwrap();

    let resolver = RefResolver::default();
    let root = resolver.resolve_ref(spec.to_str().unwrap(), "#/").unwrap();
    let root_value = root.as_ref().clone();
    let result = resolver.resolve_recursive(&root_value, spec.to_str().unwrap());

    std::env::set_current_dir(original_cwd).unwrap();

    assert!(
        result.is_ok(),
        "cross-file relative refs should resolve from any CWD: {:?}",
        result.err()
    );

    let resolved = result.unwrap();
    let payload = &resolved["channels"]["events"]["messages"]["userSignedUp"]["payload"];
    assert_eq!(payload["properties"]["email"]["type"], "string");
    assert_eq!(payload["properties"]["id"]["format"], "uuid");
}
