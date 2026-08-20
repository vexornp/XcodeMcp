# Pod install/update before Xcode build — Design Spec

- **Date:** 2026-08-20
- **Status:** Draft (pending user review)
- **Owner:** peiyan_wang
- **Implementation tracker:** `docs/superpowers/plans/` (to be created by `writing-plans`)
- **Depends on:** existing `xcode_build` / `xcode_list_schemes` tools (see `2026-08-19-xcode-mcp-server-design.md`)

## 1. Purpose

Add the ability to run `pod install` or `pod update` from the MCP server, so that when local CocoaPods files are edited (pod added/removed, Podfile.lock drift, new repo checkout), the `Pods.xcworkspace` / `Pods.xcworkspace.xcworkspace` references can be refreshed **before** an `xcodebuild` invocation. Without this, a build after a Podfile edit references stale pod targets and fails with confusing linker/scheme errors.

Scope is deliberately narrow: full-file `pod install` / `pod update` only. No per-pod updates, no `--no-repo-update` flag plumbing, no lock-diff reporting.

## 2. Non-Goals

- No per-pod name args (`pod update Alamofire`).
- No `--no-repo-update` / `--verbose` / `--silent` flag passthrough (fixed flag surface only; see §8).
- No `Podfile.lock` diffing or pod-version reporting.
- No `pod outdated` / `pod repo update` / `pod deintegrate`.
- No new CocoaPods test fixture committed to the repo (live `pod` tests stay opt-in via the existing `live-xcode` feature gate; see §9).
- No MCP progress notifications for the pod step (timeout-only abort, same as builds).
- No concurrent pod + build on the same project (serialized via `BUILD_PERMIT`; see §7).

## 3. Requirements

| # | Requirement | Met by |
|---|---|---|
| 1 | New `xcode_pod` MCP tool | §6 |
| 2 | `xcode_pod` supports `install` + `update` actions | §6, §8 |
| 3 | New optional `pod_action` param on `xcode_build` | §7 |
| 4 | New optional `pod_timeout_secs` param on `xcode_build` | §7 |
| 5 | Pod failure aborts the build (no xcodebuild runs) | §7 |
| 6 | Distinct `PodFailed` build status when build aborted by pod | §7, §10 |
| 7 | Pod step serialized with builds via `BUILD_PERMIT` | §7 |
| 8 | No shell invocation — `Command::new("pod")` | §8 |
| 9 | Fixed pod flag surface (action + `--no-ansi` only) | §8 |
| 10 | Podfile presence check before running pod | §6 |
| 11 | Pod step logged to `log_dir/{run_id}.pod.log` | §6, §7 |
| 12 | Pod timeout enforced via process-group kill | §6, §7 |
| 13 | Unit tests for pod command construction + validation | §9 |
| 14 | Debug CLI `pod` subcommand | §11 |
| 15 | README "Tools Reference" updated | §12 |

## 4. Toolchain additions

- **CocoaPods** must be installed and on `PATH` (`gem install cocoapods` or `brew install cocoapods`). The server does not install or manage CocoaPods; if `pod` is missing, the spawn fails and the caller gets a clear `XcodeSpawnFailed`-style error.

No new Rust crate dependencies. Reuses existing `tokio::process::Command`, `run_supervised`, `validate_*`, and `BuildStore`.

## 5. Module layout

New module in `xcode-mcp-core`, mirroring the existing per-domain pattern (`xcode.rs`, `scheme.rs`, `diagnostic.rs`):

```
crates/xcode-mcp-core/src/
├── pod.rs           # NEW: build_pod_command, run_pod, PodOutput, PodStepResult
├── xcode.rs         # MODIFIED: run_build gains pod pre-step (§7)
├── security.rs      # MODIFIED: add validate_pod_action, validate_pod_timeout
├── error.rs         # MODIFIED: add PodFailed variant on Error? (no — see §10)
├── store.rs         # MODIFIED: add BuildStatus::PodFailed variant
├── lib.rs           # MODIFIED: pub mod pod; re-export PodOutput, PodStepResult
└── ...
```

Rejected alternatives:
- **Extend `xcode.rs`:** mixes two external tools (xcodebuild vs cocoapods) in one file; `xcode.rs` is already the largest source file. Pod is a distinct domain.
- **Inline in `server.rs`:** violates the core/bin separation — all logic currently lives in core; `server.rs` only dispatches.

## 6. New tool: `xcode_pod`

### 6.1 MCP schema

| Parameter | Type | Required | Description |
|---|---|---|---|
| `project_or_workspace` | string | yes | Path to `.xcodeproj` or `.xcworkspace` (used to locate the Podfile dir = its parent) |
| `action` | `"install"` \| `"update"` | yes | `pod install` (respect `Podfile.lock`) or `pod update` (bump versions, ignore lock) |
| `timeout_secs` | integer | no | Default 600, max 3600 |

