# Xcode MCP Server Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a local MCP server in Rust that drives `xcodebuild` and parses build-failure diagnostics into structured data.

**Architecture:** Cargo workspace with `xcode-mcp-core` (lib: all logic, fully unit-tested) and `xcode-mcp` (bin: thin rmcp stdio server + debug CLI). Hybrid diagnostic sourcing: primary `xcresulttool get build-results` JSON, fallback stderr regex parsing, merged with dedup.

**Tech Stack:** Rust 1.97 (edition 2021), rmcp 3.1.1, tokio, serde/serde_json, thiserror, regex, clap, libc. Xcode 26+ with modern `xcresulttool`.

## Global Constraints

- **Rust edition:** 2021, MSRV 1.97
- **rmcp version:** pinned `"3.1.1"` — do not use other versions
- **No shell invocation:** all subprocess calls use `Command::new(...).arg(...)` — never `sh -c`
- **No stdout/stderr logging in server mode:** stdout/stderr are the MCP stdio channel; logs go to `$LOG_DIR/server.log` only
- **No `extra_args` passthrough:** fixed xcodebuild flag surface only
- **Security boundary:** `XCODE_MCP_ROOT` env var — all project paths must canonicalize under it
- **Feature gate:** `live-xcode` feature (default off) + `XCODE_MCP_LIVE_TESTS=1` env var — double gate for live integration tests
- **macOS only:** depends on `xcrun`, `xcodebuild`, `xcresulttool`
- **Commit style:** conventional commits (`feat:`, `test:`, `docs:`, `chore:`, `fix:`)

---

## Task 1: Foundation — Workspace Scaffold, Error Types, Shared Types

**Files:**
- Create: `Cargo.toml`, `crates/xcode-mcp-core/Cargo.toml`, `crates/xcode-mcp/Cargo.toml`
- Create: `crates/xcode-mcp-core/src/lib.rs`, `error.rs`, `diagnostic.rs` (types only)
- Create: `crates/xcode-mcp/src/main.rs` (stub)
- Test: `crates/xcode-mcp-core/tests/types.rs`

**Interfaces:**
- Produces: `Error` enum, `Severity`, `Diagnostic`, `FixIt`, `FixItRange`, `DiagnosticSource`, `ParseResult` (in `diagnostic.rs`), re-exported from `lib.rs`

- [ ] **Step 1: Create root workspace Cargo.toml**

```toml
[workspace]
members = ["crates/xcode-mcp-core", "crates/xcode-mcp"]
resolver = "2"

[workspace.package]
edition = "2021"
rust-version = "1.97"
version = "0.1.0"
license = "MIT"

[workspace.dependencies]
rmcp = { version = "3.1.1" }
tokio = { version = "1", features = ["rt-multi-thread", "macros", "process", "io-util", "sync", "fs", "time"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
thiserror = "2"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
uuid = { version = "1", features = ["v4"] }
regex = "1"
clap = { version = "4", features = ["derive"] }
libc = "0.2"
tempfile = "3"
```

- [ ] **Step 2: Create xcode-mcp-core/Cargo.toml**

```toml
[package]
name = "xcode-mcp-core"
edition.workspace = true
rust-version.workspace = true
version.workspace = true
license.workspace = true

[features]
live-xcode = []

[dependencies]
tokio.workspace = true
serde.workspace = true
serde_json.workspace = true
thiserror.workspace = true
tracing.workspace = true
uuid.workspace = true
regex.workspace = true
libc.workspace = true

[dev-dependencies]
tempfile.workspace = true
```

- [ ] **Step 3: Create xcode-mcp/Cargo.toml**

```toml
[package]
name = "xcode-mcp"
edition.workspace = true
rust-version.workspace = true
version.workspace = true
license.workspace = true

[dependencies]
xcode-mcp-core = { path = "../xcode-mcp-core" }
rmcp.workspace = true
tokio.workspace = true
serde.workspace = true
serde_json.workspace = true
tracing.workspace = true
tracing-subscriber.workspace = true
clap.workspace = true
```

- [ ] **Step 4: Create error.rs**

```rust
use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid argument: {0}")]
    InvalidArgument(String),

    #[error("XCODE_MCP_ROOT not set or invalid: {0}")]
    RootNotConfigured(String),

    #[error("path rejected by security policy: {0}")]
    PathRejected(String),

    #[error("path not found: {0}")]
    PathNotFound(PathBuf),

    #[error("xcodebuild spawn failed: {0}")]
    XcodeSpawnFailed(String),

    #[error("xcodebuild -list failed (exit {exit_code:?}): {stderr_excerpt}")]
    XcodeListFailed { exit_code: Option<i32>, stderr_excerpt: String },

    #[error("unrecognized -list output format")]
    UnrecognizedListFormat,

    #[error("unrecognized xcresult format")]
    UnrecognizedResultFormat,

    #[error("build not found: {0}")]
    BuildNotFound(String),

    #[error("no build available: {hint}")]
    NoBuildAvailable { hint: String },

    #[error("xcresulttool failed (exit {exit_code:?}): {stderr_excerpt}")]
    XcresulttoolFailed { exit_code: Option<i32>, stderr_excerpt: String },

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("internal error: {0}")]
    Internal(String),
}

pub type Result<T> = std::result::Result<T, Error>;
```

- [ ] **Step 5: Create diagnostic.rs (types only — functions added in Tasks 5-6)**

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity { Error, Warning, Note }

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
pub enum DiagnosticSource { Xcresult, Stderr }

#[derive(Debug, Clone, Default)]
pub struct ParseResult {
    pub diagnostics: Vec<Diagnostic>,
    pub parse_warnings: Vec<String>,
}
```

- [ ] **Step 6: Create lib.rs**

```rust
pub mod diagnostic;
pub mod error;

pub use diagnostic::{Diagnostic, DiagnosticSource, FixIt, FixItRange, ParseResult, Severity};
pub use error::{Error, Result};
```

- [ ] **Step 7: Create main.rs stub**

```rust
fn main() {
    eprintln!("xcode-mcp: not yet implemented");
    std::process::exit(1);
}
```

- [ ] **Step 8: Write types test**

Create `crates/xcode-mcp-core/tests/types.rs`:

```rust
use xcode_mcp_core::*;

#[test]
fn severity_serializes_lowercase() {
    assert_eq!(serde_json::to_string(&Severity::Error).unwrap(), "\"error\"");
    assert_eq!(serde_json::to_string(&Severity::Warning).unwrap(), "\"warning\"");
}

#[test]
fn diagnostic_source_serializes_snake_case() {
    assert_eq!(serde_json::to_string(&DiagnosticSource::Xcresult).unwrap(), "\"xcresult\"");
    assert_eq!(serde_json::to_string(&DiagnosticSource::Stderr).unwrap(), "\"stderr\"");
}

