# Pod install/update before Xcode build — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a standalone `xcode_pod` MCP tool and an optional `pod_action` flag on `xcode_build` so callers can run `pod install` / `pod update` to refresh workspace references before building.

**Architecture:** New `pod.rs` module in `xcode-mcp-core` mirroring the `xcode.rs` / `scheme.rs` per-domain pattern. `build_pod_command()` + `run_pod()` use the existing `run_supervised()` for process-group-kill timeouts. `run_build()` gains a pod pre-step inside the `BUILD_PERMIT` hold; pod failure aborts the build with a new `BuildStatus::PodFailed` variant.

**Tech Stack:** Rust 1.97 (edition 2021), tokio, serde/serde_json, thiserror, regex, clap. No new dependencies.

**Spec:** `docs/superpowers/specs/2026-08-20-pod-install-update-design.md`

## Global Constraints

- **Rust edition:** 2021, MSRV 1.97
- **No shell invocation:** `Command::new("pod")` — never `sh -c`. Matches existing `Command::new("xcrun")` pattern.
- **No `extra_args` passthrough:** fixed pod flag surface = `action` + `--no-ansi` only
- **Security boundary:** `XCODE_MCP_ROOT` — pod `current_dir` is `validated_path.parent()`, guaranteed under-or-equal root
- **Feature gate:** `live-xcode` feature (default off) + `XCODE_MCP_LIVE_TESTS=1` env var — double gate for live tests; no CocoaPods fixture committed
- **macOS only:** depends on `pod` (installed via `gem install cocoapods` / `brew install cocoapods`)
- **Commit style:** conventional commits (`feat:`, `test:`, `docs:`, `refactor:`)
- **No comments** in source code unless explicitly requested
- **PascalCase JSON serialization** for output structs (matches existing `BuildOutput`)

---

## File Structure

```
crates/xcode-mcp-core/src/
├── pod.rs           # NEW: build_pod_command, run_pod, PodOutput, PodStepResult
├── xcode.rs         # MODIFY: BuildParams gains pod_action/pod_timeout_secs; run_build gains pod pre-step + PodFailed handling
├── security.rs      # MODIFY: add validate_pod_action, validate_pod_timeout
├── error.rs         # MODIFY: add PodfileNotFound variant
├── store.rs         # MODIFY: add BuildStatus::PodFailed variant
├── lib.rs           # MODIFY: pub mod pod; re-export PodOutput, PodStepResult
crates/xcode-mcp-core/tests/
├── pod_commands.rs  # NEW: unit tests for build_pod_command (no spawn)
├── security.rs      # MODIFY: add validate_pod_action / validate_pod_timeout tests
├── live/pod.rs      # NEW: feature-gated live integration tests
crates/xcode-mcp-core/Cargo.toml  # MODIFY: register pod live test target
crates/xcode-mcp/src/
├── server.rs        # MODIFY: add xcode_pod tool + schema; add pod_action/pod_timeout_secs to xcode_build schema + dispatch
├── cli.rs           # MODIFY: add DebugCommand::Pod
README.md            # MODIFY: Tools Reference + Debug CLI sections
```

---

## Task 1: Error + Store variants for pod failure

**Files:**
- Modify: `crates/xcode-mcp-core/src/error.rs`
- Modify: `crates/xcode-mcp-core/src/store.rs`
- Test: `crates/xcode-mcp-core/tests/types.rs`

**Interfaces:**
- Produces: `Error::PodfileNotFound { working_dir: PathBuf }`, `BuildStatus::PodFailed`

- [ ] **Step 1: Add `PodfileNotFound` to `Error`**

Edit `crates/xcode-mcp-core/src/error.rs`. Insert this variant after the `BuildNotFound` variant (before `NoBuildAvailable`):

```rust
    #[error("no Podfile found next to {working_dir}")]
    PodfileNotFound { working_dir: PathBuf },
```

- [ ] **Step 2: Add `PodFailed` to `BuildStatus`**

Edit `crates/xcode-mcp-core/src/store.rs`. Add `PodFailed` as the last variant of the `BuildStatus` enum:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub enum BuildStatus {
    Succeeded,
    Failed,
    TimedOut,
    Canceled,
    Unknown,
    PodFailed,
}
```

- [ ] **Step 3: Write failing test for `PodFailed` serialization**

Append to `crates/xcode-mcp-core/tests/types.rs`:

```rust
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
    use xcode_mcp_core::error::Error;
    use std::path::PathBuf;
    let e = Error::PodfileNotFound {
        working_dir: PathBuf::from("/tmp/proj"),
    };
    assert_eq!(e.to_string(), "no Podfile found next to /tmp/proj");
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p xcode-mcp-core --test types`
Expected: PASS (variants are trivial additions; tests confirm serialization/display contract)

- [ ] **Step 5: Commit**

```bash
git add crates/xcode-mcp-core/src/error.rs crates/xcode-mcp-core/src/store.rs crates/xcode-mcp-core/tests/types.rs
git commit -m "feat: add PodfileNotFound error and PodFailed build status"
```

---

## Task 2: Security validation for pod action + timeout

**Files:**
- Modify: `crates/xcode-mcp-core/src/security.rs`
- Test: `crates/xcode-mcp-core/tests/security.rs`

**Interfaces:**
- Produces: `validate_pod_action(s: &str) -> Result<String>`, `validate_pod_timeout(t: Option<u32>) -> Result<u32>`

- [ ] **Step 1: Write failing tests for `validate_pod_action` and `validate_pod_timeout`**

Append to `crates/xcode-mcp-core/tests/security.rs`:

```rust
#[test]
fn pod_action_accepts_install_and_update() {
    assert_eq!(validate_pod_action("install").unwrap(), "install");
    assert_eq!(validate_pod_action("update").unwrap(), "update");
}