### 6.2 Behavior

1. Validate `project_or_workspace` against root via existing `validate_project_or_workspace`. Returns canonical path.
2. Derive `working_dir = validated_path.parent()`. Because `validated_path` is under-or-equal root and root is canonical, `working_dir` is also under-or-equal root (parent of a root-bounded path is root-bounded).
3. Require `Podfile` in `working_dir`. If absent → return `Error::PodfileNotFound { working_dir }`. Do **not** fall back to a recursive ancestor search — pod is only run when the project clearly uses CocoaPods.
4. Generate `run_id = uuid::Uuid::new_v4()`.
5. Build command via `build_pod_command(working_dir, action)` (§8).
6. Run via existing `run_supervised(cmd, timeout_secs, Some(&log_path))` where `log_path = log_dir.join(format!("{run_id}.pod.log"))`.
7. Determine status: `TimedOut` if `result.timed_out`; else `Succeeded` if `exit_code == Some(0)`; else `Failed`.
8. Compute `stderr_excerpt`: last 2 KB of stderr, char-boundary-snapped (same logic as `run_build`).

### 6.3 Output: `PodOutput`

Serialized as PascalCase JSON to match `BuildOutput`:

```rust
pub struct PodOutput {
    pub run_id: String,
    pub action: String,          // "install" | "update"
    pub working_dir: String,
    pub status: String,          // "Succeeded" | "Failed" | "TimedOut"
    pub exit_code: Option<i32>,
    pub duration_secs: f64,
    pub log_path: String,
    pub stderr_excerpt: Option<String>,
}
```

No `BuildStore` involvement — `xcode_pod` is not a build and does not produce an `.xcresult`. The caller can re-invoke `xcode_build` after a successful `xcode_pod`.

## 7. Build flag: `pod_action` on `xcode_build`

### 7.1 New params

| Parameter | Type | Description |
|---|---|---|
| `pod_action` | `"install"` \| `"update"` | When set, run pod first in the project's parent dir |
| `pod_timeout_secs` | integer | Default 600, max 3600 (separate from build `timeout_secs` because pod is network-bound and typically slower than a compile) |

### 7.2 `BuildParams` additions

```rust
pub struct BuildParams {
    // ...existing fields...
    pub pod_action: Option<String>,       // NEW
    pub pod_timeout_secs: Option<u32>,    // NEW
}
```

### 7.3 Modified `run_build` flow

The existing 12-step `run_build` gains a pod pre-step inserted **after** the `BUILD_PERMIT` is acquired, so the whole pod+build sequence is serialized under one permit hold. New ordering:

1. Validate inputs (existing) — plus `validate_pod_action` / `validate_pod_timeout` when `pod_action` is set.
2. Reserve `build_id` + paths (existing).
3. Acquire `BUILD_PERMIT` (existing step 3, unchanged position).
4. **NEW:** If `pod_action` set, run pod step (see §7.4) **while still holding the permit.** On pod failure/timeout, release the permit and return early with `status = "PodFailed"` (see §7.5). On pod success, proceed to step 5 (permit still held).
5. Build xcodebuild command + run supervised (existing steps 4–5).
6. … (existing steps 6–12 unchanged; permit released when `_permit` drops at end of scope).

> **Sequencing note:** the pod step runs **outside** the `BUILD_PERMIT` hold in the standalone `xcode_pod` tool (§6), but **inside** it when invoked via `xcode_build`'s `pod_action` flag. This is intentional: in the combined flow, the caller has already declared intent to build, so holding the permit across pod+build prevents another build from starting mid-pod and seeing a half-regenerated workspace. In the standalone flow, the caller is explicitly managing sequencing and `BUILD_PERMIT` is not relevant (pod is not a build).

### 7.4 Pod step inside `run_build`

Calls the same core `run_pod` function as the standalone tool (§6.2). `run_pod` returns `Result<PodOutput>`:
- `Ok(PodOutput { status: "Succeeded", .. })` → synthesize `PodStepResult { status: "Succeeded", .. }`, proceed to xcodebuild.
- `Ok(PodOutput { status: "Failed" | "TimedOut", .. })` → synthesize `PodStepResult` with the failing status, abort build (§7.5).
- `Err(e)` (e.g. `PodfileNotFound`, spawn failure) → synthesize `PodStepResult { status: "Failed", exit_code: None, stderr_excerpt: Some(e.to_string()), .. }`, abort build (§7.5).