#[test]
fn diagnostic_round_trips() {
    let d = Diagnostic {
        file: Some("/tmp/foo.swift".into()), line: Some(42), column: Some(8),
        severity: Severity::Error, message: "use of undeclared identifier 'bar'".into(),
        category: Some("Swift Compiler Error".into()), fix_its: None, source: DiagnosticSource::Xcresult,
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
```

- [ ] **Step 9: Run tests**

Run: `cargo test`
Expected: all tests pass, both crates compile.

- [ ] **Step 10: Commit**

```bash
git add -A && git commit -m "feat: scaffold workspace with error types and shared diagnostic types"
```

---

## Task 2: Security — Path, Scheme, and Argument Validation

**Files:**
- Create: `crates/xcode-mcp-core/src/security.rs`
- Modify: `crates/xcode-mcp-core/src/lib.rs` (add `pub mod security;`)
- Test: `crates/xcode-mcp-core/tests/security.rs`

**Interfaces:**
- Produces: `validate_project_or_workspace(path: &str, root: &Path) -> Result<PathBuf>`, `validate_scheme(s: &str) -> Result<String>`, `validate_configuration(c: &str) -> Result<String>`, `validate_action(a: &str) -> Result<String>`, `validate_destination(d: &str) -> Result<String>`, `validate_timeout(t: Option<u32>) -> Result<u32>`, `validate_build_id(id: &str) -> Result<String>`

- [ ] **Step 1: Write failing tests for path validation**

Create `crates/xcode-mcp-core/tests/security.rs`:

```rust
use std::fs;
use tempfile::tempdir;
use xcode_mcp_core::security::*;

fn make_root() -> std::path::PathBuf {
    let dir = tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    std::mem::forget(dir);
    root
}

#[test]
fn accepts_xcodeproj_under_root() {
    let root = make_root();
    let proj = root.join("App.xcodeproj");
    fs::create_dir(&proj).unwrap();
    let validated = validate_project_or_workspace(proj.to_str().unwrap(), &root).unwrap();
    assert_eq!(validated, proj.canonicalize().unwrap());
}

#[test]
fn accepts_xcworkspace_under_root() {
    let root = make_root();
    let ws = root.join("App.xcworkspace");
    fs::create_dir(&ws).unwrap();
    let validated = validate_project_or_workspace(ws.to_str().unwrap(), &root).unwrap();
    assert_eq!(validated, ws.canonicalize().unwrap());
}

#[test]
fn rejects_path_outside_root() {
    let root = make_root();
    let outside = tempdir().unwrap();
    let proj = outside.path().join("Evil.xcodeproj");
    fs::create_dir(&proj).unwrap();
    assert!(validate_project_or_workspace(proj.to_str().unwrap(), &root).is_err());
}

#[test]
fn rejects_path_traversal() {
    let root = make_root();
    let evil = format!("{}/../../etc/passwd.xcodeproj", root.display());
    assert!(validate_project_or_workspace(&evil, &root).is_err());
}

#[test]
fn rejects_wrong_extension() {
    let root = make_root();
    let fake = root.join("App.txt");
    fs::write(&fake, "not a project").unwrap();
    assert!(validate_project_or_workspace(fake.to_str().unwrap(), &root).is_err());
}

#[test]
fn rejects_nonexistent_path() {
    let root = make_root();
    assert!(validate_project_or_workspace(root.join("Nope.xcodeproj").to_str().unwrap(), &root).is_err());
}

#[test]
fn scheme_accepts_normal_name() {
    assert_eq!(validate_scheme("App").unwrap(), "App");
    assert_eq!(validate_scheme("My-App 2.0").unwrap(), "My-App 2.0");
}

#[test]
fn scheme_rejects_shell_metachars() {
    assert!(validate_scheme("App; rm -rf /").is_err());
    assert!(validate_scheme("App && evil").is_err());
    assert!(validate_scheme("App`whoami`").is_err());
    assert!(validate_scheme("App\nnewline").is_err());
}

#[test]
fn scheme_rejects_too_long() {
    assert!(validate_scheme(&"A".repeat(129)).is_err());
    assert!(validate_scheme(&"A".repeat(128)).is_ok());
}

#[test]
fn configuration_validates() {
    assert!(validate_configuration("Debug").is_ok());
    assert!(validate_configuration("Release").is_ok());
    assert!(validate_configuration("debug").is_err());
    assert!(validate_configuration("Profile").is_err());
}

#[test]
fn action_validates() {
    assert!(validate_action("build").is_ok());
    assert!(validate_action("clean").is_ok());
    assert!(validate_action("clean+build").is_ok());
    assert!(validate_action("test").is_err());
}

#[test]
fn destination_accepts_known_formats() {
    assert!(validate_destination("generic/platform=iOS").is_ok());
    assert!(validate_destination("platform=macOS").is_ok());
    assert!(validate_destination("id=ABCD-1234").is_ok());
}

#[test]
fn destination_rejects_metachars() {
    assert!(validate_destination("generic/platform=iOS; rm -rf /").is_err());
    assert!(validate_destination("$(whoami)").is_err());
}

#[test]
fn timeout_defaults_and_validates() {
    assert_eq!(validate_timeout(None).unwrap(), 1800);
    assert_eq!(validate_timeout(Some(60)).unwrap(), 60);
    assert_eq!(validate_timeout(Some(7200)).unwrap(), 7200);
    assert!(validate_timeout(Some(0)).is_err());
    assert!(validate_timeout(Some(7201)).is_err());
}

#[test]
fn build_id_accepts_uuid() {
    assert!(validate_build_id("550e8400-e29b-41d4-a716-446655440000").is_ok());
}

#[test]
fn build_id_rejects_slashes() {
    assert!(validate_build_id("../../etc/passwd").is_err());
    assert!(validate_build_id("foo/bar").is_err());
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --test security`
Expected: FAIL — `security` module not found.

- [ ] **Step 3: Implement security.rs**

```rust
use crate::error::{Error, Result};
use std::path::{Path, PathBuf};

const SCHEME_RE: &str = r"^[A-Za-z0-9_ .\-]{1,128}$";
const DESTINATION_RE: &str = r"^[A-Za-z0-9_ ./=\-,]{1,256}$";
const BUILD_ID_RE: &str = r"^[0-9a-fA-F\-]{1,64}$";

pub fn validate_project_or_workspace(path_str: &str, root: &Path) -> Result<PathBuf> {
    let path = Path::new(path_str);
    let ext = path.extension().and_then(|e| e.to_str())
        .ok_or_else(|| Error::PathRejected(format!("missing extension: {path_str}")))?;
    if ext != "xcodeproj" && ext != "xcworkspace" {
        return Err(Error::PathRejected(format!("must be .xcodeproj or .xcworkspace, got .{ext}")));
    }
    if !path.exists() {
        return Err(Error::PathNotFound(path.to_path_buf()));
    }
    let canonical = path.canonicalize()?;
    let canonical_root = root.canonicalize()?;
    if !is_under_or_equal(&canonical, &canonical_root) {
        return Err(Error::PathRejected(format!(
            "path {} is outside root {}", canonical.display(), canonical_root.display()
        )));
    }
    Ok(canonical)
}

fn is_under_or_equal(path: &Path, root: &Path) -> bool {
    if path == root { return true; }
    path.starts_with(root)
}

pub fn validate_scheme(s: &str) -> Result<String> {
    let re = regex::Regex::new(SCHEME_RE).unwrap();
    if !re.is_match(s) {
        return Err(Error::InvalidArgument(format!("invalid scheme: {s:?}")));
    }
    Ok(s.to_string())
}

pub fn validate_configuration(c: &str) -> Result<String> {
    match c {
        "Debug" | "Release" => Ok(c.to_string()),
        _ => Err(Error::InvalidArgument(format!("configuration must be Debug or Release: {c:?}"))),
    }
}

pub fn validate_action(a: &str) -> Result<String> {
    match a {
        "build" | "clean" | "clean+build" => Ok(a.to_string()),
        _ => Err(Error::InvalidArgument(format!("action must be build/clean/clean+build: {a:?}"))),
    }
}

pub fn validate_destination(d: &str) -> Result<String> {
    let re = regex::Regex::new(DESTINATION_RE).unwrap();
    if !re.is_match(d) {
        return Err(Error::InvalidArgument(format!("invalid destination: {d:?}")));
    }
    Ok(d.to_string())
}

pub fn validate_timeout(t: Option<u32>) -> Result<u32> {
    match t {
        None => Ok(1800),
        Some(v) if (1..=7200).contains(&v) => Ok(v),
        Some(v) => Err(Error::InvalidArgument(format!("timeout_secs must be 1..=7200: {v}"))),
    }
}

pub fn validate_build_id(id: &str) -> Result<String> {
    let re = regex::Regex::new(BUILD_ID_RE).unwrap();
    if !re.is_match(id) {
        return Err(Error::InvalidArgument(format!("invalid build_id: {id:?}")));
    }
    Ok(id.to_string())
}
```

- [ ] **Step 4: Add module to lib.rs** — add `pub mod security;` after `pub mod error;`

- [ ] **Step 5: Run tests**

Run: `cargo test --test security`
Expected: all tests pass.

- [ ] **Step 6: Commit**

```bash
git add -A && git commit -m "feat: add security validation for paths, schemes, and build arguments"
```

---

## Task 3: Scheme List Parsing

**Files:**
- Create: `crates/xcode-mcp-core/src/scheme.rs` (parser only)
- Modify: `crates/xcode-mcp-core/src/lib.rs` (add `pub mod scheme;`)
- Create fixtures: `tests/fixtures/list/{typical,no_schemes,reordered,extra_sections,malformed}.txt`
- Test: `crates/xcode-mcp-core/tests/scheme_parse.rs`

**Interfaces:**
- Produces: `ListInfo { schemes, configurations, targets, parse_warnings }`, `parse_list_output(stdout: &str) -> Result<ListInfo>`

- [ ] **Step 1: Create fixture files**

`tests/fixtures/list/typical.txt`:
```
Schemes:
  App
  AppTests

Targets:
  App
  AppTests

Build Configurations:
  Debug
  Release

If no build configuration is specified, Production is used.
```

`tests/fixtures/list/no_schemes.txt`:
```
Schemes:

Targets:
  App

Build Configurations:
  Debug
  Release
```

`tests/fixtures/list/reordered.txt`:
```
Targets:
  App
  AppTests

Build Configurations:
  Debug
  Release

Schemes:
  App
  AppTests
```

`tests/fixtures/list/extra_sections.txt`:
```
Schemes:
  App

Targets:
  App

Build Configurations:
  Debug
  Release

Swift Packages:
  SomePackage
```

`tests/fixtures/list/malformed.txt`:
```
This is not a valid xcodebuild -list output.
Nothing to see here.
```

- [ ] **Step 2: Write failing tests**

Create `crates/xcode-mcp-core/tests/scheme_parse.rs`:

```rust
use std::fs;
use xcode_mcp_core::scheme::*;

fn fixture(name: &str) -> String {
    fs::read_to_string(format!("tests/fixtures/list/{name}")).unwrap()
}

#[test]
fn parses_typical_output() {
    let info = parse_list_output(&fixture("typical.txt")).unwrap();
    assert_eq!(info.schemes, vec!["App", "AppTests"]);
    assert_eq!(info.targets, vec!["App", "AppTests"]);
    assert_eq!(info.configurations, vec!["Debug", "Release"]);
    assert!(info.parse_warnings.is_empty());
}

#[test]
fn handles_no_schemes() {
    let info = parse_list_output(&fixture("no_schemes.txt")).unwrap();
    assert!(info.schemes.is_empty());
    assert_eq!(info.targets, vec!["App"]);
    assert!(!info.parse_warnings.is_empty());
}

#[test]
fn handles_reordered_sections() {
    let info = parse_list_output(&fixture("reordered.txt")).unwrap();
    assert_eq!(info.schemes, vec!["App", "AppTests"]);
    assert_eq!(info.targets, vec!["App", "AppTests"]);
    assert_eq!(info.configurations, vec!["Debug", "Release"]);
}

#[test]
fn unknown_sections_go_to_warnings() {
    let info = parse_list_output(&fixture("extra_sections.txt")).unwrap();
    assert_eq!(info.schemes, vec!["App"]);
    assert!(info.parse_warnings.iter().any(|w| w.contains("Swift Packages")));
}

#[test]
fn malformed_output_returns_error() {
    assert!(parse_list_output(&fixture("malformed.txt")).is_err());
}

#[test]
fn empty_input_returns_error() {
    assert!(parse_list_output("").is_err());
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test --test scheme_parse`
Expected: FAIL — `scheme` module not found.

- [ ] **Step 4: Implement scheme.rs**

```rust
use crate::error::{Error, Result};
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
    let mut info = ListInfo { schemes: vec![], configurations: vec![], targets: vec![], parse_warnings: vec![] };
    let mut current_section: Option<&str> = None;
    let mut found_any_section = false;

    for line in stdout.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() { current_section = None; continue; }
        if trimmed.ends_with(':') && !line.starts_with(char::is_whitespace) {
            let header = &trimmed[..trimmed.len() - 1];
            current_section = match header {
                "Schemes" => { found_any_section = true; Some("schemes") }
                "Targets" => { found_any_section = true; Some("targets") }
                "Build Configurations" => { found_any_section = true; Some("configurations") }
                _ => { info.parse_warnings.push(format!("unknown section: {header}")); None }
            };
            continue;
        }
        if let Some(section) = current_section {
            let entry = trimmed.to_string();
            if entry.is_empty() { continue; }
            if !entry_re.is_match(&entry) {
                info.parse_warnings.push(format!("dropped entry with invalid charset: {entry:?}"));
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

    if !found_any_section { return Err(Error::UnrecognizedListFormat); }
    if info.schemes.is_empty() { info.parse_warnings.push("no schemes found".into()); }
    info.schemes = dedup_sorted(info.schemes);
    info.targets = dedup_sorted(info.targets);
    info.configurations = dedup_sorted(info.configurations);
    Ok(info)
}

fn dedup_sorted(mut v: Vec<String>) -> Vec<String> { v.sort(); v.dedup(); v }
```

- [ ] **Step 5: Add `pub mod scheme;` to lib.rs**

- [ ] **Step 6: Run tests**

Run: `cargo test --test scheme_parse`
Expected: all 6 tests pass.

- [ ] **Step 7: Commit**

```bash
git add -A && git commit -m "feat: add xcodebuild -list output parser with fixtures"
```


---

## Task 4: xcresult JSON Parsing

**Files:**
- Create: `crates/xcode-mcp-core/src/result_bundle.rs` (parser only)
- Modify: `crates/xcode-mcp-core/src/lib.rs` (add `pub mod result_bundle;`)
- Create fixtures: `tests/fixtures/result_bundle/{typical,no_issues,malformed_url,unknown_issue_type,missing_actions}.json`
- Test: `crates/xcode-mcp-core/tests/result_bundle_parse.rs`

**Interfaces:**
- Consumes: `Diagnostic`, `FixIt`, `FixItRange`, `Severity`, `DiagnosticSource`, `ParseResult` from Task 1
- Produces: `parse_build_results(json: &str) -> Result<ParseResult>`

- [ ] **Step 1: Create fixture files**

`tests/fixtures/result_bundle/typical.json`:
```json
{
  "actions": [
    {
      "_results": {
        "issues": [
          {
            "issueType": "BuildError",
            "message": "use of undeclared identifier 'foo'",
            "category": "Swift Compiler Error",
            "documentLocationInCreatingWorkspace": {
              "url": "file:///tmp/App/Sources/App/main.swift#Line=10&Column=5"
            },
            "fixIts": [
              {
                "message": "Replace 'foo' with 'bar'",
                "range": {"startLine": 10, "startColumn": 5, "endLine": 10, "endColumn": 8}
              }
            ]
          },
          {
            "issueType": "BuildError",
            "message": "value of optional type 'String?' must be unwrapped",
            "category": "Swift Compiler Error",
            "documentLocationInCreatingWorkspace": {
              "url": "file:///tmp/App/Sources/App/main.swift#Line=20&Column=12"
            }
          },
          {
            "issueType": "BuildWarning",
            "message": "variable 'x' was never used",
            "category": "Swift Compiler Warning",
            "documentLocationInCreatingWorkspace": {
              "url": "file:///tmp/App/Sources/App/main.swift#Line=5&Column=9"
            }
          },
          {
            "issueType": "BuildWarning",
            "message": "deprecated API usage",
            "category": "Deprecation Warning",
            "documentLocationInCreatingWorkspace": {
              "url": "file:///tmp/App/Sources/App/main.swift#Line=15&Column=1"
            }
          },
          {
            "issueType": "BuildWarning",
            "message": "unused import",
            "category": "Swift Compiler Warning"
          },
          {
            "issueType": "Note",
            "message": "previous declaration here",
            "category": "Swift Compiler",
            "documentLocationInCreatingWorkspace": {
              "url": "file:///tmp/App/Sources/App/main.swift#Line=3&Column=1"
            }
          }
        ]
      }
    }
  ]
}
```

`tests/fixtures/result_bundle/no_issues.json`:
```json
{"actions": [{"_results": {"issues": []}}]}
```

`tests/fixtures/result_bundle/malformed_url.json`:
```json
{"actions": [{"_results": {"issues": [
  {"issueType": "BuildError", "message": "linker failed", "category": "Linker",
   "documentLocationInCreatingWorkspace": {"url": "not-a-valid-url"}}
]}}]}
```

`tests/fixtures/result_bundle/unknown_issue_type.json`:
```json
{"actions": [{"_results": {"issues": [
  {"issueType": "SomeNewType", "message": "future issue", "category": "Unknown"}
]}}]}
```

`tests/fixtures/result_bundle/missing_actions.json`:
```json
{"someOtherKey": "not the expected format"}
```

- [ ] **Step 2: Write failing tests**

Create `crates/xcode-mcp-core/tests/result_bundle_parse.rs`:

```rust
use std::fs;
use xcode_mcp_core::diagnostic::*;
use xcode_mcp_core::result_bundle::*;

fn fixture(name: &str) -> String {
    fs::read_to_string(format!("tests/fixtures/result_bundle/{name}")).unwrap()
}

#[test]
fn parses_typical_with_errors_warnings_notes() {
    let result = parse_build_results(&fixture("typical.json")).unwrap();
    assert_eq!(result.diagnostics.iter().filter(|d| d.severity == Severity::Error).count(), 2);
    assert_eq!(result.diagnostics.iter().filter(|d| d.severity == Severity::Warning).count(), 3);
    assert_eq!(result.diagnostics.iter().filter(|d| d.severity == Severity::Note).count(), 1);
}

#[test]
fn extracts_file_line_column_from_url() {
    let result = parse_build_results(&fixture("typical.json")).unwrap();
    let first = &result.diagnostics[0];
    assert_eq!(first.file.as_deref(), Some("/tmp/App/Sources/App/main.swift"));
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
    let no_loc = result.diagnostics.iter().find(|d| d.message == "unused import").unwrap();
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
    for d in &result.diagnostics { assert_eq!(d.source, DiagnosticSource::Xcresult); }
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test --test result_bundle_parse`
Expected: FAIL — `result_bundle` module not found.

- [ ] **Step 4: Implement result_bundle.rs**

```rust
use crate::diagnostic::{Diagnostic, DiagnosticSource, FixIt, FixItRange, ParseResult, Severity};
use crate::error::{Error, Result};

pub fn parse_build_results(json: &str) -> Result<ParseResult> {
    let root: serde_json::Value = serde_json::from_str(json)?;
    let actions = root.get("actions").and_then(|a| a.as_array())
        .ok_or(Error::UnrecognizedResultFormat)?;
    let mut diagnostics = Vec::new();
    let mut parse_warnings = Vec::new();
    for action in actions {
        if let Some(issues) = action.get("_results").and_then(|r| r.get("issues")).and_then(|i| i.as_array()) {
            for issue in issues {
                if let Some(d) = parse_issue(issue, &mut parse_warnings) {
                    diagnostics.push(d);
                }
            }
        }
    }
    Ok(ParseResult { diagnostics, parse_warnings })
}

fn parse_issue(issue: &serde_json::Value, warnings: &mut Vec<String>) -> Option<Diagnostic> {
    let issue_type = issue.get("issueType").and_then(|t| t.as_str()).unwrap_or("");
    let severity = match issue_type {
        "BuildError" | "AnalyzerError" => Severity::Error,
        "BuildWarning" | "AnalyzerWarning" => Severity::Warning,
        "Note" => Severity::Note,
        other => { warnings.push(format!("unknown issueType: {other}")); Severity::Note }
    };
    let message = issue.get("message").and_then(|m| m.as_str()).unwrap_or("(no message)").to_string();
    let category = issue.get("category").and_then(|c| c.as_str()).map(String::from);
    let (file, line, column) = parse_document_location(issue);
    let fix_its = issue.get("fixIts").and_then(|f| f.as_array())
        .map(|arr| arr.iter().filter_map(parse_fix_it).collect());
    Some(Diagnostic { file, line, column, severity, message, category, fix_its, source: DiagnosticSource::Xcresult })
}

fn parse_document_location(issue: &serde_json::Value) -> (Option<String>, Option<u32>, Option<u32>) {
    let url = issue.get("documentLocationInCreatingWorkspace")
        .and_then(|d| d.get("url")).and_then(|u| u.as_str());
    let Some(url) = url else { return (None, None, None); };
    let (path, fragment) = match url.split_once('#') { Some((p, f)) => (p, f), None => (url, "") };
    let file = if let Some(s) = path.strip_prefix("file://") { Some(s.to_string()) }
               else if path.starts_with('/') { Some(path.to_string()) } else { None };
    (file, extract_num(fragment, "Line="), extract_num(fragment, "Column="))
}

fn extract_num(fragment: &str, key: &str) -> Option<u32> {
    let start = fragment.find(key)?;
    let rest = &fragment[start + key.len()..];
    let end = rest.find(|c: char| !c.is_ascii_digit()).unwrap_or(rest.len());
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
        start_column: val.get("startColumn")?.as_u64()? as u32,
        end_line: val.get("endLine")?.as_u64()? as u32,
        end_column: val.get("endColumn")?.as_u64()? as u32,
    })
}
```

- [ ] **Step 5: Add `pub mod result_bundle;` to lib.rs**

- [ ] **Step 6: Run tests**

Run: `cargo test --test result_bundle_parse`
Expected: all 9 tests pass.

- [ ] **Step 7: Commit**

```bash
git add -A && git commit -m "feat: add xcresulttool build-results JSON parser with fixtures"
```

---

## Task 5: stderr Diagnostic Parsing

**Files:**
- Modify: `crates/xcode-mcp-core/src/diagnostic.rs` (add `parse_stderr` function)
- Create fixtures: `tests/fixtures/stderr/{compiler_error,linker_error,mixed,multiline,no_diagnostics}.txt`
- Test: `crates/xcode-mcp-core/tests/diagnostic_parse.rs`

**Interfaces:**
- Produces: `parse_stderr(stderr: &str) -> ParseResult`

- [ ] **Step 1: Create fixture files**

`tests/fixtures/stderr/compiler_error.txt`:
```
CompileSwiftSources /tmp/Build/Products/Debug/App.swiftmodule (in target 'App' from project 'App')
    cd /tmp/App
    /usr/bin/swiftc ...
/tmp/App/Sources/App/main.swift:10:5: error: use of undeclared identifier 'foo'
    print(foo)
        ^~~
/tmp/App/Sources/App/main.swift:10:5: note: did you mean 'bar'?
** BUILD FAILED **

The following build commands failed:
	CompileSwiftSources normal arm64 (in target 'App' from project 'App')
(1 failure)
```

`tests/fixtures/stderr/linker_error.txt`:
```
Ld /tmp/Build/Products/Debug/App normal (in target 'App' from project 'App')
    cd /tmp/App
    /usr/bin/clang ...
ld: symbol(s) not found for architecture arm64
ld: warning: ignoring duplicate library '-lfoo'
** BUILD FAILED **
```

`tests/fixtures/stderr/mixed.txt`:
```
/tmp/App/Sources/App/main.swift:5:9: warning: variable 'x' was never used
    let x = 42
        ^
/tmp/App/Sources/App/main.swift:10:5: error: use of undeclared identifier 'foo'
/tmp/App/Sources/App/main.swift:20:12: note: previous declaration here
** BUILD FAILED **
```

`tests/fixtures/stderr/multiline.txt`:
```
/tmp/App/Sources/App/main.swift:10:5: error: type 'Any' has no member 'nonexistent'
    let x = Any.nonexistent(
            ~~~ ^~~~~~~~~~~~~
/tmp/App/Sources/App/main.swift:15:1: error: expected expression
```

`tests/fixtures/stderr/no_diagnostics.txt`:
```
Command PhaseScriptExecution failed with a nonzero exit code
** BUILD FAILED **

The following build commands failed:
    PhaseScriptExecution Run\ Script (in target 'App' from project 'App')
(1 failure)
```

- [ ] **Step 2: Write failing tests**

Create `crates/xcode-mcp-core/tests/diagnostic_parse.rs`:

```rust
use std::fs;
use xcode_mcp_core::diagnostic::*;

fn fixture(name: &str) -> String {
    fs::read_to_string(format!("tests/fixtures/stderr/{name}")).unwrap()
}

#[test]
fn parses_compiler_error_with_column() {
    let result = parse_stderr(&fixture("compiler_error.txt"));
    let errors: Vec<_> = result.diagnostics.iter().filter(|d| d.severity == Severity::Error).collect();
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].file.as_deref(), Some("/tmp/App/Sources/App/main.swift"));
    assert_eq!(errors[0].line, Some(10));
    assert_eq!(errors[0].column, Some(5));
    assert!(errors[0].message.contains("undeclared identifier 'foo'"));
    assert_eq!(errors[0].source, DiagnosticSource::Stderr);
}

#[test]
fn parses_note_after_error() {
    let result = parse_stderr(&fixture("compiler_error.txt"));
    let notes: Vec<_> = result.diagnostics.iter().filter(|d| d.severity == Severity::Note).collect();
    assert_eq!(notes.len(), 1);
    assert!(notes[0].message.contains("did you mean"));
}

#[test]
fn parses_linker_error_without_column() {
    let result = parse_stderr(&fixture("linker_error.txt"));
    let errors: Vec<_> = result.diagnostics.iter().filter(|d| d.severity == Severity::Error).collect();
    assert_eq!(errors.len(), 1);
    assert!(errors[0].message.contains("symbol(s) not found"));
    assert_eq!(errors[0].category.as_deref(), Some("Linker"));
}

#[test]
fn parses_linker_warning() {
    let result = parse_stderr(&fixture("linker_error.txt"));
    let warnings: Vec<_> = result.diagnostics.iter().filter(|d| d.severity == Severity::Warning).collect();
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].message.contains("ignoring duplicate library"));
}

#[test]
fn parses_mixed_errors_warnings_notes() {
    let result = parse_stderr(&fixture("mixed.txt"));
    assert_eq!(result.diagnostics.iter().filter(|d| d.severity == Severity::Error).count(), 1);
    assert_eq!(result.diagnostics.iter().filter(|d| d.severity == Severity::Warning).count(), 1);
    assert_eq!(result.diagnostics.iter().filter(|d| d.severity == Severity::Note).count(), 1);
}

#[test]
fn folds_multiline_messages() {
    let result = parse_stderr(&fixture("multiline.txt"));
    let errors: Vec<_> = result.diagnostics.iter().filter(|d| d.severity == Severity::Error).collect();
    assert_eq!(errors.len(), 2);
    assert!(errors[0].message.contains("⏎") || errors[0].message.contains("nonexistent"));
}

#[test]
fn no_diagnostics_returns_empty() {
    let result = parse_stderr(&fixture("no_diagnostics.txt"));
    assert!(result.diagnostics.is_empty());
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test --test diagnostic_parse`
Expected: FAIL — `parse_stderr` not found.

- [ ] **Step 4: Implement parse_stderr — append to diagnostic.rs**

```rust
const STDERR_DIAG_RE: &str =
    r"^(?P<file>[^:\n]+):(?P<line>\d+):(?:(?P<col>\d+):)?\s*(?P<sev>error|warning|note|fatal error):\s*(?P<msg>.*)$";

pub fn parse_stderr(stderr: &str) -> ParseResult {
    let re = match regex::Regex::new(STDERR_DIAG_RE) {
        Ok(r) => r,
        Err(e) => return ParseResult { diagnostics: vec![], parse_warnings: vec![format!("regex failed: {e}")] },
    };
    let mut diagnostics: Vec<Diagnostic> = Vec::new();
    for line in stderr.lines() {
        if let Some(caps) = re.captures(line) {
            let file = caps.name("file").map(|m| m.as_str().to_string());
            let line_num = caps.name("line").and_then(|m| m.as_str().parse::<u32>().ok());
            let column = caps.name("col").and_then(|m| m.as_str().parse::<u32>().ok());
            let sev_str = caps.name("sev").map(|m| m.as_str()).unwrap_or("note");
            let message = caps.name("msg").map(|m| m.as_str().to_string()).unwrap_or_default();
            let severity = match sev_str { "error" | "fatal error" => Severity::Error, "warning" => Severity::Warning, _ => Severity::Note };
            let category = infer_category(&file, &message);
            diagnostics.push(Diagnostic { file, line: line_num, column, severity, message, category, fix_its: None, source: DiagnosticSource::Stderr });
        } else if line.starts_with(char::is_whitespace) && !line.trim().is_empty() {
            if let Some(last) = diagnostics.last_mut() {
                if last.source == DiagnosticSource::Stderr {
                    last.message.push_str(" ⏎ ");
                    last.message.push_str(line.trim());
                }
            }
        }
    }
    ParseResult { diagnostics, parse_warnings: vec![] }
}

fn infer_category(file: &Option<String>, message: &str) -> Option<String> {
    let f = file.as_deref().unwrap_or("");
    let m = message.to_lowercase();
    if f.ends_with(".o") || f.ends_with(".a") || f.ends_with(".dylib") || f.ends_with(".framework")
        || m.starts_with("ld:") || m.contains("linker") {
        return Some("Linker".into());
    }
    if f.is_empty() { return Some("Build System".into()); }
    Some("Compiler".into())
}
```

- [ ] **Step 5: Run tests**

Run: `cargo test --test diagnostic_parse`
Expected: all 7 tests pass.

- [ ] **Step 6: Commit**

```bash
git add -A && git commit -m "feat: add stderr diagnostic line parser with fixtures"
```

---

## Task 6: Diagnostic Merging

**Files:**
- Modify: `crates/xcode-mcp-core/src/diagnostic.rs` (add `merge_diagnostics`)
- Test: `crates/xcode-mcp-core/tests/diagnostic_merge.rs`

**Interfaces:**
- Produces: `MergedDiagnostics { errors, warnings, notes, parse_warnings }`, `merge_diagnostics(xcresult: ParseResult, stderr: ParseResult) -> MergedDiagnostics`

- [ ] **Step 1: Write failing tests**

Create `crates/xcode-mcp-core/tests/diagnostic_merge.rs`:

```rust
use xcode_mcp_core::diagnostic::*;

fn make_diag(sev: Severity, file: &str, line: u32, col: u32, msg: &str, src: DiagnosticSource) -> Diagnostic {
    Diagnostic { file: Some(file.into()), line: Some(line), column: Some(col), severity: sev,
        message: msg.into(), category: None, fix_its: None, source: src }
}

#[test]
fn merges_xcresult_and_stderr() {
    let xcresult = ParseResult { diagnostics: vec![make_diag(Severity::Error, "/a.swift", 10, 5, "foo", DiagnosticSource::Xcresult)], parse_warnings: vec![] };
    let stderr = ParseResult { diagnostics: vec![make_diag(Severity::Warning, "/b.swift", 1, 1, "bar", DiagnosticSource::Stderr)], parse_warnings: vec![] };
    let merged = merge_diagnostics(xcresult, stderr);
    assert_eq!(merged.errors.len(), 1);
    assert_eq!(merged.warnings.len(), 1);
    assert!(merged.notes.is_empty());
}

#[test]
fn dedups_same_diagnostic_keeps_xcresult() {
    let xcresult = ParseResult { diagnostics: vec![make_diag(Severity::Error, "/a.swift", 10, 5, "undeclared identifier 'foo'", DiagnosticSource::Xcresult)], parse_warnings: vec![] };
    let stderr = ParseResult { diagnostics: vec![make_diag(Severity::Error, "/a.swift", 10, 5, "undeclared identifier 'foo'", DiagnosticSource::Stderr)], parse_warnings: vec![] };
    let merged = merge_diagnostics(xcresult, stderr);
    assert_eq!(merged.errors.len(), 1);
    assert_eq!(merged.errors[0].source, DiagnosticSource::Xcresult);
}

#[test]
fn normalizes_whitespace_for_dedup() {
    let xcresult = ParseResult { diagnostics: vec![make_diag(Severity::Error, "/a.swift", 10, 5, "undeclared  identifier   'foo'", DiagnosticSource::Xcresult)], parse_warnings: vec![] };
    let stderr = ParseResult { diagnostics: vec![make_diag(Severity::Error, "/a.swift", 10, 5, "undeclared identifier 'foo'", DiagnosticSource::Stderr)], parse_warnings: vec![] };
    let merged = merge_diagnostics(xcresult, stderr);
    assert_eq!(merged.errors.len(), 1);
}

#[test]
fn sorts_by_file_then_line_then_column() {
    let xcresult = ParseResult { diagnostics: vec![
        make_diag(Severity::Error, "/b.swift", 5, 1, "e2", DiagnosticSource::Xcresult),
        make_diag(Severity::Error, "/a.swift", 20, 1, "e1", DiagnosticSource::Xcresult),
        make_diag(Severity::Error, "/a.swift", 10, 1, "e0", DiagnosticSource::Xcresult),
    ], parse_warnings: vec![] };
    let merged = merge_diagnostics(xcresult, ParseResult::default());
    assert_eq!(merged.errors[0].line, Some(10));
    assert_eq!(merged.errors[1].line, Some(20));
    assert_eq!(merged.errors[2].file.as_deref(), Some("/b.swift"));
}

#[test]
fn combines_parse_warnings() {
    let xcresult = ParseResult { diagnostics: vec![], parse_warnings: vec!["xc warning".into()] };
    let stderr = ParseResult { diagnostics: vec![], parse_warnings: vec!["stderr warning".into()] };
    let merged = merge_diagnostics(xcresult, stderr);
    assert!(merged.parse_warnings.contains(&"xc warning".to_string()));
    assert!(merged.parse_warnings.contains(&"stderr warning".to_string()));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --test diagnostic_merge`
Expected: FAIL — `merge_diagnostics` not found.

- [ ] **Step 3: Implement merge_diagnostics — append to diagnostic.rs**

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MergedDiagnostics {
    pub errors: Vec<Diagnostic>,
    pub warnings: Vec<Diagnostic>,
    pub notes: Vec<Diagnostic>,
    pub parse_warnings: Vec<String>,
}

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
        if seen.contains(&key) { continue; }
        seen.push(key);
        deduped.push(d.clone());
    }
    let mut errors: Vec<_> = deduped.iter().filter(|d| d.severity == Severity::Error).cloned().collect();
    let mut warnings: Vec<_> = deduped.iter().filter(|d| d.severity == Severity::Warning).cloned().collect();
    let mut notes: Vec<_> = deduped.iter().filter(|d| d.severity == Severity::Note).cloned().collect();
    let cmp = |a: &Diagnostic, b: &Diagnostic| {
        (a.file.as_deref().unwrap_or(""), a.line.unwrap_or(0), a.column.unwrap_or(0))
            .cmp(&(b.file.as_deref().unwrap_or(""), b.line.unwrap_or(0), b.column.unwrap_or(0)))
    };
    errors.sort_by(cmp);
    warnings.sort_by(cmp);
    notes.sort_by(cmp);
    MergedDiagnostics { errors, warnings, notes, parse_warnings }
}

fn normalize_message(msg: &str) -> String {
    msg.trim().split_whitespace().collect::<Vec<_>>().join(" ")
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test --test diagnostic_merge`
Expected: all 5 tests pass.

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat: add diagnostic merging with dedup and sorting"
```

---

## Task 7: Build Store

**Files:**
- Create: `crates/xcode-mcp-core/src/store.rs`
- Modify: `crates/xcode-mcp-core/src/lib.rs` (add `pub mod store;`)
- Test: `crates/xcode-mcp-core/tests/store.rs`

**Interfaces:**
- Produces: `BuildStatus` enum, `BuildRecord` struct, `BuildStore` with `new(cap)`, `push(record)`, `most_recent()`, `get(build_id)`

- [ ] **Step 1: Write failing tests**

Create `crates/xcode-mcp-core/tests/store.rs`:

```rust
use std::path::PathBuf;
use xcode_mcp_core::store::*;

fn make_record(id: &str) -> BuildRecord {
    BuildRecord {
        build_id: id.into(), status: BuildStatus::Failed, exit_code: Some(1), duration_secs: 1.0,
        project_or_workspace: PathBuf::from("/tmp/App.xcodeproj"), scheme: "App".into(),
        xcresult_path: PathBuf::from(format!("/tmp/{id}.xcresult")),
        log_path: PathBuf::from(format!("/tmp/{id}.log")),
        result_bundle_written: true, error_count: 1, warning_count: 0, stderr_excerpt: None,
        created_at: std::time::SystemTime::now(),
    }
}

#[test]
fn push_and_get() {
    let store = BuildStore::new(32);
    store.push(make_record("build-1"));
    assert_eq!(store.get("build-1").unwrap().build_id, "build-1");
}

#[test]
fn most_recent_returns_last_pushed() {
    let store = BuildStore::new(32);
    store.push(make_record("build-1"));
    store.push(make_record("build-2"));
    assert_eq!(store.most_recent().unwrap().build_id, "build-2");
}

#[test]
fn returns_none_when_empty() {
    let store = BuildStore::new(32);
    assert!(store.most_recent().is_none());
    assert!(store.get("nope").is_none());
}

#[test]
fn evicts_oldest_when_full() {
    let store = BuildStore::new(2);
    store.push(make_record("build-1"));
    store.push(make_record("build-2"));
    store.push(make_record("build-3"));
    assert!(store.get("build-1").is_none());
    assert!(store.get("build-2").is_some());
    assert!(store.get("build-3").is_some());
    assert_eq!(store.most_recent().unwrap().build_id, "build-3");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --test store`
Expected: FAIL — `store` module not found.

- [ ] **Step 3: Implement store.rs**

```rust
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::SystemTime;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub enum BuildStatus { Succeeded, Failed, TimedOut, Canceled, Unknown }

#[derive(Debug, Clone)]
pub struct BuildRecord {
    pub build_id: String,
    pub status: BuildStatus,
    pub exit_code: Option<i32>,
    pub duration_secs: f64,
    pub project_or_workspace: PathBuf,
    pub scheme: String,
    pub xcresult_path: PathBuf,
    pub log_path: PathBuf,
    pub result_bundle_written: bool,
    pub error_count: u32,
    pub warning_count: u32,
    pub stderr_excerpt: Option<String>,
    pub created_at: SystemTime,
}

pub struct BuildStore {
    records: Mutex<VecDeque<BuildRecord>>,
    cap: usize,
}

impl BuildStore {
    pub fn new(cap: usize) -> Self {
        Self { records: Mutex::new(VecDeque::with_capacity(cap)), cap }
    }
    pub fn push(&self, record: BuildRecord) {
        let mut records = self.records.lock().unwrap();
        if records.len() >= self.cap { records.pop_front(); }
        records.push_back(record);
    }
    pub fn most_recent(&self) -> Option<BuildRecord> {
        self.records.lock().unwrap().back().cloned()
    }
    pub fn get(&self, build_id: &str) -> Option<BuildRecord> {
        self.records.lock().unwrap().iter().find(|r| r.build_id == build_id).cloned()
    }
}
```

- [ ] **Step 4: Add `pub mod store;` to lib.rs**

- [ ] **Step 5: Run tests**

Run: `cargo test --test store`
Expected: all 4 tests pass.

- [ ] **Step 6: Commit**

```bash
git add -A && git commit -m "feat: add build store with ring buffer and lookup"
```

---

## Task 8: xcode.rs — Command Builders

**Files:**
- Create: `crates/xcode-mcp-core/src/xcode.rs` (command builders only)
- Modify: `crates/xcode-mcp-core/src/lib.rs` (add `pub mod xcode;`)
- Test: `crates/xcode-mcp-core/tests/xcode_commands.rs`

**Interfaces:**
- Produces: `build_list_schemes_command(path: &Path) -> Command`, `build_xcodebuild_command(...)` , `build_xcresulttool_command(path: &Path) -> Command`

- [ ] **Step 1: Write failing tests**

Create `crates/xcode-mcp-core/tests/xcode_commands.rs`:

```rust
use std::path::PathBuf;
use xcode_mcp_core::xcode::*;

fn args_of(cmd: tokio::process::Command) -> Vec<String> {
    let std_cmd: std::process::Command = cmd.into();
    std_cmd.get_args().map(|s| s.to_str().unwrap().to_string()).collect()
}

#[test]
fn list_schemes_command_for_project() {
    let args = args_of(build_list_schemes_command(&PathBuf::from("/tmp/App.xcodeproj")));
    assert!(args.contains(&"xcodebuild".into()));
    assert!(args.contains(&"-list".into()));
    assert!(args.contains(&"-project".into()));
    assert!(args.contains(&"/tmp/App.xcodeproj".into()));
}

#[test]
fn list_schemes_command_for_workspace() {
    let args = args_of(build_list_schemes_command(&PathBuf::from("/tmp/App.xcworkspace")));
    assert!(args.contains(&"-workspace".into()));
    assert!(args.contains(&"/tmp/App.xcworkspace".into()));
}

#[test]
fn build_command_has_required_flags() {
    let cmd = build_xcodebuild_command(
        &PathBuf::from("/tmp/App.xcodeproj"), "App", "build",
        Some("Debug"), Some("generic/platform=iOS"),
        &PathBuf::from("/tmp/result.xcresult"), &PathBuf::from("/tmp/dd"),
    );
    let args = args_of(cmd);
    assert!(args.contains(&"-scheme".into()));
    assert!(args.contains(&"App".into()));
    assert!(args.contains(&"-configuration".into()));
    assert!(args.contains(&"Debug".into()));
    assert!(args.contains(&"-destination".into()));
    assert!(args.contains(&"generic/platform=iOS".into()));
    assert!(args.contains(&"-resultBundlePath".into()));
    assert!(args.contains(&"-derivedDataPath".into()));
    assert!(args.contains(&"-quiet".into()));
    assert!(args.contains(&"build".into()));
}

#[test]
fn build_command_clean_plus_build_passes_two_actions() {
    let cmd = build_xcodebuild_command(
        &PathBuf::from("/tmp/App.xcodeproj"), "App", "clean+build",
        None, None, &PathBuf::from("/tmp/r.xcresult"), &PathBuf::from("/tmp/dd"),
    );
    let args = args_of(cmd);
    assert!(args.contains(&"clean".into()));
    assert!(args.contains(&"build".into()));
    assert!(!args.contains(&"-configuration".into()));
    assert!(!args.contains(&"-destination".into()));
}

#[test]
fn build_command_uses_workspace_for_xcworkspace() {
    let cmd = build_xcodebuild_command(
        &PathBuf::from("/tmp/App.xcworkspace"), "App", "build",
        None, None, &PathBuf::from("/tmp/r.xcresult"), &PathBuf::from("/tmp/dd"),
    );
    let args = args_of(cmd);
    assert!(args.contains(&"-workspace".into()));
    assert!(!args.contains(&"-project".into()));
}

#[test]
fn xcresulttool_command_format() {
    let args = args_of(build_xcresulttool_command(&PathBuf::from("/tmp/r.xcresult")));
    assert!(args.contains(&"xcresulttool".into()));
    assert!(args.contains(&"get".into()));
    assert!(args.contains(&"build-results".into()));
    assert!(args.contains(&"--format".into()));
    assert!(args.contains(&"json".into()));
    assert!(args.contains(&"--path".into()));
    assert!(args.contains(&"/tmp/r.xcresult".into()));
}

#[test]
fn no_shell_invocation() {
    let cmd = build_list_schemes_command(&PathBuf::from("/tmp/App.xcodeproj"));
    let std_cmd: std::process::Command = cmd.into();
    assert_eq!(std_cmd.get_program().to_str().unwrap(), "xcrun");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --test xcode_commands`
Expected: FAIL — `xcode` module not found.

- [ ] **Step 3: Implement command builders in xcode.rs**

```rust
use std::path::Path;
use tokio::process::Command;

pub fn build_list_schemes_command(project_or_workspace: &Path) -> Command {
    let mut cmd = Command::new("xcrun");
    cmd.arg("xcodebuild").arg("-list");
    if project_or_workspace.to_string_lossy().ends_with(".xcworkspace") {
        cmd.arg("-workspace").arg(project_or_workspace);
    } else {
        cmd.arg("-project").arg(project_or_workspace);
    }
    cmd
}

#[allow(clippy::too_many_arguments)]
pub fn build_xcodebuild_command(
    project_or_workspace: &Path, scheme: &str, action: &str,
    configuration: Option<&str>, destination: Option<&str>,
    xcresult_path: &Path, derived_data_path: &Path,
) -> Command {
    let mut cmd = Command::new("xcrun");
    cmd.arg("xcodebuild").arg("-scheme").arg(scheme);
    if project_or_workspace.to_string_lossy().ends_with(".xcworkspace") {
        cmd.arg("-workspace").arg(project_or_workspace);
    } else {
        cmd.arg("-project").arg(project_or_workspace);
    }
    if let Some(cfg) = configuration { cmd.arg("-configuration").arg(cfg); }
    if let Some(dest) = destination { cmd.arg("-destination").arg(dest); }
    cmd.arg("-resultBundlePath").arg(xcresult_path)
       .arg("-derivedDataPath").arg(derived_data_path)
       .arg("-quiet");
    match action {
        "clean+build" => { cmd.arg("clean").arg("build"); }
        "clean" => { cmd.arg("clean"); }
        _ => { cmd.arg("build"); }
    }
    cmd
}

pub fn build_xcresulttool_command(xcresult_path: &Path) -> Command {
    let mut cmd = Command::new("xcrun");
    cmd.arg("xcresulttool").arg("get").arg("build-results")
       .arg("--format").arg("json").arg("--path").arg(xcresult_path);
    cmd
}
```

- [ ] **Step 4: Add `pub mod xcode;` to lib.rs**

- [ ] **Step 5: Run tests**

Run: `cargo test --test xcode_commands`
Expected: all 7 tests pass.

- [ ] **Step 6: Commit**

```bash
git add -A && git commit -m "feat: add xcodebuild/xcresulttool command builders"
```

---

## Task 9: xcode.rs — Process Supervisor

**Files:**
- Modify: `crates/xcode-mcp-core/src/xcode.rs` (add `run_supervised`)
- Test: `crates/xcode-mcp-core/tests/process_supervisor.rs`

**Interfaces:**
- Produces: `SupervisedResult { exit_code, stdout, stderr, timed_out }`, `async fn run_supervised(cmd: Command, timeout_secs: u32, log_file: Option<&Path>) -> Result<SupervisedResult>`

- [ ] **Step 1: Write failing tests (using `sleep`/`echo` as controllable subprocesses)**

Create `crates/xcode-mcp-core/tests/process_supervisor.rs`:

```rust
use std::time::Duration;
use tokio::process::Command;
use xcode_mcp_core::xcode::*;

#[tokio::test]
async fn runs_command_to_completion() {
    let mut cmd = Command::new("echo"); cmd.arg("hello");
    let result = run_supervised(cmd, 10, None).await.unwrap();
    assert!(!result.timed_out);
    assert_eq!(result.exit_code, Some(0));
    assert_eq!(String::from_utf8_lossy(&result.stdout).trim(), "hello");
}

#[tokio::test]
async fn captures_stderr() {
    let mut cmd = Command::new("sh"); cmd.args(["-c", "echo err >&2; exit 3"]);
    let result = run_supervised(cmd, 10, None).await.unwrap();
    assert_eq!(result.exit_code, Some(3));
    assert!(String::from_utf8_lossy(&result.stderr).contains("err"));
}

#[tokio::test]
async fn times_out_and_kills() {
    let mut cmd = Command::new("sleep"); cmd.arg("30");
    let start = tokio::time::Instant::now();
    let result = run_supervised(cmd, 1, None).await.unwrap();
    assert!(result.timed_out);
    assert_eq!(result.exit_code, None);
    assert!(start.elapsed() < Duration::from_secs(10));
}

#[tokio::test]
async fn writes_to_log_file_when_provided() {
    let log_path = tempfile::NamedTempFile::new().unwrap().into_temp_path().keep().unwrap();
    let mut cmd = Command::new("echo"); cmd.arg("logged");
    let result = run_supervised(cmd, 10, Some(&log_path)).await.unwrap();
    assert_eq!(result.exit_code, Some(0));
    assert!(std::fs::read_to_string(&log_path).unwrap().contains("logged"));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --test process_supervisor`
Expected: FAIL — `run_supervised` not found.

- [ ] **Step 3: Implement run_supervised — append to xcode.rs**

```rust
use crate::error::{Error, Result};
use std::path::Path;
use std::sync::Arc;
use tokio::io::AsyncReadExt;
use tokio::sync::Mutex;

pub struct SupervisedResult {
    pub exit_code: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub timed_out: bool,
}

pub async fn run_supervised(
    mut cmd: Command, timeout_secs: u32, log_file: Option<&Path>,
) -> Result<SupervisedResult> {
    unsafe { cmd.pre_exec(|| { libc::setsid(); Ok(()) }); }
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());
    cmd.kill_on_drop(true);

    let mut child = cmd.spawn().map_err(|e| Error::XcodeSpawnFailed(e.to_string()))?;
    let pid = child.id().expect("child pid");
    let mut stdout = child.stdout.take().expect("stdout piped");
    let mut stderr = child.stderr.take().expect("stderr piped");
    let stdout_buf = Arc::new(Mutex::new(Vec::new()));
    let stderr_buf = Arc::new(Mutex::new(Vec::new()));
    let log_writer = if let Some(path) = log_file {
        Some(Arc::new(Mutex::new(std::fs::File::create(path)?)))
    } else { None };

    let stdout_task = {
        let buf = stdout_buf.clone(); let log = log_writer.clone();
        tokio::spawn(async move {
            let mut chunk = [0u8; 4096];
            loop {
                match stdout.read(&mut chunk).await {
                    Ok(0) => break, Ok(n) => {
                        let data = &chunk[..n];
                        buf.lock().await.extend_from_slice(data);
                        if let Some(ref log) = log {
                            use std::io::Write;
                            let _ = log.lock().await.write_all(data);
                        }
                    }, Err(_) => break,
                }
            }
        })
    };
    let stderr_task = {
        let buf = stderr_buf.clone(); let log = log_writer.clone();
        tokio::spawn(async move {
            let mut chunk = [0u8; 4096];
            loop {
                match stderr.read(&mut chunk).await {
                    Ok(0) => break, Ok(n) => {
                        let data = &chunk[..n];
                        buf.lock().await.extend_from_slice(data);
                        if let Some(ref log) = log {
                            use std::io::Write;
                            let _ = log.lock().await.write_all(data);
                        }
                    }, Err(_) => break,
                }
            }
        })
    };

    let wait_result = tokio::time::timeout(
        std::time::Duration::from_secs(timeout_secs as u64), child.wait()
    ).await;

    match wait_result {
        Ok(Ok(status)) => {
            let _ = stdout_task.await; let _ = stderr_task.await;
            Ok(SupervisedResult {
                exit_code: status.code(), timed_out: false,
                stdout: stdout_buf.lock().await.clone(),
                stderr: stderr_buf.lock().await.clone(),
            })
        }
        Ok(Err(e)) => Err(Error::Internal(format!("wait failed: {e}"))),
        Err(_) => {
            kill_process_group(pid, libc::SIGTERM);
            if tokio::time::timeout(std::time::Duration::from_secs(5), child.wait()).await.is_err() {
                kill_process_group(pid, libc::SIGKILL);
                let _ = child.wait().await;
            }
            let _ = stdout_task.await; let _ = stderr_task.await;
            Ok(SupervisedResult {
                exit_code: None, timed_out: true,
                stdout: stdout_buf.lock().await.clone(),
                stderr: stderr_buf.lock().await.clone(),
            })
        }
    }
}

fn kill_process_group(pid: u32, sig: i32) {
    unsafe { libc::kill(-(pid as i32), sig); }
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test --test process_supervisor`
Expected: all 4 tests pass (timeout test may take ~1-6s).

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat: add process supervisor with timeout and process-group kill"
```


---

## Task 10: scheme.rs — Async `list_schemes` Invocation

**Files:**
- Modify: `crates/xcode-mcp-core/src/scheme.rs` (add `list_schemes` async function)
- Test: deferred to Task 16 (live integration tests)

**Interfaces:**
- Produces: `async fn list_schemes(project_or_workspace: &str, root: &Path) -> Result<ListInfo>`

- [ ] **Step 1: Implement list_schemes — append to scheme.rs**

```rust
use std::path::Path;
use crate::security::validate_project_or_workspace;
use crate::xcode::{build_list_schemes_command, run_supervised};

pub async fn list_schemes(project_or_workspace: &str, root: &Path) -> Result<ListInfo> {
    let validated_path = validate_project_or_workspace(project_or_workspace, root)?;
    let cmd = build_list_schemes_command(&validated_path);
    let result = run_supervised(cmd, 30, None).await?;
    if result.timed_out {
        return Err(crate::error::Error::XcodeListFailed { exit_code: None, stderr_excerpt: "timed out".into() });
    }
    if result.exit_code != Some(0) {
        let excerpt = String::from_utf8_lossy(&result.stderr).chars().take(2000).collect();
        return Err(crate::error::Error::XcodeListFailed { exit_code: result.exit_code, stderr_excerpt: excerpt });
    }
    let stdout = String::from_utf8_lossy(&result.stdout);
    parse_list_output(&stdout)
}
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo build -p xcode-mcp-core`
Expected: compiles without errors.

- [ ] **Step 3: Commit**

```bash
git add -A && git commit -m "feat: add async list_schemes invocation wrapper"
```

---

## Task 11: result_bundle.rs — Async `read_build_results` Invocation

**Files:**
- Modify: `crates/xcode-mcp-core/src/result_bundle.rs` (add `read_build_results`)
- Test: deferred to Task 16

**Interfaces:**
- Produces: `async fn read_build_results(xcresult_path: &Path) -> Result<ParseResult>`

- [ ] **Step 1: Implement read_build_results — append to result_bundle.rs**

```rust
use std::path::Path;
use crate::error::{Error, Result};
use crate::xcode::{build_xcresulttool_command, run_supervised};

pub async fn read_build_results(xcresult_path: &Path) -> Result<ParseResult> {
    let cmd = build_xcresulttool_command(xcresult_path);
    let result = run_supervised(cmd, 60, None).await?;
    if result.timed_out {
        return Err(Error::XcresulttoolFailed { exit_code: None, stderr_excerpt: "timed out".into() });
    }
    if result.exit_code != Some(0) {
        let excerpt = String::from_utf8_lossy(&result.stderr).chars().take(2000).collect();
        return Err(Error::XcresulttoolFailed { exit_code: result.exit_code, stderr_excerpt: excerpt });
    }
    let stdout = String::from_utf8_lossy(&result.stdout);
    if stdout.trim().is_empty() {
        return Err(Error::XcresulttoolFailed { exit_code: result.exit_code, stderr_excerpt: "empty output".into() });
    }
    parse_build_results(&stdout)
}
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo build -p xcode-mcp-core`
Expected: compiles without errors.

- [ ] **Step 3: Commit**

```bash
git add -A && git commit -m "feat: add async read_build_results invocation wrapper"
```

---

## Task 12: diagnostic.rs — Hybrid `load_diagnostics` Loader

**Files:**
- Modify: `crates/xcode-mcp-core/src/diagnostic.rs` (add `load_diagnostics`)
- Modify: `crates/xcode-mcp-core/src/lib.rs` (re-exports)
- Test: `crates/xcode-mcp-core/tests/load_diagnostics.rs`

**Interfaces:**
- Produces: `DiagnosticSourceLabel` enum, `DiagnosticOutput` struct, `async fn load_diagnostics(build_id: Option<&str>, store: &BuildStore, result_dir: &Path, log_dir: &Path) -> Result<DiagnosticOutput>`

- [ ] **Step 1: Write failing tests**

Create `crates/xcode-mcp-core/tests/load_diagnostics.rs`:

```rust
use std::fs;
use std::path::PathBuf;
use tempfile::tempdir;
use xcode_mcp_core::diagnostic::*;
use xcode_mcp_core::store::*;

fn make_record(id: &str, xcresult: &PathBuf, log: &PathBuf, status: BuildStatus) -> BuildRecord {
    BuildRecord {
        build_id: id.into(), status, exit_code: Some(1), duration_secs: 1.0,
        project_or_workspace: PathBuf::from("/tmp/App.xcodeproj"), scheme: "App".into(),
        xcresult_path: xcresult.clone(), log_path: log.clone(),
        result_bundle_written: true, error_count: 0, warning_count: 0,
        stderr_excerpt: None, created_at: std::time::SystemTime::now(),
    }
}

#[tokio::test]
async fn returns_none_source_for_succeeded_build() {
    let dir = tempdir().unwrap();
    let result_dir = dir.path().join("results");
    let log_dir = dir.path().join("logs");
    fs::create_dir_all(&result_dir).unwrap();
    fs::create_dir_all(&log_dir).unwrap();
    let xcresult = result_dir.join("b1.xcresult");
    let log = log_dir.join("b1.log");
    fs::write(&log, "").unwrap();
    let store = BuildStore::new(32);
    store.push(make_record("b1", &xcresult, &log, BuildStatus::Succeeded));
    let output = load_diagnostics(Some("b1"), &store, &result_dir, &log_dir).await.unwrap();
    assert_eq!(output.build_id, "b1");
    assert!(matches!(output.source, DiagnosticSourceLabel::None));
    assert!(output.merged.errors.is_empty());
}

#[tokio::test]
async fn errors_when_build_not_found() {
    let dir = tempdir().unwrap();
    let result_dir = dir.path().join("results");
    let log_dir = dir.path().join("logs");
    fs::create_dir_all(&result_dir).unwrap();
    fs::create_dir_all(&log_dir).unwrap();
    let store = BuildStore::new(32);
    assert!(load_diagnostics(Some("nonexistent"), &store, &result_dir, &log_dir).await.is_err());
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --test load_diagnostics`
Expected: FAIL — `load_diagnostics` not found.

- [ ] **Step 3: Implement load_diagnostics — append to diagnostic.rs**

```rust
use crate::error::{Error, Result};
use crate::result_bundle::read_build_results;
use crate::store::{BuildStatus, BuildStore};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticSourceLabel { Xcresult, StderrOnly, None }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagnosticOutput {
    pub build_id: String,
    pub build_status: BuildStatus,
    pub source: DiagnosticSourceLabel,
    pub merged: MergedDiagnostics,
}

pub async fn load_diagnostics(
    build_id: Option<&str>, store: &BuildStore,
    result_dir: &Path, log_dir: &Path,
) -> Result<DiagnosticOutput> {
    let record = match build_id {
        Some(id) => store.get(id).or_else(|| {
            let xcresult_path = result_dir.join(format!("{id}.xcresult"));
            if xcresult_path.exists() {
                Some(BuildRecord {
                    build_id: id.to_string(), status: BuildStatus::Unknown,
                    exit_code: None, duration_secs: 0.0,
                    project_or_workspace: Path::new("").into(), scheme: String::new(),
                    xcresult_path, log_path: log_dir.join(format!("{id}.log")),
                    result_bundle_written: true, error_count: 0, warning_count: 0,
                    stderr_excerpt: None, created_at: std::time::SystemTime::now(),
                })
            } else { None }
        }).ok_or_else(|| Error::BuildNotFound(id.to_string()))?,
        None => store.most_recent().ok_or_else(|| Error::NoBuildAvailable {
            hint: "no builds in session".into()
        })?,
    };

    let mut parse_warnings = Vec::new();
    let xcresult_result = if record.result_bundle_written && record.xcresult_path.exists() {
        match read_build_results(&record.xcresult_path).await {
            Ok(r) => Some(r),
            Err(e) => { parse_warnings.push(format!("xcresult parse failed: {e}")); None }
        }
    } else { None };

    let stderr_result = if record.log_path.exists() {
        let log_contents = std::fs::read_to_string(&record.log_path).unwrap_or_default();
        Some(parse_stderr(&log_contents))
    } else { None };

    let xcresult = xcresult_result.unwrap_or_default();
    let stderr = stderr_result.unwrap_or_default();
    let merged = merge_diagnostics(xcresult, stderr);

    let source = if record.result_bundle_written && record.xcresult_path.exists() && xcresult_result.is_some() {
        DiagnosticSourceLabel::Xcresult
    } else if stderr_result.is_some() {
        DiagnosticSourceLabel::StderrOnly
    } else {
        DiagnosticSourceLabel::None
    };

    Ok(DiagnosticOutput {
        build_id: record.build_id, build_status: record.status, source, merged,
    })
}
```

Note: `BuildRecord` must be imported. Add `use crate::store::BuildRecord;` at the top of the appended block, or use the fully qualified path `crate::store::BuildRecord`.

- [ ] **Step 4: Update lib.rs re-exports** — add:

```rust
pub use diagnostic::{DiagnosticOutput, DiagnosticSourceLabel, MergedDiagnostics};
```

- [ ] **Step 5: Run tests**

Run: `cargo test --test load_diagnostics`
Expected: both tests pass.

- [ ] **Step 6: Commit**

```bash
git add -A && git commit -m "feat: add hybrid load_diagnostics with xcresult + stderr merge"
```

---

## Task 13: xcode.rs — Full `run_build` Lifecycle

**Files:**
- Modify: `crates/xcode-mcp-core/src/xcode.rs` (add `run_build`)
- Modify: `crates/xcode-mcp-core/src/lib.rs` (re-exports)
- Test: deferred to Task 16

**Interfaces:**
- Produces: `BuildParams` struct, `BuildOutput` struct, `async fn run_build(params: BuildParams, root: &Path, result_dir: &Path, log_dir: &Path, store: &BuildStore) -> Result<BuildOutput>`

- [ ] **Step 1: Implement run_build — append to xcode.rs**

```rust
use crate::diagnostic::load_diagnostics;
use crate::security::{
    validate_action, validate_configuration, validate_destination,
    validate_project_or_workspace, validate_scheme, validate_timeout,
};
use crate::store::{BuildRecord, BuildStatus, BuildStore};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tokio::sync::Semaphore;

static BUILD_PERMIT: Semaphore = Semaphore::const_new(1);

#[derive(Debug, Clone, Deserialize)]
pub struct BuildParams {
    pub project_or_workspace: String,
    pub scheme: String,
    pub action: Option<String>,
    pub configuration: Option<String>,
    pub destination: Option<String>,
    pub timeout_secs: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BuildOutput {
    pub build_id: String,
    pub status: String,
    pub exit_code: Option<i32>,
    pub duration_secs: f64,
    pub xcresult_path: String,
    pub log_path: String,
    pub result_bundle_written: bool,
    pub error_count: u32,
    pub warning_count: u32,
    pub truncated_stderr_excerpt: Option<String>,
}

pub async fn run_build(
    params: BuildParams, root: &Path, result_dir: &Path, log_dir: &Path,
    store: &BuildStore,
) -> Result<BuildOutput> {
    // 1. Validate inputs (before acquiring permit)
    let validated_path = validate_project_or_workspace(&params.project_or_workspace, root)?;
    let scheme = validate_scheme(&params.scheme)?;
    let action = validate_action(params.action.as_deref().unwrap_or("build"))?;
    let configuration = match params.configuration.as_deref() {
        Some(c) => Some(validate_configuration(c)?), None => None,
    };
    let destination = match params.destination.as_deref() {
        Some(d) => Some(validate_destination(d)?), None => None,
    };
    let timeout_secs = validate_timeout(params.timeout_secs)?;

    // 2. Reserve build_id + paths
    let build_id = uuid::Uuid::new_v4().to_string();
    let xcresult_path = result_dir.join(format!("{build_id}.xcresult"));
    let derived_data_path = result_dir.join("DerivedData").join(&build_id);
    let log_path = log_dir.join(format!("{build_id}.log"));
    std::fs::File::create(&log_path)?;

    // 3. Acquire global build permit (serialized execution)
    let _permit = BUILD_PERMIT.acquire().await
        .map_err(|e| crate::error::Error::Internal(format!("semaphore closed: {e}")))?;

    // 4. Build command
    let cmd = build_xcodebuild_command(
        &validated_path, &scheme, &action,
        configuration.as_deref(), destination.as_deref(),
        &xcresult_path, &derived_data_path,
    );

    // 5. Run supervised
    let start = std::time::Instant::now();
    let result = run_supervised(cmd, timeout_secs, Some(&log_path)).await?;
    let duration = start.elapsed().as_secs_f64();

    // 6. Check result bundle
    let result_bundle_written = xcresult_path.exists();

    // 7. Determine status
    let (status, exit_code) = if result.timed_out {
        ("TimedOut".to_string(), None)
    } else if result.exit_code == Some(0) {
        ("Succeeded".to_string(), result.exit_code)
    } else {
        ("Failed".to_string(), result.exit_code)
    };

    // 8. Truncated stderr excerpt (last 2KB)
    let truncated_stderr_excerpt = if (status == "Failed" || status == "TimedOut") && !result_bundle_written {
        let s = String::from_utf8_lossy(&result.stderr);
        let chars: Vec<char> = s.chars().rev().take(2048).collect();
        Some(chars.into_iter().rev().collect())
    } else { None };

    // 9. Clean up derived data
    if derived_data_path.exists() {
        if let Err(e) = std::fs::remove_dir_all(&derived_data_path) {
            tracing::warn!("failed to clean derived data: {e}");
        }
    }

    // 10. Register in store
    let build_status = match status.as_str() {
        "Succeeded" => BuildStatus::Succeeded,
        "TimedOut" => BuildStatus::TimedOut,
        _ => BuildStatus::Failed,
    };
    store.push(BuildRecord {
        build_id: build_id.clone(), status: build_status.clone(), exit_code,
        duration_secs: duration, project_or_workspace: validated_path.clone(),
        scheme: scheme.clone(), xcresult_path: xcresult_path.clone(),
        log_path: log_path.clone(), result_bundle_written,
        error_count: 0, warning_count: 0,
        stderr_excerpt: truncated_stderr_excerpt.clone(),
        created_at: std::time::SystemTime::now(),
    });

    // 11. Best-effort: compute error/warning counts
    let (error_count, warning_count) = if result_bundle_written {
        match load_diagnostics(Some(&build_id), store, result_dir, log_dir).await {
            Ok(o) => (o.merged.errors.len() as u32, o.merged.warnings.len() as u32),
            Err(e) => { tracing::warn!("failed to compute diagnostic counts: {e}"); (0, 0) }
        }
    } else { (0, 0) };

    Ok(BuildOutput {
        build_id, status, exit_code, duration_secs: duration,
        xcresult_path: xcresult_path.to_string_lossy().into_owned(),
        log_path: log_path.to_string_lossy().into_owned(),
        result_bundle_written, error_count, warning_count, truncated_stderr_excerpt,
    })
}
```

- [ ] **Step 2: Update lib.rs re-exports** — add:

```rust
pub use xcode::{BuildOutput, BuildParams, SupervisedResult};
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo build -p xcode-mcp-core`
Expected: compiles without errors.

- [ ] **Step 4: Commit**

```bash
git add -A && git commit -m "feat: add full run_build lifecycle with timeout, store, and error counts"
```

---

## Task 14: xcode-mcp Bin — Debug CLI

**Files:**
- Create: `crates/xcode-mcp/src/cli.rs`
- Modify: `crates/xcode-mcp/src/main.rs`
- Create: `crates/xcode-mcp/src/server.rs` (stub — full impl in Task 15)
- Test: manual (`xcode-mcp debug --help`, `xcode-mcp debug inspector-help`)

- [ ] **Step 1: Implement cli.rs**

Create `crates/xcode-mcp/src/cli.rs`:

```rust
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use xcode_mcp_core::{
    diagnostic::load_diagnostics, scheme::list_schemes,
    store::BuildStore, xcode::{run_build, BuildParams},
};

#[derive(Parser)]
#[command(name = "xcode-mcp")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand)]
pub enum Command {
    /// Run as MCP server (stdio transport)
    Serve,
    /// Debug subcommands for non-MCP testing
    Debug { #[command(subcommand)] subcommand: DebugCommand },
}

#[derive(Subcommand)]
pub enum DebugCommand {
    /// List schemes for a project or workspace
    ListSchemes {
        project_or_workspace: String,
        #[arg(long)] root: Option<String>,
    },
    /// Run a build
    Build {
        #[arg(long)] project: String,
        #[arg(long)] scheme: String,
        #[arg(long, default_value = "build")] action: String,
        #[arg(long)] configuration: Option<String>,
        #[arg(long)] destination: Option<String>,
        #[arg(long)] timeout_secs: Option<u32>,
        #[arg(long)] root: Option<String>,
        #[arg(long)] result_dir: Option<PathBuf>,
        #[arg(long)] log_dir: Option<PathBuf>,
    },
    /// Get build errors for a build
    BuildErrors {
        build_id: Option<String>,
        #[arg(long)] result_dir: Option<PathBuf>,
        #[arg(long)] log_dir: Option<PathBuf>,
    },
    /// Print MCP Inspector instructions
    InspectorHelp,
}

pub async fn run_debug(subcommand: DebugCommand) -> Result<(), Box<dyn std::error::Error>> {
    match subcommand {
        DebugCommand::ListSchemes { project_or_workspace, root } => {
            let root = resolve_root(&root, &project_or_workspace)?;
            let info = list_schemes(&project_or_workspace, &root).await?;
            println!("{}", serde_json::to_string_pretty(&info)?);
        }
        DebugCommand::Build { project, scheme, action, configuration, destination, timeout_secs, root, result_dir, log_dir } => {
            let root = resolve_root(&root, &project)?;
            let result_dir = result_dir.unwrap_or_else(|| root.join(".xcode-mcp-results"));
            let log_dir = log_dir.unwrap_or_else(|| root.join(".xcode-mcp-logs"));
            std::fs::create_dir_all(&result_dir)?;
            std::fs::create_dir_all(&log_dir)?;
            let store = BuildStore::new(32);
            let params = BuildParams { project_or_workspace: project, scheme, action: Some(action), configuration, destination, timeout_secs };
            let output = run_build(params, &root, &result_dir, &log_dir, &store).await?;
            println!("{}", serde_json::to_string_pretty(&output)?);
        }
        DebugCommand::BuildErrors { build_id, result_dir, log_dir } => {
            let root = std::env::current_dir()?;
            let result_dir = result_dir.unwrap_or_else(|| root.join(".xcode-mcp-results"));
            let log_dir = log_dir.unwrap_or_else(|| root.join(".xcode-mcp-logs"));
            let store = BuildStore::new(32);
            let output = load_diagnostics(build_id.as_deref(), &store, &result_dir, &log_dir).await?;
            println!("{}", serde_json::to_string_pretty(&output)?);
        }
        DebugCommand::InspectorHelp => { print_inspector_help(); }
    }
    Ok(())
}

fn resolve_root(root: &Option<String>, project: &str) -> Result<PathBuf, Box<dyn std::error::Error>> {
    if let Some(r) = root {
        let p = PathBuf::from(r);
        if !p.is_absolute() { return Err("--root must be absolute".into()); }
        Ok(p.canonicalize()?)
    } else {
        let p = PathBuf::from(project);
        Ok(p.parent().ok_or("cannot determine root")?.canonicalize()?)
    }
}

fn print_inspector_help() {
    println!("MCP Inspector Instructions");
    println!("===========================");
    println!();
    println!("1. Build the server:");
    println!("   cargo build --release");
    println!();
    println!("2. Launch the MCP Inspector:");
    println!("   npx @modelcontextprotocol/inspector ./target/release/xcode-mcp serve");
    println!();
    println!("3. In the Inspector UI:");
    println!("   - Transport: STDIO");
    println!("   - Command:   ./target/release/xcode-mcp");
    println!("   - Args:      serve");
    println!("   - Env:       XCODE_MCP_ROOT=/path/to/your/projects");
    println!("   - Click Connect");
    println!();
    println!("4. Set XCODE_MCP_ROOT in the Environment panel BEFORE connecting.");
    println!();
    println!("5. Verify each tool:");
    println!("   - xcode_list_schemes: pass a .xcodeproj path");
    println!("   - xcode_build:        pass path + scheme");
    println!("   - xcode_get_build_errors: pass the build_id");
}
```

- [ ] **Step 2: Create server.rs stub**

Create `crates/xcode-mcp/src/server.rs`:

```rust
pub async fn run_server() -> Result<(), Box<dyn std::error::Error>> {
    eprintln!("xcode-mcp server: not yet implemented (see Task 15)");
    std::process::exit(1);
}
```

- [ ] **Step 3: Implement main.rs**

```rust
mod cli;
mod server;

use clap::Parser;
use cli::{Cli, Command};

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let result = match cli.command {
        None | Some(Command::Serve) => server::run_server().await,
        Some(Command::Debug { subcommand }) => cli::run_debug(subcommand).await,
    };
    if let Err(e) = result {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
```

- [ ] **Step 4: Verify CLI compiles and help works**

Run: `cargo build -p xcode-mcp && ./target/debug/xcode-mcp debug inspector-help`
Expected: prints MCP Inspector instructions.

Run: `./target/debug/xcode-mcp debug --help`
Expected: prints usage for all debug subcommands.

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat: add debug CLI with list-schemes, build, build-errors subcommands"
```

---

## Task 15: xcode-mcp Bin — MCP Server (rmcp)

**Files:**
- Modify: `crates/xcode-mcp/src/server.rs` (full implementation)
- Test: manual (verified via MCP Inspector)

> **Implementer note:** The exact rmcp 3.1.1 trait method signatures may differ. Verify against `docs.rs/rmcp/3.1.1`. The core logic (arg parsing, dispatch to xcode-mcp-core) is correct; only the rmcp trait plumbing may need adjustment.

- [ ] **Step 1: Implement server.rs**

```rust
use rmcp::{model::*, ServerHandler, ServiceExt};
use serde_json::Value;
use std::env;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::{stdin, stdout};
use xcode_mcp_core::{
    diagnostic::load_diagnostics, scheme::list_schemes,
    store::BuildStore, xcode::{run_build, BuildParams},
};

#[derive(Clone)]
struct XcodeMcpServer {
    root: PathBuf,
    result_dir: PathBuf,
    log_dir: PathBuf,
    store: Arc<BuildStore>,
}

impl ServerHandler for XcodeMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            server_info: Implementation {
                name: "xcode-mcp".into(),
                version: env!("CARGO_PKG_VERSION").into(),
            },
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            ..Default::default()
        }
    }

    fn list_tools(&self) -> Vec<Tool> {
        vec![
            Tool {
                name: "xcode_list_schemes".into(),
                description: "List schemes, targets, and configurations for an Xcode project or workspace".into(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": { "project_or_workspace": { "type": "string" } },
                    "required": ["project_or_workspace"]
                }),
            },
            Tool {
                name: "xcode_build".into(),
                description: "Run an xcodebuild and return build_id + status".into(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "project_or_workspace": {"type": "string"},
                        "scheme": {"type": "string"},
                        "action": {"type": "string", "enum": ["build", "clean", "clean+build"]},
                        "configuration": {"type": "string", "enum": ["Debug", "Release"]},
                        "destination": {"type": "string"},
                        "timeout_secs": {"type": "integer"}
                    },
                    "required": ["project_or_workspace", "scheme"]
                }),
            },
            Tool {
                name: "xcode_get_build_errors".into(),
                description: "Get structured build diagnostics for a build".into(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": { "build_id": {"type": "string"} }
                }),
            },
        ]
    }

    fn call_tool(&self, name: &str, params: CallToolRequestParam) -> Result<CallToolResult, rmcp::Error> {
        let runtime = tokio::runtime::Handle::current();
        let result = match name {
            "xcode_list_schemes" => {
                let args: Value = serde_json::from_str(&params.arguments.unwrap_or_default())
                    .unwrap_or(Value::Null);
                let project = args["project_or_workspace"].as_str()
                    .ok_or_else(|| rmcp::Error::invalid_params("project_or_workspace required", None))?;
                let info = runtime.block_on(list_schemes(project, &self.root))
                    .map_err(|e| rmcp::Error::internal_error(format!("{e}"), None))?;
                serde_json::to_value(&info).map_err(|e| rmcp::Error::internal_error(format!("{e}"), None))?
            }
            "xcode_build" => {
                let args: Value = serde_json::from_str(&params.arguments.unwrap_or_default())
                    .unwrap_or(Value::Null);
                let build_params = BuildParams {
                    project_or_workspace: args["project_or_workspace"].as_str()
                        .ok_or_else(|| rmcp::Error::invalid_params("project_or_workspace required", None))?.to_string(),
                    scheme: args["scheme"].as_str()
                        .ok_or_else(|| rmcp::Error::invalid_params("scheme required", None))?.to_string(),
                    action: args["action"].as_str().map(String::from),
                    configuration: args["configuration"].as_str().map(String::from),
                    destination: args["destination"].as_str().map(String::from),
                    timeout_secs: args["timeout_secs"].as_u64().map(|n| n as u32),
                };
                let output = runtime.block_on(run_build(
                    build_params, &self.root, &self.result_dir, &self.log_dir, &self.store,
                )).map_err(|e| rmcp::Error::internal_error(format!("{e}"), None))?;
                serde_json::to_value(&output).map_err(|e| rmcp::Error::internal_error(format!("{e}"), None))?
            }
            "xcode_get_build_errors" => {
                let args: Value = serde_json::from_str(&params.arguments.unwrap_or_default())
                    .unwrap_or(Value::Null);
                let build_id = args["build_id"].as_str();
                let output = runtime.block_on(load_diagnostics(
                    build_id, &self.store, &self.result_dir, &self.log_dir,
                )).map_err(|e| rmcp::Error::internal_error(format!("{e}"), None))?;
                serde_json::to_value(&output).map_err(|e| rmcp::Error::internal_error(format!("{e}"), None))?
            }
            _ => return Err(rmcp::Error::invalid_params(format!("unknown tool: {name}"), None)),
        };
        Ok(CallToolResult {
            content: vec![Content::Text { text: serde_json::to_string_pretty(&result)
                .map_err(|e| rmcp::Error::internal_error(format!("{e}"), None))? }],
            ..Default::default()
        })
    }
}

