use crate::diagnostic::{Diagnostic, DiagnosticSource, FixIt, FixItRange, ParseResult, Severity};
use crate::error::{Error, Result};

pub fn parse_build_results(json: &str) -> Result<ParseResult> {
    let root: serde_json::Value = serde_json::from_str(json)?;
    let actions = root
        .get("actions")
        .and_then(|a| a.as_array())
        .ok_or(Error::UnrecognizedResultFormat)?;
    let mut diagnostics = Vec::new();
    let mut parse_warnings = Vec::new();
    for action in actions {
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
        "BuildError" | "AnalyzerError" => Severity::Error,
        "BuildWarning" | "AnalyzerWarning" => Severity::Warning,
        "Note" => Severity::Note,
        other => {
            warnings.push(format!("unknown issueType: {other}"));
            Severity::Note
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
        .and_then(|u| u.as_str());
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
        extract_num(fragment, "Line="),
        extract_num(fragment, "Column="),
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