#[test]
fn pod_action_rejects_others() {
    assert!(validate_pod_action("outdated").is_err());
    assert!(validate_pod_action("install --verbose").is_err());
    assert!(validate_pod_action("").is_err());
    assert!(validate_pod_action("INSTALL").is_err());
    assert!(validate_pod_action("repo-update").is_err());
}

#[test]
fn pod_timeout_defaults_and_validates() {
    assert_eq!(validate_pod_timeout(None).unwrap(), 600);
    assert_eq!(validate_pod_timeout(Some(60)).unwrap(), 60);
    assert_eq!(validate_pod_timeout(Some(3600)).unwrap(), 3600);
    assert!(validate_pod_timeout(Some(0)).is_err());
    assert!(validate_pod_timeout(Some(3601)).is_err());
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p xcode-mcp-core --test security pod_action pod_timeout`
Expected: FAIL with "cannot find function `validate_pod_action`" / `validate_pod_timeout`

- [ ] **Step 3: Implement `validate_pod_action` and `validate_pod_timeout`**

Append to `crates/xcode-mcp-core/src/security.rs`:

```rust
pub fn validate_pod_action(a: &str) -> Result<String> {
    match a {
        "install" | "update" => Ok(a.to_string()),
        _ => Err(Error::InvalidArgument(format!(
            "pod action must be install or update: {a:?}"
        ))),
    }
}

pub fn validate_pod_timeout(t: Option<u32>) -> Result<u32> {
    match t {
        None => Ok(600),
        Some(v) if (1..=3600).contains(&v) => Ok(v),
        Some(v) => Err(Error::InvalidArgument(format!(
            "pod_timeout_secs must be 1..=3600: {v}"
        ))),
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p xcode-mcp-core --test security pod_action pod_timeout`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/xcode-mcp-core/src/security.rs crates/xcode-mcp-core/tests/security.rs
git commit -m "feat: add validate_pod_action and validate_pod_timeout"
```

---

## Task 3: `pod.rs` module — command builder + `run_pod` + types

**Files:**
- Create: `crates/xcode-mcp-core/src/pod.rs`
- Modify: `crates/xcode-mcp-core/src/lib.rs`
- Test: `crates/xcode-mcp-core/tests/pod_commands.rs`

**Interfaces:**
- Consumes: `validate_project_or_workspace` (from `security.rs`), `validate_pod_action`, `validate_pod_timeout` (from Task 2), `run_supervised` (from `xcode.rs`), `Error::PodfileNotFound` (from Task 1)
- Produces: `build_pod_command(working_dir: &Path, action: &str) -> Command`, `run_pod(params: PodParams, root: &Path, log_dir: &Path) -> Result<PodOutput>`, `struct PodParams { project_or_workspace: String, action: Option<String>, timeout_secs: Option<u32> }`, `struct PodOutput { run_id, action, working_dir, status, exit_code, duration_secs, log_path, stderr_excerpt }`

- [ ] **Step 1: Write failing tests for `build_pod_command`**

Create `crates/xcode-mcp-core/tests/pod_commands.rs`:

```rust
use std::path::PathBuf;
use xcode_mcp_core::pod::build_pod_command;

fn args_of(cmd: tokio::process::Command) -> Vec<String> {
    cmd.as_std()
        .get_args()
        .map(|s| s.to_str().unwrap().to_string())
        .collect()
}

#[test]
fn pod_command_install_has_required_flags() {
    let cmd = build_pod_command(&PathBuf::from("/tmp/proj"), "install");
    let args = args_of(cmd);
    assert!(args.contains(&"install".into()));
    assert!(args.contains(&"--no-ansi".into()));
}

#[test]
fn pod_command_update_uses_update_action() {
    let cmd = build_pod_command(&PathBuf::from("/tmp/proj"), "update");
    let args = args_of(cmd);
    assert!(args.contains(&"update".into()));
    assert!(args.contains(&"--no-ansi".into()));
}

#[test]
fn pod_command_no_shell_invocation() {
    let cmd = build_pod_command(&PathBuf::from("/tmp/proj"), "install");
    assert_eq!(cmd.as_std().get_program().to_str().unwrap(), "pod");
}

#[test]
fn pod_command_no_extra_flags() {
    let cmd = build_pod_command(&PathBuf::from("/tmp/proj"), "install");
    let args = args_of(cmd);
    assert_eq!(args, vec!["install", "--no-ansi"]);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p xcode-mcp-core --test pod_commands`
Expected: FAIL with "unresolved module `pod`" / "cannot find function `build_pod_command`"

- [ ] **Step 3: Create `pod.rs` with command builder + types + `run_pod`**

Create `crates/xcode-mcp-core/src/pod.rs`:

```rust
use crate::error::{Error, Result};
use crate::security::{
    validate_pod_action, validate_pod_timeout, validate_project_or_workspace,
};
use crate::xcode::run_supervised;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tokio::process::Command;

pub fn build_pod_command(working_dir: &Path, action: &str) -> Command {
    let mut cmd = Command::new("pod");
    cmd.arg(action).arg("--no-ansi");
    cmd.current_dir(working_dir);
    cmd
}

#[derive(Debug, Clone, Deserialize)]
pub struct PodParams {
    pub project_or_workspace: String,
    pub action: Option<String>,
    pub timeout_secs: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct PodOutput {
    pub run_id: String,
    pub action: String,
    pub working_dir: String,
    pub status: String,
    pub exit_code: Option<i32>,
    pub duration_secs: f64,
    pub log_path: String,
    pub stderr_excerpt: Option<String>,
}

pub async fn run_pod(params: PodParams, root: &Path, log_dir: &Path) -> Result<PodOutput> {
    let validated_path = validate_project_or_workspace(&params.project_or_workspace, root)?;
    let working_dir: PathBuf = validated_path
        .parent()
        .ok_or_else(|| Error::PathRejected(format!(
            "cannot resolve parent of {}", validated_path.display()
        )))?
        .to_path_buf();

    let action = validate_pod_action(params.action.as_deref().unwrap_or("install"))?;
    let timeout_secs = validate_pod_timeout(params.timeout_secs)?;

    if !working_dir.join("Podfile").exists() {
        return Err(Error::PodfileNotFound { working_dir });
    }

    let run_id = uuid::Uuid::new_v4().to_string();
    let log_path = log_dir.join(format!("{run_id}.pod.log"));
    std::fs::File::create(&log_path)?;

    let cmd = build_pod_command(&working_dir, &action);
    let start = std::time::Instant::now();
    let result = run_supervised(cmd, timeout_secs, Some(&log_path)).await?;
    let duration = start.elapsed().as_secs_f64();

    let (status, exit_code) = if result.timed_out {
        ("TimedOut".to_string(), None)
    } else if result.exit_code == Some(0) {
        ("Succeeded".to_string(), result.exit_code)
    } else {
        ("Failed".to_string(), result.exit_code)
    };

    let stderr_excerpt = if status != "Succeeded" {
        let s = String::from_utf8_lossy(&result.stderr);
        let bytes = s.as_bytes();
        let start = bytes.len().saturating_sub(2048);
        let start = s.ceil_char_boundary(start);
        Some(s[start..].to_string())
    } else {
        None
    };

    Ok(PodOutput {
        run_id,
        action,
        working_dir: working_dir.to_string_lossy().into_owned(),
        status,
        exit_code,
        duration_secs: duration,
        log_path: log_path.to_string_lossy().into_owned(),
        stderr_excerpt,
    })
}
```

- [ ] **Step 4: Register the `pod` module and re-exports**

Edit `crates/xcode-mcp-core/src/lib.rs`. Add `pub mod pod;` to the module list (after `pub mod xcode;`), and add re-exports after the existing `xcode` re-exports:

```rust
pub mod pod;
```

And at the bottom, extend the re-export block:

```rust
pub use pod::{PodOutput, PodParams};
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p xcode-mcp-core --test pod_commands`
Expected: PASS (all 4 tests)

- [ ] **Step 6: Run full unit test suite to confirm no regressions**

Run: `cargo test -p xcode-mcp-core --lib && cargo test -p xcode-mcp-core --tests --features live-xcode --no-run`
Expected: all pass / compile

- [ ] **Step 7: Commit**

```bash
git add crates/xcode-mcp-core/src/pod.rs crates/xcode-mcp-core/src/lib.rs crates/xcode-mcp-core/tests/pod_commands.rs
git commit -m "feat: add pod module with build_pod_command and run_pod"
```

---

## Task 4: Integrate pod pre-step into `run_build`

**Files:**
- Modify: `crates/xcode-mcp-core/src/xcode.rs`
- Modify: `crates/xcode-mcp/src/cli.rs` (update existing `BuildParams` construction in `DebugCommand::Build` arm)
- Modify: `crates/xcode-mcp-core/tests/live/build.rs` (3 existing `BuildParams` constructions)
- Modify: `crates/xcode-mcp-core/tests/live/errors.rs` (1 existing `BuildParams` construction)
- Test: `crates/xcode-mcp-core/tests/xcode_commands.rs` (extend)

**Interfaces:**
- Consumes: `run_pod`, `PodParams`, `PodOutput` (from Task 3), `validate_pod_action`, `validate_pod_timeout` (from Task 2), `BuildStatus::PodFailed` (from Task 1)
- Produces: `BuildParams` gains `pod_action: Option<String>` + `pod_timeout_secs: Option<u32>`; `BuildOutput` gains `pod: Option<PodStepResult>`; `struct PodStepResult { action, status, exit_code, duration_secs, log_path, stderr_excerpt }`

- [ ] **Step 1: Add `PodStepResult` and extend `BuildParams` / `BuildOutput`**

Edit `crates/xcode-mcp-core/src/xcode.rs`. Add `PodStepResult` (a trimmed mirror of `PodOutput`) near `BuildOutput`:

```rust
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct PodStepResult {
    pub action: String,
    pub status: String,
    pub exit_code: Option<i32>,
    pub duration_secs: f64,
    pub log_path: String,
    pub stderr_excerpt: Option<String>,
}
```

Extend `BuildParams` with two fields:

```rust
#[derive(Debug, Clone, Deserialize)]
pub struct BuildParams {
    pub project_or_workspace: String,
    pub scheme: String,
    pub action: Option<String>,
    pub configuration: Option<String>,
    pub destination: Option<String>,
    pub timeout_secs: Option<u32>,
    pub pod_action: Option<String>,
    pub pod_timeout_secs: Option<u32>,
}
```

Extend `BuildOutput` with one field:

```rust
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
    pub pod: Option<PodStepResult>,
}
```

- [ ] **Step 2: Add the pod pre-step to `run_build`**

Edit `crates/xcode-mcp-core/src/xcode.rs`. In `run_build`, after the existing validation block (step 1) and before the `BUILD_PERMIT` acquisition, insert pod validation. Then after the permit is acquired, run the pod step. The modification touches the section between "2. Reserve build_id + paths" and "4. Build command".

Add these imports at the top of `xcode.rs`:

```rust
use crate::pod::{run_pod, PodParams};
use crate::security::{validate_pod_action, validate_pod_timeout};
```

Replace the body of `run_build` from the validation block through the `BUILD_PERMIT` acquisition with:

```rust
    // 1. Validate inputs (before acquiring permit)
    let validated_path: PathBuf =
        validate_project_or_workspace(&params.project_or_workspace, root)?;
    let scheme = validate_scheme(&params.scheme)?;
    let action = validate_action(params.action.as_deref().unwrap_or("build"))?;
    let configuration = match params.configuration.as_deref() {
        Some(c) => Some(validate_configuration(c)?),
        None => None,
    };
    let destination = match params.destination.as_deref() {
        Some(d) => Some(validate_destination(d)?),
        None => None,
    };
    let timeout_secs = validate_timeout(params.timeout_secs)?;
    let pod_action = match params.pod_action.as_deref() {
        Some(a) => Some(validate_pod_action(a)?),
        None => None,
    };
    let pod_timeout_secs = validate_pod_timeout(params.pod_timeout_secs)?;

    // 2. Reserve build_id + paths
    let build_id = uuid::Uuid::new_v4().to_string();
    let xcresult_path = result_dir.join(format!("{build_id}.xcresult"));
    let derived_data_path = result_dir.join("DerivedData").join(&build_id);
    let log_path = log_dir.join(format!("{build_id}.log"));
    std::fs::File::create(&log_path)?;

    // 3. Acquire global build permit (serialized execution)
    let _permit = BUILD_PERMIT
        .acquire()
        .await
        .map_err(|e| crate::error::Error::Internal(format!("semaphore closed: {e}")))?;

    // 4. Pod pre-step (if requested). Runs inside the permit so the whole
    //    pod+build sequence is serialized; a concurrent build cannot see a
    //    half-regenerated Pods.xcworkspace.
    let pod_step: Option<PodStepResult> = if let Some(ref action) = pod_action {
        let pod_params = PodParams {
            project_or_workspace: params.project_or_workspace.clone(),
            action: Some(action.clone()),
            timeout_secs: Some(pod_timeout_secs),
        };
        match run_pod(pod_params, root, log_dir).await {
            Ok(out) => {
                let step = PodStepResult {
                    action: out.action,
                    status: out.status.clone(),
                    exit_code: out.exit_code,
                    duration_secs: out.duration_secs,
                    log_path: out.log_path,
                    stderr_excerpt: out.stderr_excerpt.clone(),
                };
                if out.status != "Succeeded" {
                    // 4a. Pod failed — abort build, register PodFailed record.
                    let build_status = BuildStatus::PodFailed;
                    store.push(BuildRecord {
                        build_id: build_id.clone(),
                        status: build_status.clone(),
                        exit_code: step.exit_code,
                        duration_secs: step.duration_secs,
                        project_or_workspace: validated_path.clone(),
                        scheme: scheme.clone(),
                        xcresult_path: xcresult_path.clone(),
                        log_path: log_path.clone(),
                        result_bundle_written: false,
                        error_count: 0,
                        warning_count: 0,
                        stderr_excerpt: step.stderr_excerpt.clone(),
                        created_at: std::time::SystemTime::now(),
                    });
                    return Ok(BuildOutput {
                        build_id,
                        status: "PodFailed".to_string(),
                        exit_code: step.exit_code,
                        duration_secs: step.duration_secs,
                        xcresult_path: xcresult_path.to_string_lossy().into_owned(),
                        log_path: log_path.to_string_lossy().into_owned(),
                        result_bundle_written: false,
                        error_count: 0,
                        warning_count: 0,
                        truncated_stderr_excerpt: step.stderr_excerpt.clone(),
                        pod: Some(step),
                    });
                }
                Some(step)
            }
            Err(e) => {
                // 4b. Pod step returned an error (e.g. PodfileNotFound, spawn failure).
                let excerpt = Some(e.to_string());
                let step = PodStepResult {
                    action: action.clone(),
                    status: "Failed".to_string(),
                    exit_code: None,
                    duration_secs: 0.0,
                    log_path: log_dir
                        .join(format!("{build_id}-pod.log"))
                        .to_string_lossy()
                        .into_owned(),
                    stderr_excerpt: excerpt.clone(),
                };
                store.push(BuildRecord {
                    build_id: build_id.clone(),
                    status: BuildStatus::PodFailed,
                    exit_code: None,
                    duration_secs: 0.0,
                    project_or_workspace: validated_path.clone(),
                    scheme: scheme.clone(),
                    xcresult_path: xcresult_path.clone(),
                    log_path: log_path.clone(),
                    result_bundle_written: false,
                    error_count: 0,
                    warning_count: 0,
                    stderr_excerpt: excerpt.clone(),
                    created_at: std::time::SystemTime::now(),
                });
                return Ok(BuildOutput {
                    build_id,
                    status: "PodFailed".to_string(),
                    exit_code: None,
                    duration_secs: 0.0,
                    xcresult_path: xcresult_path.to_string_lossy().into_owned(),
                    log_path: log_path.to_string_lossy().into_owned(),
                    result_bundle_written: false,
                    error_count: 0,
                    warning_count: 0,
                    truncated_stderr_excerpt: excerpt,
                    pod: Some(step),
                });
            }
        }
    } else {
        None
    };

    // 5. Build command
    let cmd = build_xcodebuild_command(
        &validated_path,
        &scheme,
        &action,
        configuration.as_deref(),
        destination.as_deref(),
        &xcresult_path,
        &derived_data_path,
    );

    // 6. Run supervised
    let start = std::time::Instant::now();
    let result = run_supervised(cmd, timeout_secs, Some(&log_path)).await?;
    let duration = start.elapsed().as_secs_f64();
```

Then at the end of `run_build`, in the final `Ok(BuildOutput { ... })`, add the `pod` field:

```rust
    Ok(BuildOutput {
        build_id,
        status,
        exit_code,
        duration_secs: duration,
        xcresult_path: xcresult_path.to_string_lossy().into_owned(),
        log_path: log_path.to_string_lossy().into_owned(),
        result_bundle_written,
        error_count,
        warning_count,
        truncated_stderr_excerpt,
        pod: pod_step,
    })
```

> **Note on the existing step-numbering comments:** the original `run_build` had numbered comments `// 4. Build command` through `// 12. Register in store`. After inserting the pod step as `// 4`, renumber the remaining comments `// 5` through `// 13` to keep them sequential. The logic in those later steps is unchanged.

- [ ] **Step 3: Update all existing `BuildParams` construction sites to include the new fields**

The struct change in Step 1 breaks every existing `BuildParams { ... }` literal. There are 5 sites across 3 files — each needs `pod_action: None, pod_timeout_secs: None,` added before the closing brace.

**Site 1 — `crates/xcode-mcp/src/cli.rs`**, in the `DebugCommand::Build` arm of `run_debug` (around line 100). The construction currently ends with `timeout_secs,`; add the two fields after it:

```rust
            let params = BuildParams {
                project_or_workspace: project,
                scheme,
                action: Some(action),
                configuration,
                destination,
                timeout_secs,
                pod_action: None,
                pod_timeout_secs: None,
            };
```

**Sites 2–4 — `crates/xcode-mcp-core/tests/live/build.rs`**, three constructions (in `build_succeeds_for_valid_app` ~line 47, `build_fails_for_broken_app` ~line 74, `build_times_out` ~line 101). Each ends with `timeout_secs: Some(...)`. Add after that line in each:

```rust
        pod_action: None,
        pod_timeout_secs: None,
```

**Site 5 — `crates/xcode-mcp-core/tests/live/errors.rs`**, one construction (~line 34, ends with `timeout_secs: Some(300)`). Add after it:

```rust
        pod_action: None,
        pod_timeout_secs: None,
```

- [ ] **Step 4: Verify both crates compile (including live tests)**

Run: `cargo check --workspace --features xcode-mcp-core/live-xcode`
Expected: compiles with no errors. This confirms all 5 `BuildParams` construction sites are updated. (May have unused-import warnings for `validate_pod_action`/`validate_pod_timeout` if the linter is strict — those are used in `run_build` so should be clean.)

- [ ] **Step 5: Extend `xcode_commands.rs` test to confirm `BuildParams` accepts new fields**

Append to `crates/xcode-mcp-core/tests/xcode_commands.rs`:

```rust
#[test]
fn build_params_accepts_pod_action_fields() {
    use xcode_mcp_core::xcode::BuildParams;
    let params = BuildParams {
        project_or_workspace: "/tmp/App.xcodeproj".into(),
        scheme: "App".into(),
        action: Some("build".into()),
        configuration: None,
        destination: None,
        timeout_secs: None,
        pod_action: Some("install".into()),
        pod_timeout_secs: Some(300),
    };
    assert_eq!(params.pod_action.as_deref(), Some("install"));
    assert_eq!(params.pod_timeout_secs, Some(300));
}
```

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test -p xcode-mcp-core --test xcode_commands`
Expected: PASS (existing tests unchanged + new test passes)

- [ ] **Step 7: Commit**

```bash
git add crates/xcode-mcp-core/src/xcode.rs crates/xcode-mcp/src/cli.rs crates/xcode-mcp-core/tests/live/build.rs crates/xcode-mcp-core/tests/live/errors.rs crates/xcode-mcp-core/tests/xcode_commands.rs
git commit -m "feat: integrate pod pre-step into run_build with PodFailed abort"
```

---

## Task 5: Wire `xcode_pod` tool + `pod_action` flag into the MCP server

**Files:**
- Modify: `crates/xcode-mcp/src/server.rs`

**Interfaces:**
- Consumes: `run_pod`, `PodParams` (from Task 3), `BuildParams` new fields (from Task 4)
- Produces: `xcode_pod` tool in `tools/list`; `pod_action` / `pod_timeout_secs` params on `xcode_build`

- [ ] **Step 1: Add the `xcode_pod` tool schema and extend the `xcode_build` schema**

Edit `crates/xcode-mcp/src/server.rs`. In `make_tool_list()`, add a `pod_schema` and the new optional fields to `build_schema`. Replace the `build_schema` definition and add `pod_schema` after `errors_schema`:

Replace the existing `build_schema` block:

```rust
    let build_schema: serde_json::Map<String, serde_json::Value> = serde_json::from_str(
        r#"{
            "type": "object",
            "properties": {
                "project_or_workspace": { "type": "string" },
                "scheme": { "type": "string" },
                "action": { "type": "string", "enum": ["build", "clean", "clean+build"] },
                "configuration": { "type": "string", "enum": ["Debug", "Release"] },
                "destination": { "type": "string" },
                "timeout_secs": { "type": "integer" },
                "pod_action": { "type": "string", "enum": ["install", "update"] },
                "pod_timeout_secs": { "type": "integer" }
            },
            "required": ["project_or_workspace", "scheme"]
        }"#,
    )
    .unwrap();
```

Add after the `errors_schema` block:

```rust
    let pod_schema: serde_json::Map<String, serde_json::Value> = serde_json::from_str(
        r#"{
            "type": "object",
            "properties": {
                "project_or_workspace": { "type": "string" },
                "action": { "type": "string", "enum": ["install", "update"] },
                "timeout_secs": { "type": "integer" }
            },
            "required": ["project_or_workspace", "action"]
        }"#,
    )
    .unwrap();
```

Add the `xcode_pod` tool to the returned `vec![...]`:

```rust
    vec![
        Tool::new(
            "xcode_list_schemes",
            "List schemes, targets, and configurations for an Xcode project or workspace",
            list_schema,
        ),
        Tool::new(
            "xcode_build",
            "Run an xcodebuild and return build_id + status",
            build_schema,
        ),
        Tool::new(
            "xcode_get_build_errors",
            "Get structured build diagnostics for a build",
            errors_schema,
        ),
        Tool::new(
            "xcode_pod",
            "Run `pod install` or `pod update` in the project's directory to refresh Pods.xcworkspace references before building",
            pod_schema,
        ),
    ]
```

- [ ] **Step 2: Wire `xcode_pod` dispatch + pass new fields to `BuildParams`**

Edit `crates/xcode-mcp/src/server.rs`. In `dispatch_tool`, update the imports at the top of the file:

```rust
use xcode_mcp_core::{
    diagnostic::load_diagnostics,
    pod::{run_pod, PodParams},
    scheme::list_schemes,
    store::BuildStore,
    xcode::{run_build, BuildParams},
};
```

In the `"xcode_build"` arm of `dispatch_tool`, add the two new fields to `BuildParams`:

```rust
            "xcode_build" => {
                let project = get_string_arg(&args, "project_or_workspace")?;
                let scheme = get_string_arg(&args, "scheme")?;
                let build_params = BuildParams {
                    project_or_workspace: project,
                    scheme,
                    action: get_optional_string_arg(&args, "action"),
                    configuration: get_optional_string_arg(&args, "configuration"),
                    destination: get_optional_string_arg(&args, "destination"),
                    timeout_secs: get_optional_u64_arg(&args, "timeout_secs").map(|n| n as u32),
                    pod_action: get_optional_string_arg(&args, "pod_action"),
                    pod_timeout_secs: get_optional_u64_arg(&args, "pod_timeout_secs").map(|n| n as u32),
                };
                run_build(
                    build_params,
                    &self.root,
                    &self.result_dir,
                    &self.log_dir,
                    &self.store,
                )
                .await
                .map(|output| serde_json::to_value(&output).unwrap_or(serde_json::Value::Null))
                .map_err(|e| e.to_string())
            }
```

Add a new arm for `"xcode_pod"` (before the `_ =>` arm):

```rust
            "xcode_pod" => {
                let project = get_string_arg(&args, "project_or_workspace")?;
                let action = get_string_arg(&args, "action")?;
                let pod_params = PodParams {
                    project_or_workspace: project,
                    action: Some(action),
                    timeout_secs: get_optional_u64_arg(&args, "timeout_secs").map(|n| n as u32),
                };
                run_pod(pod_params, &self.root, &self.log_dir)
                    .await
                    .map(|output| serde_json::to_value(&output).unwrap_or(serde_json::Value::Null))
                    .map_err(|e| e.to_string())
            }
```

- [ ] **Step 3: Verify the binary compiles**

Run: `cargo check -p xcode-mcp`
Expected: compiles with no errors

- [ ] **Step 4: Run the full test suite**

Run: `cargo test --workspace`
Expected: all existing tests pass (no new tests here — server wiring is covered by the live integration tests in Task 7)

- [ ] **Step 5: Commit**

```bash
git add crates/xcode-mcp/src/server.rs
git commit -m "feat: expose xcode_pod tool and pod_action flag on xcode_build"
```

---

## Task 6: Debug CLI `pod` subcommand

**Files:**
- Modify: `crates/xcode-mcp/src/cli.rs`

**Interfaces:**
- Consumes: `run_pod`, `PodParams` (from Task 3)

- [ ] **Step 1: Add `Pod` variant to `DebugCommand`**

Edit `crates/xcode-mcp/src/cli.rs`. Add a new variant to `DebugCommand`:

```rust
    /// Run pod install or pod update in the project's directory
    Pod {
        #[arg(long)]
        project: String,
        #[arg(long)]
        action: String,
        #[arg(long)]
        timeout_secs: Option<u32>,
        #[arg(long)]
        root: Option<String>,
        #[arg(long)]
        log_dir: Option<PathBuf>,
    },
```

- [ ] **Step 2: Update the `use` import and add the dispatch arm**

Edit `crates/xcode-mcp/src/cli.rs`. Update the imports from `xcode_mcp_core`:

```rust
use xcode_mcp_core::{
    diagnostic::load_diagnostics,
    pod::{run_pod, PodParams},
    scheme::list_schemes,
    store::BuildStore,
    xcode::{run_build, BuildParams},
};
```

Add the dispatch arm in `run_debug` (after `DebugCommand::BuildErrors` and before `DebugCommand::InspectorHelp`):

```rust
        DebugCommand::Pod {
            project,
            action,
            timeout_secs,
            root,
            log_dir,
        } => {
            let root = resolve_root(&root, &project)?;
            let log_dir = resolve_dir(log_dir, &root, ".xcode-mcp-logs")?;
            std::fs::create_dir_all(&log_dir)?;
            let params = PodParams {
                project_or_workspace: project,
                action: Some(action),
                timeout_secs,
            };
            let output = run_pod(params, &root, &log_dir).await?;
            println!("{}", serde_json::to_string_pretty(&output)?);
        }
```

- [ ] **Step 3: Verify the binary compiles**

Run: `cargo check -p xcode-mcp`
Expected: compiles with no errors

- [ ] **Step 4: Smoke-test the CLI help**

Run: `cargo run -p xcode-mcp -- debug --help`
Expected: prints help including the `pod` subcommand

- [ ] **Step 5: Commit**

```bash
git add crates/xcode-mcp/src/cli.rs
git commit -m "feat: add `debug pod` subcommand for manual pod testing"
```

---

## Task 7: Live integration tests (feature-gated)

**Files:**
- Create: `crates/xcode-mcp-core/tests/live/pod.rs`
- Modify: `crates/xcode-mcp-core/Cargo.toml`

**Interfaces:**
- Consumes: `run_pod`, `PodParams` (from Task 3), `run_build`, `BuildParams` (from Task 4), `list_schemes` (existing)

- [ ] **Step 1: Register the live pod test target in `Cargo.toml`**

Edit `crates/xcode-mcp-core/Cargo.toml`. Add a new `[[test]]` block after the existing `errors` block:

```toml
[[test]]
name = "pod"
path = "tests/live/pod.rs"
required-features = ["live-xcode"]
```

- [ ] **Step 2: Write the live test file**

Create `crates/xcode-mcp-core/tests/live/pod.rs`:

```rust
#![cfg(feature = "live-xcode")]

use std::env;
use std::path::PathBuf;
use xcode_mcp_core::{
    pod::{run_pod, PodParams},
    store::BuildStore,
    xcode::{run_build, BuildParams},
};

fn skip_if_not_enabled() -> bool {
    env::var("XCODE_MCP_LIVE_TESTS").is_err()
}

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/MiniApp")
        .canonicalize()
        .unwrap()
}

