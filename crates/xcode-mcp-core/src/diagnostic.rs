use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
    Note,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FixIt {
    pub message: String,
    pub range: Option<FixItRange>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FixItRange {
    pub start_line: u32,
    pub start_col: u32,
    pub end_line: u32,
    pub end_col: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Diagnostic {
    pub file: Option<String>,
    pub line: Option<u32>,
    pub column: Option<u32>,
    pub severity: Severity,
    pub message: String,
    pub category: Option<String>,
    pub fix_its: Option<Vec<FixIt>>,
    pub source: DiagnosticSource,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticSource {
    Xcresult,
    Stderr,
}

#[derive(Debug, Clone, Default)]
pub struct ParseResult {
    pub diagnostics: Vec<Diagnostic>,
    pub parse_warnings: Vec<String>,
}
