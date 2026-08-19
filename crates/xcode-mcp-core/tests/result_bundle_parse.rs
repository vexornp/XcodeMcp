use std::fs;
use xcode_mcp_core::diagnostic::*;
use xcode_mcp_core::result_bundle::*;

fn fixture(name: &str) -> String {
    fs::read_to_string(format!("tests/fixtures/result_bundle/{name}")).unwrap()
}

#[test]
fn parses_typical_with_errors_warnings_notes() {
    let result = parse_build_results(&fixture("typical.json")).unwrap();
    assert_eq!(
        result
            .diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Error)
            .count(),
        2
    );
    assert_eq!(
        result
            .diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Warning)
            .count(),
        3
    );
    assert_eq!(
        result
            .diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Note)
            .count(),
        1
    );
}

#[test]
fn extracts_file_line_column_from_url() {
    let result = parse_build_results(&fixture("typical.json")).unwrap();
    let first = &result.diagnostics[0];
    assert_eq!(
        first.file.as_deref(),
        Some("/tmp/App/Sources/App/main.swift")
    );
    assert_eq!(first.line, Some(10));
    assert_eq!(first.column, Some(5));
}

#[test]
fn extracts_fix_its() {
    let result = parse_build_results(&fixture("typical.json")).unwrap();
    let fix_its = result.diagnostics[0].fix_its.as_ref().unwrap();
    assert_eq!(fix_its.len(), 1);
    assert_eq!(fix_its[0].message, "Replace 'foo' with 'bar'");
    assert_eq!(fix_its[0].range.as_ref().unwrap().start_line, 10);
}

#[test]
fn handles_missing_location() {
    let result = parse_build_results(&fixture("typical.json")).unwrap();
    let no_loc = result
        .diagnostics
        .iter()
        .find(|d| d.message == "unused import")
        .unwrap();
    assert!(no_loc.file.is_none());
    assert!(no_loc.line.is_none());
}

#[test]
fn no_issues_returns_empty() {
    let result = parse_build_results(&fixture("no_issues.json")).unwrap();
    assert!(result.diagnostics.is_empty());
}

#[test]
fn malformed_url_still_emits_diagnostic() {
    let result = parse_build_results(&fixture("malformed_url.json")).unwrap();
    assert_eq!(result.diagnostics.len(), 1);
    assert!(result.diagnostics[0].file.is_none());
}

#[test]
fn unknown_issue_type_becomes_note_with_warning() {
    let result = parse_build_results(&fixture("unknown_issue_type.json")).unwrap();
    assert_eq!(result.diagnostics[0].severity, Severity::Note);
    assert!(!result.parse_warnings.is_empty());
}

#[test]
fn missing_actions_returns_error() {
    assert!(parse_build_results(&fixture("missing_actions.json")).is_err());
}

#[test]
fn all_diagnostics_tagged_xcresult_source() {
    let result = parse_build_results(&fixture("typical.json")).unwrap();
    for d in &result.diagnostics {
        assert_eq!(d.source, DiagnosticSource::Xcresult);
    }
}
