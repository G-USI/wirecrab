use super::*;
use std::io::Write;
use tempfile::NamedTempFile;

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