pub async fn run_server() -> Result<(), Box<dyn std::error::Error>> {
    let root_str = env::var("XCODE_MCP_ROOT").map_err(|_| {
        "XCODE_MCP_ROOT not set. Set it to the directory containing your Xcode projects."
    })?;
    let root = PathBuf::from(&root_str);
    if !root.exists() { return Err(format!("XCODE_MCP_ROOT does not exist: {root_str}").into()); }
    let root = root.canonicalize()?;

    let result_dir = PathBuf::from(env::var("XCODE_MCP_RESULT_DIR")
        .unwrap_or_else(|_| root.join(".xcode-mcp-results").to_string_lossy().into_owned()));
    let log_dir = PathBuf::from(env::var("XCODE_MCP_LOG_DIR")
        .unwrap_or_else(|_| root.join(".xcode-mcp-logs").to_string_lossy().into_owned()));
    std::fs::create_dir_all(&result_dir)?;
    std::fs::create_dir_all(&log_dir)?;

    // File logging (never stdout/stderr — MCP channel)
    let log_file = std::fs::OpenOptions::new()
        .create(true).append(true)
        .open(log_dir.join("server.log"))?;
    tracing_subscriber::fmt()
        .with_writer(std::sync::Mutex::new(log_file))
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    tracing::info!("xcode-mcp server starting: root={}, result_dir={}, log_dir={}",
        root.display(), result_dir.display(), log_dir.display());

    let store = Arc::new(BuildStore::new(
        env::var("XCODE_MCP_STORE_CAP").ok().and_then(|s| s.parse().ok()).unwrap_or(32),
    ));
    let server = XcodeMcpServer { root, result_dir, log_dir, store };
    let transport = (stdin(), stdout());
    server.serve(transport).await?;
    Ok(())
}
```

> **Note:** If rmcp 3.1.1 uses async trait methods, change `fn call_tool` to `async fn call_tool` and remove `runtime.block_on` calls — use `.await` directly. If `ServiceExt::serve` has a different signature, adjust accordingly. The tool argument parsing and dispatch logic stays the same.

- [ ] **Step 2: Verify it compiles**

Run: `cargo build`
Expected: compiles. If rmcp API differs, adjust trait impl to match.

- [ ] **Step 3: Test server startup (should fail fast without XCODE_MCP_ROOT)**

Run: `./target/debug/xcode-mcp serve`
Expected: error about `XCODE_MCP_ROOT` not set, exits 1.

Run: `XCODE_MCP_ROOT=/tmp ./target/debug/xcode-mcp serve &  sleep 1; kill %1`
Expected: server starts, logs to `/tmp/.xcode-mcp-logs/server.log`.

- [ ] **Step 4: Commit**

```bash
git add -A && git commit -m "feat: add MCP server with 3 tools over stdio transport"
```

---

## Task 16: MiniApp Fixture + Live Integration Tests

**Files:**
- Create: `crates/xcode-mcp-core/tests/fixtures/MiniApp/project.yml` (xcodegen spec)
- Create: `crates/xcode-mcp-core/tests/fixtures/MiniApp/Sources/MiniApp/main.swift` (valid)
- Create: `crates/xcode-mcp-core/tests/fixtures/MiniApp/Sources/MiniAppBroken/main.swift` (deliberate error)
- Create: `crates/xcode-mcp-core/tests/live/build.rs`
- Create: `crates/xcode-mcp-core/tests/live/errors.rs`

**Prerequisites:** `brew install xcodegen` (dev-only tool for generating the fixture .xcodeproj)

- [ ] **Step 1: Create fixture project spec**

Create `crates/xcode-mcp-core/tests/fixtures/MiniApp/project.yml`:

```yaml
name: MiniApp
options:
  bundleIdPrefix: com.xcode-mcp.test