#[tokio::test]
async fn pod_returns_podfile_not_found_for_clean_project() {
    if skip_if_not_enabled() {
        eprintln!("skipping: set XCODE_MCP_LIVE_TESTS=1");
        return;
    }
    let root = fixture_root();
    let proj = root.join("MiniApp.xcodeproj");
    let log_dir = root.join(".xcode-mcp-logs");
    std::fs::create_dir_all(&log_dir).unwrap();
    let params = PodParams {
        project_or_workspace: proj.to_string_lossy().into_owned(),
        action: Some("install".into()),
        timeout_secs: None,
    };
    let result = run_pod(params, &root, &log_dir).await;
    assert!(result.is_err(), "MiniApp has no Podfile — expected error");
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("no Podfile found"),
        "expected PodfileNotFound, got: {msg}"
    );
}

#[tokio::test]
async fn build_with_pod_action_aborts_when_no_podfile() {
    if skip_if_not_enabled() {
        eprintln!("skipping: set XCODE_MCP_LIVE_TESTS=1");
        return;
    }
    let root = fixture_root();
    let proj = root.join("MiniApp.xcodeproj");
    let result_dir = root.join(".xcode-mcp-results");
    let log_dir = root.join(".xcode-mcp-logs");
    std::fs::create_dir_all(&result_dir).unwrap();
    std::fs::create_dir_all(&log_dir).unwrap();
    let store = BuildStore::new(32);
    let params = BuildParams {
        project_or_workspace: proj.to_string_lossy().into_owned(),
        scheme: "MiniApp".into(),
        action: Some("build".into()),
        configuration: None,
        destination: None,
        timeout_secs: None,
        pod_action: Some("install".into()),
        pod_timeout_secs: None,
    };
    let output = run_build(params, &root, &result_dir, &log_dir, &store)
        .await
        .expect("run_build should not hard-error on PodFailed");
    assert_eq!(output.status, "PodFailed");
    assert!(!output.result_bundle_written);
    assert!(output.pod.is_some(), "pod field must be present");
    let pod = output.pod.unwrap();
    assert_eq!(pod.status, "Failed");
    assert!(pod.stderr_excerpt.is_some());
    assert!(output.error_count == 0);
}
```

- [ ] **Step 3: Verify the test compiles (do not run live)**

Run: `cargo test -p xcode-mcp-core --test pod --features live-xcode --no-run`
Expected: compiles successfully

- [ ] **Step 4: Run the live tests (only if user opts in)**

Run: `XCODE_MCP_LIVE_TESTS=1 cargo test -p xcode-mcp-core --test pod --features live-xcode -- --test-threads=1`
Expected: both tests pass (they test the PodfileNotFound / PodFailed path against the existing MiniApp fixture which has no Podfile — no CocoaPods install required for these specific tests)

- [ ] **Step 5: Commit**

```bash
git add crates/xcode-mcp-core/Cargo.toml crates/xcode-mcp-core/tests/live/pod.rs
git commit -m "test: add feature-gated live tests for pod PodfileNotFound path"
```

---

## Task 8: README documentation

**Files:**
- Modify: `README.md`

- [ ] **Step 1: Add `xcode_pod` to the "Tools Reference" section**

Edit `README.md`. After the `### xcode_get_build_errors` subsection (before the `## Testing` section), insert:

```markdown
### `xcode_pod`

Runs `pod install` or `pod update` in the project's parent directory to refresh `Pods.xcworkspace` references before building. Use this when local CocoaPods files (Podfile, Podfile.lock) have changed.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `project_or_workspace` | string | yes | Path to `.xcodeproj` or `.xcworkspace` (parent dir must contain a `Podfile`) |
| `action` | `"install"` \| `"update"` | yes | `install` respects `Podfile.lock`; `update` bumps pod versions |
| `timeout_secs` | integer | no | Default 600, max 3600 |

Returns a `PodOutput` with `run_id`, `status` (`Succeeded` / `Failed` / `TimedOut`), `exit_code`, `duration_secs`, `log_path`, and (on failure) `stderr_excerpt`.
```

- [ ] **Step 2: Add `pod_action` / `pod_timeout_secs` rows to the `xcode_build` table**

Edit `README.md`. In the `### xcode_build` param table, append after the `timeout_secs` row:

```markdown
| `pod_action` | `"install"` \| `"update"` | no | When set, run pod first in the project's parent dir. On pod failure, the build is aborted with `status: "PodFailed"` and no xcodebuild runs. |
| `pod_timeout_secs` | integer | no | Default 600, max 3600 (separate from `timeout_secs` because pod is network-bound) |
```

