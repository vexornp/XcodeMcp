use std::path::Path;

use crate::error::{Error, Result};
use crate::security::validate_project_or_workspace;
use crate::xcode::{build_list_schemes_command, run_supervised};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListInfo {
    pub schemes: Vec<String>,
    pub configurations: Vec<String>,
    pub targets: Vec<String>,
    pub parse_warnings: Vec<String>,
}

const ENTRY_RE: &str = r"^[A-Za-z0-9_ .\-]{1,128}$";

pub fn parse_list_output(stdout: &str) -> Result<ListInfo> {
    let entry_re = regex::Regex::new(ENTRY_RE).unwrap();
    let mut info = ListInfo {
        schemes: vec![],
        configurations: vec![],
        targets: vec![],
        parse_warnings: vec![],
    };
    let mut current_section: Option<&str> = None;
    let mut found_any_section = false;

    for line in stdout.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            current_section = None;
            continue;
        }
        if trimmed.ends_with(':') && !line.starts_with(char::is_whitespace) {
            let header = &trimmed[..trimmed.len() - 1];
            current_section = match header {
                "Schemes" => {
                    found_any_section = true;
                    Some("schemes")
                }
                "Targets" => {
                    found_any_section = true;
                    Some("targets")
                }
                "Build Configurations" => {
                    found_any_section = true;
                    Some("configurations")
                }
                _ => {
                    info.parse_warnings
                        .push(format!("unknown section: {header}"));
                    None
                }
            };
            continue;
        }
        if let Some(section) = current_section {
            let entry = trimmed.to_string();
            if entry.is_empty() {
                continue;
            }
            if !entry_re.is_match(&entry) {
                info.parse_warnings
                    .push(format!("dropped entry with invalid charset: {entry:?}"));
                continue;
            }
            match section {
                "schemes" => info.schemes.push(entry),
                "targets" => info.targets.push(entry),
                "configurations" => info.configurations.push(entry),
                _ => {}
            }
        }
    }

    if !found_any_section {
        return Err(Error::UnrecognizedListFormat);
    }
    if info.schemes.is_empty() {
        info.parse_warnings.push("no schemes found".into());
    }
    info.schemes = dedup_sorted(info.schemes);
    info.targets = dedup_sorted(info.targets);
    info.configurations = dedup_sorted(info.configurations);
    Ok(info)
}

fn dedup_sorted(mut v: Vec<String>) -> Vec<String> {
    v.sort();
    v.dedup();
    v
}

pub async fn list_schemes(project_or_workspace: &str, root: &Path) -> Result<ListInfo> {
    let validated_path = validate_project_or_workspace(project_or_workspace, root)?;
    let cmd = build_list_schemes_command(&validated_path);
    let result = run_supervised(cmd, 30, None).await?;
    if result.timed_out {
        return Err(crate::error::Error::XcodeListFailed {
            exit_code: None,
            stderr_excerpt: "timed out".into(),
        });
    }
    if result.exit_code != Some(0) {
        let excerpt = String::from_utf8_lossy(&result.stderr)
            .chars()
            .take(2000)
            .collect();
        return Err(crate::error::Error::XcodeListFailed {
            exit_code: result.exit_code,
            stderr_excerpt: excerpt,
        });
    }
    let stdout = String::from_utf8_lossy(&result.stdout);
    parse_list_output(&stdout)
}