Sub-id grouping:
- `run_id` is generated as a sub-id of the build: `format!("{build_id}-pod")` so logs are grouped.
- `log_path = log_dir.join(format!("{build_id}-pod.log"))`.

`PodStepResult` is a trimmed `PodOutput` (no `working_dir`/`run_id`, since those are derivable from the build context):

```rust
pub struct PodStepResult {
    pub action: String,
    pub status: String,          // "Succeeded" | "Failed" | "TimedOut"
    pub exit_code: Option<i32>,
    pub duration_secs: f64,
    pub log_path: String,
    pub stderr_excerpt: Option<String>,
}
```

### 7.5 Pod failure → abort build (per user decision)

If the pod step produces a `PodStepResult` with `status != "Succeeded"` (covering both `Ok(Failed|TimedOut)` and `Err(...)` from §7.4):
- Do **not** run xcodebuild.
- Do **not** write an `.xcresult`.
- Register a `BuildRecord` with:
  - `status = BuildStatus::PodFailed`
  - `exit_code = pod_step.exit_code`
  - `result_bundle_written = false`
  - `error_count = 0`, `warning_count = 0`
  - `stderr_excerpt = pod_step.stderr_excerpt` (so `xcode_get_build_errors` on this `build_id` surfaces the pod failure, not an empty build log)
- Return `BuildOutput` with:
  - `status = "PodFailed"`
  - `result_bundle_written = false`
  - `error_count = 0`, `warning_count = 0`
  - `truncated_stderr_excerpt = pod_step.stderr_excerpt`
  - `pod = Some(pod_step)` (success or failure — caller sees the pod details)

### 7.6 `BuildOutput` addition

```rust
pub struct BuildOutput {
    // ...existing fields...
    pub pod: Option<PodStepResult>,   // NEW — present when pod_action was set
}
```

For backward compatibility, `pod` is `None` (serialized as `null`) on all builds that did not set `pod_action`.

## 8. Security model

Mirrors the existing `xcodebuild` security posture (see `2026-08-19-xcode-mcp-server-design.md` §11):