targets:
  MiniApp:
    type: tool
    platform: macOS
    sources:
      - Sources/MiniApp/main.swift
  MiniAppBroken:
    type: tool
    platform: macOS
    sources:
      - Sources/MiniAppBroken/main.swift
```

- [ ] **Step 2: Create valid Swift source**

Create `crates/xcode-mcp-core/tests/fixtures/MiniApp/Sources/MiniApp/main.swift`:

```swift
import Foundation

print("Hello from MiniApp")
```

- [ ] **Step 3: Create broken Swift source**

Create `crates/xcode-mcp-core/tests/fixtures/MiniApp/Sources/MiniAppBroken/main.swift`:

```swift
import Foundation

// Deliberate error: use of undeclared identifier
print(nonexistentVariable)

// Deliberate warning: unused variable
let unused = 42

print("This will never print")
```

- [ ] **Step 4: Generate the .xcodeproj**

Run: `cd crates/xcode-mcp-core/tests/fixtures/MiniApp && xcodegen generate && cd -`
Expected: `MiniApp.xcodeproj` created.

- [ ] **Step 5: Create live build test**

Create `crates/xcode-mcp-core/tests/live/build.rs`:

```rust
#![cfg(feature = "live-xcode")]

use std::env;
use std::path::PathBuf;
use xcode_mcp_core::{scheme::list_schemes, store::BuildStore, xcode::{run_build, BuildParams}};

