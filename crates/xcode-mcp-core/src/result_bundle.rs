use std::path::Path;

use crate::diagnostic::{Diagnostic, DiagnosticSource, FixIt, FixItRange, ParseResult, Severity};
use crate::error::{Error, Result};
use crate::xcode::{build_xcresulttool_command, run_supervised};

pub fn parse_build_results(json: &str) -> Result<ParseResult> {
    let root: serde_json::Value = serde_json::from_str(json)?;
    let mut diagnostics = Vec::new();
    let mut parse_warnings = Vec::new();
    let mut recognized = false;

    if root.get("actions").and_then(|a| a.as_array()).is_some() {
        recognized = true;
        for action in root["actions"].as_array().unwrap() {
            if let Some(issues) = action
                .get("_results")
                .and_then(|r| r.get("issues"))
                .and_then(|i| i.as_array())
            {
                for issue in issues {
                    if let Some(d) = parse_issue(issue, &mut parse_warnings) {
                        diagnostics.push(d);
                    }
                }
            }
        }
    } else {
        for key in ["errors", "warnings", "analyzerWarnings", "analyzerErrors"] {
            if let Some(issues) = root.get(key).and_then(|a| a.as_array()) {
                recognized = true;
                for issue in issues {
                    if let Some(d) = parse_issue(issue, &mut parse_warnings) {
                        diagnostics.push(d);
                    }
                }
            }
        }
    }

    if !recognized {
        return Err(Error::UnrecognizedResultFormat);
    }

    Ok(ParseResult {
        diagnostics,
        parse_warnings,
    })
}

fn parse_issue(issue: &serde_json::Value, warnings: &mut Vec<String>) -> Option<Diagnostic> {
    let issue_type = issue
        .get("issueType")
        .and_then(|t| t.as_str())
        .unwrap_or("");
    let severity = match issue_type {
        "BuildError"
        | "AnalyzerError"
        | "Swift Compiler Error"
        | "Build Error"
        | "Analyzer Error" => Severity::Error,
        "BuildWarning"
        | "AnalyzerWarning"
        | "Swift Compiler Warning"
        | "Build Warning"
        | "Analyzer Warning"
        | "Deprecation Warning" => Severity::Warning,
        "Note" => Severity::Note,
        other => {
            let lower = other.to_lowercase();
            if lower.contains("error") {
                Severity::Error
            } else if lower.contains("warning") {
                Severity::Warning
            } else {
                warnings.push(format!("unknown issueType: {other}"));
                Severity::Note
            }
        }
    };
    let message = issue
        .get("message")
        .and_then(|m| m.as_str())
        .unwrap_or("(no message)")
        .to_string();
    let category = issue
        .get("category")
        .and_then(|c| c.as_str())
        .map(String::from);
    let (file, line, column) = parse_document_location(issue);
    let fix_its = issue
        .get("fixIts")
        .and_then(|f| f.as_array())
        .map(|arr| arr.iter().filter_map(parse_fix_it).collect());
    Some(Diagnostic {
        file,
        line,
        column,
        severity,
        message,
        category,
        fix_its,
        source: DiagnosticSource::Xcresult,
    })
}

fn parse_document_location(
    issue: &serde_json::Value,
) -> (Option<String>, Option<u32>, Option<u32>) {
    let url = issue
        .get("documentLocationInCreatingWorkspace")
        .and_then(|d| d.get("url"))
        .and_then(|u| u.as_str())
        .or_else(|| issue.get("sourceURL").and_then(|u| u.as_str()));
    let Some(url) = url else {
        return (None, None, None);
    };
    let (path, fragment) = match url.split_once('#') {
        Some((p, f)) => (p, f),
        None => (url, ""),
    };
    let file = if let Some(s) = path.strip_prefix("file://") {
        Some(s.to_string())
    } else if path.starts_with('/') {
        Some(path.to_string())
    } else {
        None
    };
    (
        file,
        extract_num(fragment, "Line=").or_else(|| extract_num(fragment, "StartingLineNumber=")),
        extract_num(fragment, "Column=").or_else(|| extract_num(fragment, "StartingColumnNumber=")),
    )
}

fn extract_num(fragment: &str, key: &str) -> Option<u32> {
    let start = fragment.find(key)?;
    let rest = &fragment[start + key.len()..];
    let end = rest
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(rest.len());
    rest[..end].parse().ok()
}

fn parse_fix_it(val: &serde_json::Value) -> Option<FixIt> {
    let message = val.get("message").and_then(|m| m.as_str())?.to_string();
    let range = val.get("range").and_then(parse_fix_it_range);
    Some(FixIt { message, range })
}

fn parse_fix_it_range(val: &serde_json::Value) -> Option<FixItRange> {
    Some(FixItRange {
        start_line: val.get("startLine")?.as_u64()? as u32,
        start_col: val.get("startColumn")?.as_u64()? as u32,
        end_line: val.get("endLine")?.as_u64()? as u32,
        end_col: val.get("endColumn")?.as_u64()? as u32,
    })
}

pub async fn read_build_results(xcresult_path: &Path) -> Result<ParseResult> {
    let cmd = build_xcresulttool_command(xcresult_path);
    let result = run_supervised(cmd, 60, None).await?;
    if result.timed_out {
        return Err(Error::XcresulttoolFailed {
            exit_code: None,
            stderr_excerpt: "timed out".into(),
        });
    }
    if result.exit_code != Some(0) {
        let excerpt = String::from_utf8_lossy(&result.stderr)
            .chars()
            .take(2000)
            .collect();
        return Err(Error::XcresulttoolFailed {
            exit_code: result.exit_code,
            stderr_excerpt: excerpt,
        });
    }
    let stdout = String::from_utf8_lossy(&result.stdout);
    if stdout.trim().is_empty() {
        return Err(Error::XcresulttoolFailed {
            exit_code: result.exit_code,
            stderr_excerpt: "empty output".into(),
        });
    }
    parse_build_results(&stdout)
}
