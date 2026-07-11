use serde_json::Value;

pub mod core_rules;

/// A validation issue found by a rule check.
#[derive(Debug, Clone)]
pub struct ValidationIssue {
    pub message: String,
    pub path: String,
    pub rule: &'static str,
}

pub fn validate_rules(document: &Value) -> Vec<ValidationIssue> {
    crate::rules::core_rules::check_parameter_location(document)
}