fn skip_if_not_enabled() -> bool { env::var("XCODE_MCP_LIVE_TESTS").is_err() }
fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/MiniApp").canonicalize().unwrap()
}

#[tokio::test]
async fn list_schemes_finds_miniapp() {
    if skip_if_not_enabled() { eprintln!("skipping: set XCODE_MCP_LIVE_TESTS=1"); return; }
    let root = fixture_root();
    let proj = root.join("MiniApp.xcodeproj");
    let info = list_schemes(proj.to_str().unwrap(), &root).await.unwrap();
    assert!(info.schemes.contains(&"MiniApp".to_string()));
    assert!(info.schemes.contains(&"MiniAppBroken".to_string()));
}

#[tokio::test]
async fn build_succeeds_for_valid_app() {
    if skip_if_not_enabled() { eprintln!("skipping: set XCODE_MCP_LIVE_TESTS=1"); return; }
    let root = fixture_root();
    let result_dir = root.join(".xcode-mcp-results");
    let log_dir = root.join(".xcode-mcp-logs");
    std::fs::create_dir_all(&result_dir).unwrap();
    std::fs::create_dir_all(&log_dir).unwrap();
    let store = BuildStore::new(32);
    let params = BuildParams {
        project_or_workspace: root.join("MiniApp.xcodeproj").to_string_lossy().into(),
        scheme: "MiniApp".into(), action: Some("build".into()),
        configuration: Some("Debug".into()), destination: Some("platform=macOS".into()),
        timeout_secs: Some(300),
    };
    let output = run_build(params, &root, &result_dir, &log_dir, &store).await.unwrap();
    assert_eq!(output.status, "Succeeded");
    assert!(output.result_bundle_written);
}

