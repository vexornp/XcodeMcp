use xcode_mcp_core::diagnostic::*;

fn make_diag(
    sev: Severity,
    file: &str,
    line: u32,
    col: u32,
    msg: &str,
    src: DiagnosticSource,
) -> Diagnostic {
    Diagnostic {
        file: Some(file.into()),
        line: Some(line),
        column: Some(col),
        severity: sev,
        message: msg.into(),
        category: None,
        fix_its: None,
        source: src,
    }
}

#[test]
fn merges_xcresult_and_stderr() {
    let xcresult = ParseResult {
        diagnostics: vec![make_diag(
            Severity::Error,
            "/a.swift",
            10,
            5,
            "foo",
            DiagnosticSource::Xcresult,
        )],
        parse_warnings: vec![],
    };
    let stderr = ParseResult {
        diagnostics: vec![make_diag(
            Severity::Warning,
            "/b.swift",
            1,
            1,
            "bar",
            DiagnosticSource::Stderr,
        )],
        parse_warnings: vec![],
    };
    let merged = merge_diagnostics(xcresult, stderr);
    assert_eq!(merged.errors.len(), 1);
    assert_eq!(merged.warnings.len(), 1);
    assert!(merged.notes.is_empty());
}

#[test]
fn dedups_same_diagnostic_keeps_xcresult() {
    let xcresult = ParseResult {
        diagnostics: vec![make_diag(
            Severity::Error,
            "/a.swift",
            10,
            5,
            "undeclared identifier 'foo'",
            DiagnosticSource::Xcresult,
        )],
        parse_warnings: vec![],
    };
    let stderr = ParseResult {
        diagnostics: vec![make_diag(
            Severity::Error,
            "/a.swift",
            10,
            5,
            "undeclared identifier 'foo'",
            DiagnosticSource::Stderr,
        )],
        parse_warnings: vec![],
    };
    let merged = merge_diagnostics(xcresult, stderr);
    assert_eq!(merged.errors.len(), 1);
    assert_eq!(merged.errors[0].source, DiagnosticSource::Xcresult);
}

#[test]
fn normalizes_whitespace_for_dedup() {
    let xcresult = ParseResult {
        diagnostics: vec![make_diag(
            Severity::Error,
            "/a.swift",
            10,
            5,
            "undeclared  identifier   'foo'",
            DiagnosticSource::Xcresult,
        )],
        parse_warnings: vec![],
    };
    let stderr = ParseResult {
        diagnostics: vec![make_diag(
            Severity::Error,
            "/a.swift",
            10,
            5,
            "undeclared identifier 'foo'",
            DiagnosticSource::Stderr,
        )],
        parse_warnings: vec![],
    };
    let merged = merge_diagnostics(xcresult, stderr);
    assert_eq!(merged.errors.len(), 1);
}

#[test]
fn sorts_by_file_then_line_then_column() {
    let xcresult = ParseResult {
        diagnostics: vec![
            make_diag(
                Severity::Error,
                "/b.swift",
                5,
                1,
                "e2",
                DiagnosticSource::Xcresult,
            ),
            make_diag(
                Severity::Error,
                "/a.swift",
                20,
                1,
                "e1",
                DiagnosticSource::Xcresult,
            ),
            make_diag(
                Severity::Error,
                "/a.swift",
                10,
                1,
                "e0",
                DiagnosticSource::Xcresult,
            ),
        ],
        parse_warnings: vec![],
    };
    let merged = merge_diagnostics(xcresult, ParseResult::default());
    assert_eq!(merged.errors[0].line, Some(10));
    assert_eq!(merged.errors[1].line, Some(20));
    assert_eq!(merged.errors[2].file.as_deref(), Some("/b.swift"));
}

#[test]
fn combines_parse_warnings() {
    let xcresult = ParseResult {
        diagnostics: vec![],
        parse_warnings: vec!["xc warning".into()],
    };
    let stderr = ParseResult {
        diagnostics: vec![],
        parse_warnings: vec!["stderr warning".into()],
    };
    let merged = merge_diagnostics(xcresult, stderr);
    assert!(merged.parse_warnings.contains(&"xc warning".to_string()));
    assert!(merged
        .parse_warnings
        .contains(&"stderr warning".to_string()));
}
