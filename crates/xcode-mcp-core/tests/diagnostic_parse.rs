use std::fs;
use xcode_mcp_core::diagnostic::*;

fn fixture(name: &str) -> String {
    fs::read_to_string(format!("tests/fixtures/stderr/{name}")).unwrap()
}

#[test]
fn parses_compiler_error_with_column() {
    let result = parse_stderr(&fixture("compiler_error.txt"));
    let errors: Vec<_> = result
        .diagnostics
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .collect();
    assert_eq!(errors.len(), 1);
    assert_eq!(
        errors[0].file.as_deref(),
        Some("/tmp/App/Sources/App/main.swift")
    );
    assert_eq!(errors[0].line, Some(10));
    assert_eq!(errors[0].column, Some(5));
    assert!(errors[0].message.contains("undeclared identifier 'foo'"));
    assert_eq!(errors[0].source, DiagnosticSource::Stderr);
}

#[test]
fn parses_note_after_error() {
    let result = parse_stderr(&fixture("compiler_error.txt"));
    let notes: Vec<_> = result
        .diagnostics
        .iter()
        .filter(|d| d.severity == Severity::Note)
        .collect();
    assert_eq!(notes.len(), 1);
    assert!(notes[0].message.contains("did you mean"));
}

#[test]
fn parses_linker_error_without_column() {
    let result = parse_stderr(&fixture("linker_error.txt"));
    let errors: Vec<_> = result
        .diagnostics
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .collect();
    assert_eq!(errors.len(), 1);
    assert!(errors[0].message.contains("symbol(s) not found"));
    assert_eq!(errors[0].category.as_deref(), Some("Linker"));
}

#[test]
fn parses_linker_warning() {
    let result = parse_stderr(&fixture("linker_error.txt"));
    let warnings: Vec<_> = result
        .diagnostics
        .iter()
        .filter(|d| d.severity == Severity::Warning)
        .collect();
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].message.contains("ignoring duplicate library"));
}

#[test]
fn parses_mixed_errors_warnings_notes() {
    let result = parse_stderr(&fixture("mixed.txt"));
    assert_eq!(
        result
            .diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Error)
            .count(),
        1
    );
    assert_eq!(
        result
            .diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Warning)
            .count(),
        1
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
fn folds_multiline_messages() {
    let result = parse_stderr(&fixture("multiline.txt"));
    let errors: Vec<_> = result
        .diagnostics
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .collect();
    assert_eq!(errors.len(), 2);
    assert!(errors[0].message.contains("⏎") || errors[0].message.contains("nonexistent"));
}

#[test]
fn no_diagnostics_returns_empty() {
    let result = parse_stderr(&fixture("no_diagnostics.txt"));
    assert!(result.diagnostics.is_empty());
}