#[tokio::test]
async fn build_fails_for_broken_app() {
    if skip_if_not_enabled() { eprintln!("skipping: set XCODE_MCP_LIVE_TESTS=1"); return; }
    let root = fixture_root();
    let result_dir = root.join(".xcode-mcp-results");
    let log_dir = root.join(".xcode-mcp-logs");
    std::fs::create_dir_all(&result_dir).unwrap();
    std::fs::create_dir_all(&log_dir).unwrap();
    let store = BuildStore::new(32);
    let params = BuildParams {
        project_or_workspace: root.join("MiniApp.xcodeproj").to_string_lossy().into(),
        scheme: "MiniAppBroken".into(), action: Some("build".into()),
        configuration: Some("Debug".into()), destination: Some("platform=macOS".into()),
        timeout_secs: Some(300),
    };
    let output = run_build(params, &root, &result_dir, &log_dir, &store).await.unwrap();
    assert_eq!(output.status, "Failed");
    assert!(output.error_count > 0);
}

#[tokio::test]
async fn build_times_out() {
    if skip_if_not_enabled() { eprintln!("skipping: set XCODE_MCP_LIVE_TESTS=1"); return; }
    let root = fixture_root();
    let result_dir = root.join(".xcode-mcp-results");
    let log_dir = root.join(".xcode-mcp-logs");
    std::fs::create_dir_all(&result_dir).unwrap();
    std::fs::create_dir_all(&log_dir).unwrap();
    let store = BuildStore::new(32);
    let params = BuildParams {
        project_or_workspace: root.join("MiniApp.xcodeproj").to_string_lossy().into(),
        scheme: "MiniApp".into(), action: Some("clean+build".into()),
        configuration: Some("Debug".into()), destination: Some("platform=macOS".into()),
        timeout_secs: Some(1),
    };
    let output = run_build(params, &root, &result_dir, &log_dir, &store).await.unwrap();
    assert_eq!(output.status, "TimedOut");
    assert!(output.exit_code.is_none());
}
```

- [ ] **Step 6: Create live errors test**

Create `crates/xcode-mcp-core/tests/live/errors.rs`:

```rust
#![cfg(feature = "live-xcode")]

