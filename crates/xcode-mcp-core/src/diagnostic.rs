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

const STDERR_DIAG_RE: &str = r"^(?P<file>[^:\n]+):(?P<line>\d+):(?:(?P<col>\d+):)?\s*(?P<sev>error|warning|note|fatal error):\s*(?P<msg>.*)$";

pub fn parse_stderr(stderr: &str) -> ParseResult {
    let re = match regex::Regex::new(STDERR_DIAG_RE) {
        Ok(r) => r,
        Err(e) => {
            return ParseResult {
                diagnostics: vec![],
                parse_warnings: vec![format!("regex failed: {e}")],
            }
        }
    };
    let mut diagnostics: Vec<Diagnostic> = Vec::new();
    for line in stderr.lines() {
        if let Some(caps) = re.captures(line) {
            let file = caps.name("file").map(|m| m.as_str().to_string());
            let line_num = caps
                .name("line")
                .and_then(|m| m.as_str().parse::<u32>().ok());
            let column = caps
                .name("col")
                .and_then(|m| m.as_str().parse::<u32>().ok());
            let sev_str = caps.name("sev").map(|m| m.as_str()).unwrap_or("note");
            let message = caps
                .name("msg")
                .map(|m| m.as_str().to_string())
                .unwrap_or_default();
            let severity = match sev_str {
                "error" | "fatal error" => Severity::Error,
                "warning" => Severity::Warning,
                _ => Severity::Note,
            };
            let category = infer_category(&file, &message);
            diagnostics.push(Diagnostic {
                file,
                line: line_num,
                column,
                severity,
                message,
                category,
                fix_its: None,
                source: DiagnosticSource::Stderr,
            });
        } else if let Some(d) = parse_ld_line(line) {
            diagnostics.push(d);
        } else if line.starts_with(char::is_whitespace) && !line.trim().is_empty() {
            if let Some(last) = diagnostics.last_mut() {
                if last.source == DiagnosticSource::Stderr {
                    last.message.push_str(" ⏎ ");
                    last.message.push_str(line.trim());
                }
            }
        }
    }
    ParseResult {
        diagnostics,
        parse_warnings: vec![],
    }
}

fn parse_ld_line(line: &str) -> Option<Diagnostic> {
    let trimmed = line.trim_start();
    let rest = trimmed.strip_prefix("ld:")?;
    let rest = rest.trim_start();
    let (severity, message) = if let Some(msg) = rest.strip_prefix("warning:") {
        (Severity::Warning, msg.trim().to_string())
    } else if let Some(msg) = rest.strip_prefix("error:") {
        (Severity::Error, msg.trim().to_string())
    } else {
        (Severity::Error, rest.to_string())
    };
    Some(Diagnostic {
        file: None,
        line: None,
        column: None,
        severity,
        message,
        category: Some("Linker".into()),
        fix_its: None,
        source: DiagnosticSource::Stderr,
    })
}

fn infer_category(file: &Option<String>, message: &str) -> Option<String> {
    let f = file.as_deref().unwrap_or("");
    let m = message.to_lowercase();
    if f.ends_with(".o")
        || f.ends_with(".a")
        || f.ends_with(".dylib")
        || f.ends_with(".framework")
        || m.starts_with("ld:")
        || m.contains("linker")
    {
        return Some("Linker".into());
    }
    if f.is_empty() {
        return Some("Build System".into());
    }
    Some("Compiler".into())
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MergedDiagnostics {
    pub errors: Vec<Diagnostic>,
    pub warnings: Vec<Diagnostic>,
    pub notes: Vec<Diagnostic>,
    pub parse_warnings: Vec<String>,
}

#[allow(clippy::type_complexity)]
pub fn merge_diagnostics(xcresult: ParseResult, stderr: ParseResult) -> MergedDiagnostics {
    let mut parse_warnings = xcresult.parse_warnings;
    parse_warnings.extend(stderr.parse_warnings);
    let mut all: Vec<Diagnostic> = xcresult.diagnostics;
    all.extend(stderr.diagnostics);
    let mut seen: Vec<(Option<String>, Option<u32>, Option<u32>, Severity, String)> = Vec::new();
    let mut deduped: Vec<Diagnostic> = Vec::new();
    for d in &all {
        let norm = normalize_message(&d.message);
        let key = (d.file.clone(), d.line, d.column, d.severity.clone(), norm);
        if seen.contains(&key) {
            continue;
        }
        seen.push(key);
        deduped.push(d.clone());
    }
    let mut errors: Vec<_> = deduped
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .cloned()
        .collect();
    let mut warnings: Vec<_> = deduped
        .iter()
        .filter(|d| d.severity == Severity::Warning)
        .cloned()
        .collect();
    let mut notes: Vec<_> = deduped
        .iter()
        .filter(|d| d.severity == Severity::Note)
        .cloned()
        .collect();
    let cmp = |a: &Diagnostic, b: &Diagnostic| {
        (
            a.file.as_deref().unwrap_or(""),
            a.line.unwrap_or(0),
            a.column.unwrap_or(0),
        )
            .cmp(&(
                b.file.as_deref().unwrap_or(""),
                b.line.unwrap_or(0),
                b.column.unwrap_or(0),
            ))
    };
    errors.sort_by(cmp);
    warnings.sort_by(cmp);
    notes.sort_by(cmp);
    MergedDiagnostics {
        errors,
        warnings,
        notes,
        parse_warnings,
    }
}

fn normalize_message(msg: &str) -> String {
    msg.split_whitespace().collect::<Vec<_>>().join(" ")
}
