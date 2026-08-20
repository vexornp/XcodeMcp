use xcode_mcp_core::*;

#[test]
fn severity_serializes_lowercase() {
    assert_eq!(
        serde_json::to_string(&Severity::Error).unwrap(),
        "\"error\""
    );
    assert_eq!(
        serde_json::to_string(&Severity::Warning).unwrap(),
        "\"warning\""
    );
}

#[test]
fn diagnostic_source_serializes_snake_case() {
    assert_eq!(
        serde_json::to_string(&DiagnosticSource::Xcresult).unwrap(),
        "\"xcresult\""
    );
    assert_eq!(
        serde_json::to_string(&DiagnosticSource::Stderr).unwrap(),
        "\"stderr\""
    );
}

#[test]
fn diagnostic_round_trips() {
    let d = Diagnostic {
        file: Some("/tmp/foo.swift".into()),
        line: Some(42),
        column: Some(8),
        severity: Severity::Error,
        message: "use of undeclared identifier 'bar'".into(),
        category: Some("Swift Compiler Error".into()),
        fix_its: None,
        source: DiagnosticSource::Xcresult,
    };
    let json = serde_json::to_string(&d).unwrap();
    let back: Diagnostic = serde_json::from_str(&json).unwrap();
    assert_eq!(back.file, d.file);
    assert_eq!(back.line, d.line);
    assert_eq!(back.severity, d.severity);
}

#[test]
fn error_display_formats() {
    let e = Error::InvalidArgument("scheme too long".into());
    assert_eq!(e.to_string(), "invalid argument: scheme too long");
}

#[test]
fn build_status_pod_failed_serializes_as_pascal_case() {
    use xcode_mcp_core::store::BuildStatus;
    let json = serde_json::to_string(&BuildStatus::PodFailed).unwrap();
    assert_eq!(json, "\"PodFailed\"");
    let back: BuildStatus = serde_json::from_str("\"PodFailed\"").unwrap();
    assert_eq!(back, BuildStatus::PodFailed);
}

#[test]
fn podfile_not_found_error_displays_working_dir() {
    use std::path::PathBuf;
    use xcode_mcp_core::error::Error;
    let e = Error::PodfileNotFound {
        working_dir: PathBuf::from("/tmp/proj"),
    };
    assert_eq!(e.to_string(), "no Podfile found next to /tmp/proj");
}