- [ ] **Step 3: Add a `# Pod` example to the "Debug CLI" section**

Edit `README.md`. In the `## Debug CLI` section, after the `# Get build errors` block, add:

```markdown
# Pod install (refresh Pods.xcworkspace after Podfile edit)
xcode-mcp debug pod \
  --project /path/App.xcodeproj \
  --action install
```

- [ ] **Step 4: Commit**

```bash
git add README.md
git commit -m "docs: document xcode_pod tool and pod_action build flag"
```

---

## Task 9: Final verification

- [ ] **Step 1: Clean build with no warnings**

Run: `cargo build --release`
Expected: compiles with no warnings

- [ ] **Step 2: Full unit test suite**

Run: `cargo test --workspace`
Expected: all tests pass

- [ ] **Step 3: Clippy**

Run: `cargo clippy --workspace --all-targets -- -D warnings`
Expected: no warnings

- [ ] **Step 4: Verify tools/list via Inspector (manual)**

Run: `npx @modelcontextprotocol/inspector ./target/release/xcode-mcp serve`
Expected: `tools/list` returns 4 tools including `xcode_pod`; `xcode_build` schema includes `pod_action` and `pod_timeout_secs`

- [ ] **Step 5: Final commit (if any cleanup needed)**

Only if previous steps surfaced issues requiring fixes.