use std::env;
use std::path::PathBuf;
use xcode_mcp_core::{diagnostic::{load_diagnostics, DiagnosticSourceLabel}, store::BuildStore, xcode::{run_build, BuildParams}};

fn skip_if_not_enabled() -> bool { env::var("XCODE_MCP_LIVE_TESTS").is_err() }
fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/MiniApp").canonicalize().unwrap()
}

#[tokio::test]
async fn get_build_errors_returns_xcresult_diagnostics() {
    if skip_if_not_enabled() { eprintln!("skipping: set XCODE_MCP_LIVE_TESTS=1"); return; }
    let root = fixture_root();
    let result_dir = root.join(".xcode-mcp-results");
    let log_dir = root.join(".xcode-mcp-logs");
    std::fs::create_dir_all(&result_dir).unwrap();
    std::fs::create_dir_all(&log_dir).unwrap();
    let store = BuildStore::new(32);
    let params = BuildParams {
        project_or_workspace: root.join("MiniApp.xcodeproj").to_string_lossy().into(),
        scheme: "MiniAppBroken".into(), action: Some("build".into()),
        configuration: Some("Debug".into()), destination: Some("platform=macOS".into()),
        timeout_secs: Some(300),
    };
    let build_output = run_build(params, &root, &result_dir, &log_dir, &store).await.unwrap();
    assert_eq!(build_output.status, "Failed");
    let diag_output = load_diagnostics(Some(&build_output.build_id), &store, &result_dir, &log_dir).await.unwrap();
    assert_eq!(diag_output.build_id, build_output.build_id);
    assert!(matches!(diag_output.source, DiagnosticSourceLabel::Xcresult));
    assert!(!diag_output.merged.errors.is_empty());
    let first_error = &diag_output.merged.errors[0];
    assert!(first_error.file.is_some());
    assert!(first_error.message.contains("nonexistentVariable") || first_error.line.is_some());
}
```

- [ ] **Step 7: Run unit tests (should pass without live tests)**

Run: `cargo test`
Expected: all unit tests pass; live tests excluded (feature off).

- [ ] **Step 8: Run live tests (double-gated)**

Run: `XCODE_MCP_LIVE_TESTS=1 cargo test --features xcode-mcp-core/live-xcode --test build -- --test-threads=1`
Expected: all 4 live build tests pass.

Run: `XCODE_MCP_LIVE_TESTS=1 cargo test --features xcode-mcp-core/live-xcode --test errors -- --test-threads=1`
Expected: errors test passes.

- [ ] **Step 9: Commit**

```bash
git add -A && git commit -m "test: add MiniApp fixture and live integration tests"
```

---

## Task 17: README + Documentation

**Files:**
- Create: `README.md`

- [ ] **Step 1: Write README.md**

Create `README.md`:

```markdown
# Xcode MCP Server

