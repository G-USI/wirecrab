use serde_json::Value;

pub mod core_rules;

pub trait Rule: Send + Sync {
    fn name(&self) -> &'static str;
    fn category(&self) -> RuleCategory;
    fn check(&self, document: &Value) -> Vec<ValidationIssue>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RuleCategory {
    Core,
    Protocol,
}

inventory::collect!(&'static dyn Rule);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
    Info,
}

#[derive(Debug, Clone)]
pub struct ValidationIssue {
    pub severity: Severity,
    pub message: String,
    pub path: String,
    pub rule: &'static str,
}

impl ValidationIssue {
    pub fn error(message: impl Into<String>, path: impl Into<String>, rule: &'static str) -> Self {
        Self {
            severity: Severity::Error,
            message: message.into(),
            path: path.into(),
            rule,
        }
    }

    pub fn warning(
        message: impl Into<String>,
        path: impl Into<String>,
        rule: &'static str,
    ) -> Self {
        Self {
            severity: Severity::Warning,
            message: message.into(),
            path: path.into(),
            rule,
        }
    }
}

pub fn validate_rules(document: &Value) -> Vec<ValidationIssue> {
    inventory::iter::<&'static dyn Rule>()
        .flat_map(|rule| rule.check(document))
        .collect()
}