- **No shell invocation.** `Command::new("pod")` — `pod` resolved from `PATH`, same pattern as `Command::new("xcrun")`.
- **Fixed flag surface.** Only `action` (`install` | `update`) and `--no-ansi` are ever passed. No `extra_args` passthrough. `--no-ansi` keeps logs parseable and prevents terminal control sequences from polluting captured stderr.
- **`current_dir` bounded.** Set to `validated_path.parent()`, which is under-or-equal root (see §6.2 step 2). Pod cannot operate outside root.
- **`action` charset-validated.** New `validate_pod_action(s: &str) -> Result<String>` accepts exactly `"install"` or `"update"`, rejects everything else. Mirrors `validate_action`.
- **`timeout_secs` bounded.** New `validate_pod_timeout(t: Option<u32>) -> Result<u32>`: `None → 600`, `Some(v) if 1..=3600 → v`, else `Err`. Mirrors `validate_timeout` (which is 1..=7200 for builds; pod gets a smaller ceiling because it's network-bound and rarely exceeds 10 min).
- **Process-group kill on timeout.** `run_supervised` already does `setsid()` + `kill(-pgid, SIGTERM→SIGKILL)`, so a hung `pod update` (e.g. stuck on CDN) cannot orphan Ruby/CocoaPods subprocesses.

### 8.1 `build_pod_command`

```rust
pub fn build_pod_command(working_dir: &Path, action: &str) -> Command {
    let mut cmd = Command::new("pod");
    cmd.arg(action).arg("--no-ansi");
    cmd.current_dir(working_dir);
    cmd
}
```

No `xcrun` wrapper — `pod` is not an Xcode tool. `current_dir` is set on the `Command` (not via a `cd` shell command), consistent with how the rest of the codebase avoids shell invocation.

## 9. Testing

### 9.1 New unit tests (`crates/xcode-mcp-core/tests/pod_commands.rs`)

Mirrors `tests/xcode_commands.rs`. All assert against `Command` args without spawning:

| Test | Asserts |
|---|---|
| `pod_command_install` | program is `pod`; args contain `install`, `--no-ansi`; `current_dir` set to working dir |
| `pod_command_update` | args contain `update` |
| `pod_command_no_shell` | `cmd.as_std().get_program() == "pod"` (not `/bin/sh -c ...`) |
| `pod_command_no_extra_flags` | args are exactly `[action, "--no-ansi"]` (no `--verbose`, no passthrough) |

### 9.2 Extended security tests (`crates/xcode-mcp-core/tests/security.rs`)

| Test | Asserts |
|---|---|
| `pod_action_accepts_install_update` | `validate_pod_action("install")` / `"update"` → Ok |
| `pod_action_rejects_others` | `"outdated"`, `"install --verbose"`, `""`, `"INSTALL"` → Err |
| `pod_timeout_defaults_and_validates` | `None → 600`; `Some(60) → 60`; `Some(3600) → 3600`; `Some(0)` / `Some(3601)` → Err |

### 9.3 Podfile-presence logic

Covered implicitly by `run_pod` integration (§9.4). No separate unit test — the check is a single `working_dir.join("Podfile").exists()`.

### 9.4 Live integration tests (opt-in)

Gated behind `live-xcode` feature **and** `XCODE_MCP_LIVE_TESTS=1` env var, matching the existing double-gate. **No new CocoaPods fixture is committed** — these tests require CocoaPods installed locally and are skipped in CI without it.

A new `tests/live/pod.rs` (feature-gated) covers:
- `xcode_pod install` on a temp project with a minimal `Podfile` (no pods, or one trivial pod) → `Succeeded`.
- `xcode_build` with `pod_action=install` on the same → `pod` field present, build proceeds.
- `xcode_build` with `pod_action=update` on a project with **no** `Podfile` → `PodFailed` / `PodfileNotFound`.

These are marked `#[ignore]` by default and require an env var to run, to avoid surprising local contributors.

## 10. Error handling

### 10.1 New `Error` variant

```rust
#[error("no Podfile found next to {working_dir}")]
PodfileNotFound { working_dir: PathBuf },
```

Used by both `run_pod` (standalone) and the pod step inside `run_build`. In the standalone `xcode_pod` tool, this surfaces as a normal MCP tool error (`isError=true` with the message). In `xcode_build`'s combined flow, it converts to a `PodFailed` build status (see §7.5) rather than aborting the whole MCP call — so the caller still gets a `build_id` they can query.

### 10.2 New `BuildStatus` variant

```rust
pub enum BuildStatus {
    Succeeded,
    Failed,
    TimedOut,
    Canceled,
    Unknown,
    PodFailed,   // NEW
}
```

`PodFailed` is serialized as `"PodFailed"` (PascalCase, matching the existing variants). It is distinct from `Failed` so the LLM caller knows the build itself never ran — there is no `.xcresult` to inspect, and the failure is in the pod step, not the compile/link step.

### 10.3 `BuildStore` impact

None beyond the new enum variant. `BuildRecord` already carries `stderr_excerpt`; pod-failure records reuse that field to carry the pod stderr. No schema migration needed (the store is in-memory only, not persisted).

## 11. Debug CLI

New `DebugCommand::Pod` subcommand, mirroring `DebugCommand::Build`:

```
xcode-mcp debug pod \
  --project /path/App.xcodeproj \
  --action install \
  [--timeout-secs 600] \
  [--root /path] \
  [--log-dir /path]
```

Resolves root/log_dir via the same `resolve_root` / `resolve_dir` helpers. Calls `run_pod` and prints `PodOutput` as pretty JSON. No `--result-dir` (pod produces no result bundle).

## 12. Documentation

README "Tools Reference" gains:

- A new `### xcode_pod` subsection with the §6.1 param table + a one-paragraph behavior note.
- Two new rows in the `### xcode_build` param table (`pod_action`, `pod_timeout_secs`).
- A note in the `### xcode_build` section explaining the abort-on-pod-failure behavior and the `PodFailed` status.

README "Debug CLI" gains a `# Pod` example block.

No changes to the "Security Model" or "Architecture" sections beyond a one-line mention that `pod` is also invoked via `Command::new` (no shell).

## 13. Open questions

None. All three design questions were resolved during brainstorming:
1. API shape → Both (standalone tool + build flag).
2. Pod command scope → Both `install` + `update`.
3. Pod failure behavior → Abort the build.

## 14. Acceptance criteria

- [ ] `cargo build --release` succeeds with no warnings.
- [ ] `cargo test` passes (all existing + new unit tests).
- [ ] `xcode_pod` appears in `tools/list` with the §6.1 schema.
- [ ] `xcode_build` schema includes the two new optional params.
- [ ] `xcode_pod install` on a project with a Podfile returns `status: "Succeeded"` (live test, opt-in).
- [ ] `xcode_build` with `pod_action=install` and a valid Podfile runs pod then builds; `pod` field present in output.
- [ ] `xcode_build` with `pod_action=update` and **no** Podfile returns `status: "PodFailed"`, no `.xcresult` written, no xcodebuild spawned.
- [ ] `xcode_get_build_errors` on a `PodFailed` build surfaces the pod stderr excerpt.
- [ ] `xcode-mcp debug pod --project ... --action install` runs and prints JSON.
- [ ] README "Tools Reference" updated.