A local MCP (Model Context Protocol) server that drives `xcodebuild` and parses build-failure diagnostics into structured data. Written in Rust.

## Requirements

- **Rust** 1.97+
- **Xcode** 26+ (any version with modern `xcrun xcresulttool get build-results`)
- **macOS** (depends on `xcrun`, `xcodebuild`, `xcresulttool`)
- **xcodegen** (only for live integration tests: `brew install xcodegen`)

## Build

```bash
cargo build --release
```

Binary at `target/release/xcode-mcp`.

## Configure

Set `XCODE_MCP_ROOT` to the directory containing your `.xcodeproj`/`.xcworkspace` files:

```bash
export XCODE_MCP_ROOT=/path/to/your/projects
```

Optional environment variables:

| Variable | Default | Description |
|---|---|---|
| `XCODE_MCP_RESULT_DIR` | `$ROOT/.xcode-mcp-results` | Where `.xcresult` bundles are stored |
| `XCODE_MCP_LOG_DIR` | `$ROOT/.xcode-mcp-logs` | Where build logs + server log are stored |
| `XCODE_MCP_STORE_CAP` | `32` | Max builds kept in memory ring buffer |
| `XCODE_MCP_RESULT_TTL_HOURS` | `24` | Max age of `.xcresult` files before pruning |

## Run as MCP Server

```bash
XCODE_MCP_ROOT=/path/to/projects ./target/release/xcode-mcp serve
```

Or just `./target/release/xcode-mcp` (serve is the default).

## MCP Inspector

```bash
# 1. Build the server
cargo build --release

# 2. Launch the MCP Inspector
npx @modelcontextprotocol/inspector ./target/release/xcode-mcp serve

# 3. In the Inspector UI:
#    - Transport: STDIO
#    - Command:   ./target/release/xcode-mcp
#    - Args:      serve
#    - Env:       XCODE_MCP_ROOT=/path/to/your/projects
#    - Click "Connect"

# 4. Set the env var in the Inspector's "Environment" panel BEFORE connecting
#    (the server fail-fasts at startup if XCODE_MCP_ROOT is unset).

# 5. Verify each tool:
#    - xcode_list_schemes: pass a .xcodeproj path -> see schemes/configs/targets
#    - xcode_build:        pass path + scheme -> see build_id + status
#    - xcode_get_build_errors: pass the build_id -> see diagnostics

# 6. Debug CLI alongside Inspector:
#    ./target/release/xcode-mcp debug list-schemes /path/to/App.xcodeproj
```

You can also print these instructions: `./target/release/xcode-mcp debug inspector-help`.

## Debug CLI

```bash
# List schemes
xcode-mcp debug list-schemes /path/to/App.xcodeproj

# Build
xcode-mcp debug build \
  --project /path/to/App.xcodeproj \
  --scheme App \
  --configuration Debug \
  --destination "platform=macOS"

# Get build errors (build_id from build output)
xcode-mcp debug build-errors <build_id>
```

## Tools Reference

### `xcode_list_schemes`

| Parameter | Type | Required | Description |
|---|---|---|---|
| `project_or_workspace` | string | yes | Path to `.xcodeproj` or `.xcworkspace` |

### `xcode_build`

| Parameter | Type | Required | Description |
|---|---|---|---|
| `project_or_workspace` | string | yes | Path to `.xcodeproj` or `.xcworkspace` |
| `scheme` | string | yes | Scheme name |
| `action` | `"build"` \| `"clean"` \| `"clean+build"` | no | Default: `build` |
| `configuration` | `"Debug"` \| `"Release"` | no | If unset, xcodebuild picks |
| `destination` | string | no | e.g. `generic/platform=iOS`, `platform=macOS` |
| `timeout_secs` | integer | no | Default 1800, max 7200 |

### `xcode_get_build_errors`

| Parameter | Type | Required | Description |
|---|---|---|---|
| `build_id` | string | no | Defaults to most recent build |

## Testing

```bash
# Unit tests (fast, no Xcode needed)
cargo test

# Live integration tests (requires Xcode + xcodegen)
cd crates/xcode-mcp-core/tests/fixtures/MiniApp && xcodegen generate && cd -
XCODE_MCP_LIVE_TESTS=1 cargo test --features xcode-mcp-core/live-xcode -- --test-threads=1
```

Live tests are double-gated: both the `live-xcode` feature flag AND the `XCODE_MCP_LIVE_TESTS=1` env var must be set.

## Security Model

- `XCODE_MCP_ROOT` is the single trust boundary. All project paths must canonicalize under it.
- No shell invocation — all `xcodebuild` calls use `Command::new("xcrun").arg(...)`.
- No `extra_args` passthrough — fixed xcodebuild flag surface only.
- Scheme/destination/configuration values are charset-validated to prevent flag injection.
- Build timeout kills the entire process group (SIGTERM -> SIGKILL) to prevent orphaned compiler processes.

## Architecture

Cargo workspace:
- `xcode-mcp-core` (lib): all logic — security validation, scheme parsing, xcresult parsing, stderr parsing, diagnostic merging, process supervision, build store.
- `xcode-mcp` (bin): thin rmcp stdio server + debug CLI.

Diagnostic sourcing is hybrid: primary `xcresulttool get build-results` JSON (structured, with fix-its), fallback stderr regex parsing (for early-exit failures that never produce a result bundle).
```

- [ ] **Step 2: Commit**

```bash
git add -A && git commit -m "docs: add README with build, run, Inspector, and testing instructions"
```

---

## Verification Checklist

After all 17 tasks are complete, verify each of the 15 requirements:

| # | Requirement | How to verify |
|---|---|---|
| 1 | rmcp 3.1.1 MCP server | `grep 'rmcp = "3.1.1"' Cargo.toml` + server boots under Inspector |
| 2 | stdio transport | `rg 'stdin\|stdout' crates/xcode-mcp/src/server.rs` — no TCP/HTTP |
| 3 | `xcode_list_schemes` | `cargo test --test scheme_parse` + live test |
| 4 | `xcode_build` | `cargo test --test xcode_commands` + live test |
| 5 | `xcode_get_build_errors` | `cargo test --test load_diagnostics` + live test |
| 6 | `.xcodeproj` + `.xcworkspace` | `cargo test --test security` (both extensions) |
| 7 | stdout/stderr capture | `cargo test --test process_supervisor` |
| 8 | `.xcresult` generation | `rg resultBundlePath crates/xcode-mcp-core/src/xcode.rs` + live test |
| 9 | `xcresulttool get build-results` | `cargo test --test result_bundle_parse` + live test |
| 10 | compiler/linker/build diagnostics | `cargo test --test diagnostic_parse` + `cargo test --test result_bundle_parse` |
| 11 | timeout + termination | `cargo test --test process_supervisor` (timeout test) + live test |
| 12 | `XCODE_MCP_ROOT` security | `cargo test --test security` (path traversal, symlink escape) |
| 13 | no shell invocation | `cargo test --test xcode_commands` (`no_shell_invocation` test) |
| 14 | unit tests for diagnostic + scheme parsing | `cargo test --test scheme_parse --test diagnostic_parse --test result_bundle_parse` |
| 15 | MCP Inspector instructions | `./target/debug/xcode-mcp debug inspector-help` + README §MCP Inspector |

Final gate: `cargo test` clean, `XCODE_MCP_LIVE_TESTS=1 cargo test --features xcode-mcp-core/live-xcode -- --test-threads=1` clean, manual Inspector smoke test documented in README.
