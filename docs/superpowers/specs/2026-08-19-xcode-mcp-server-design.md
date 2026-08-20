# Xcode MCP Server — Design Spec

- **Date:** 2026-08-19
- **Status:** Draft (pending user review)
- **Owner:** peiyan_wang
- **Implementation tracker:** `docs/superpowers/plans/` (to be created by `writing-plans`)

## 1. Purpose

A local MCP (Model Context Protocol) server, written in Rust, that drives `xcodebuild` against `.xcodeproj` / `.xcworkspace` bundles and parses build-failure diagnostics into structured data an LLM can act on.

Scope is deliberately narrow: list schemes, run a build, return structured diagnostics. No testing, archiving, signing, or running of the built product.

## 2. Non-Goals

- No `xcodebuild test`, `archive`, `install`, or `build-for-testing`.
- No code signing / provisioning profile management.
- No running of the built app on a simulator or device.
- No concurrent builds (serialized; see §9).
- No MCP progress notifications or cancellation in v1 (timeout-only abort; see §9).
- No remote / networked operation — local stdio only.
- No `xcodebuild` flag passthrough (no `extra_args`) — fixed flag surface (see §8, §11).

## 3. Requirements (15 items)

| # | Requirement | Met by |
|---|---|---|
| 1 | rmcp 3.1.1 MCP server | §6 (deps), §10 (server) |
| 2 | stdio transport | §10 |
| 3 | `xcode_list_schemes` tool | §7 |
| 4 | `xcode_build` tool | §8 |
| 5 | `xcode_get_build_errors` tool | §9 |
| 6 | `.xcodeproj` and `.xcworkspace` support | §7, §11 (validation) |
| 7 | xcodebuild stdout/stderr capture | §8 (tee'd to log file + in-memory excerpt) |
| 8 | `.xcresult` generation | §8 (`-resultBundlePath` always set) |
| 9 | modern `xcresulttool get build-results` parsing | §9 (result_bundle.rs) |
| 10 | compiler/linker/build diagnostics extraction | §9 (hybrid xcresult + stderr) |
| 11 | build timeout + process termination | §8 (process-group kill on timeout) |
| 12 | workspace-root security via `XCODE_MCP_ROOT` | §11 |
| 13 | no shell invocation / no arbitrary command execution | §11, §8 |
| 14 | unit tests for diagnostic and scheme parsing | §12 |
| 15 | MCP Inspector instructions | §13 |

## 4. Toolchain

- **Rust:** 1.97.1 (MSRV 1.97), edition 2021.
- **Xcode:** 26.3 (any version with modern `xcrun xcresulttool get build-results --format json`, schema 0.1.0+).
- **OS:** macOS only (depends on `xcrun`, `xcodebuild`, `xcresulttool`).

## 5. Workspace Layout

Cargo workspace: one lib crate (all logic) + one bin crate (thin rmcp wiring + a debug CLI).

```
XcodeMcp/
├── Cargo.toml                      # [workspace], members = crates/*
├── README.md
├── crates/
│   ├── xcode-mcp-core/             # lib: all logic, fully unit-tested
│   │   ├── Cargo.toml
│   │   ├── src/
│   │   │   ├── lib.rs              # re-exports public API
│   │   │   ├── error.rs            # typed errors (thiserror)
│   │   │   ├── security.rs         # XCODE_MCP_ROOT, path/scheme/config validation
│   │   │   ├── xcode.rs            # xcodebuild process supervision + timeout
│   │   │   ├── scheme.rs           # -list invocation + parse
│   │   │   ├── result_bundle.rs    # xcresulttool get build-results JSON parse
│   │   │   ├── diagnostic.rs       # stderr parse + merge with xcresult diagnostics
│   │   │   └── store.rs            # build_id ring buffer + on-disk result mgmt
│   │   └── tests/
│   │       ├── fixtures/
│   │       │   ├── list/           # recorded -list outputs
│   │       │   ├── result_bundle/  # recorded build-results.json
│   │       │   ├── stderr/         # recorded stderr snippets
│   │       │   └── MiniApp/        # committed fixture .xcodeproj (deliberate error)
│   │       ├── scheme_parse.rs
│   │       ├── diagnostic_parse.rs
│   │       ├── result_bundle_parse.rs
│   │       ├── security.rs
│   │       └── live/               # #[cfg(feature = "live-xcode")]
│   │           ├── build.rs
│   │           └── errors.rs
│   └── xcode-mcp/                  # bin: thin rmcp wiring + debug CLI
│       ├── Cargo.toml
│       └── src/
│           ├── main.rs             # env load → dispatch {serve|debug}
│           ├── server.rs           # rmcp tool handlers, stdio transport
│           └── cli.rs              # `xcode-mcp debug <subcmd>` for non-MCP testing
```

**Rationale:** the headline value of the project is the *parsing* (schemes, xcresult JSON, stderr). A library crate makes that logic first-class, fully unit-testable, and shared between the server, the debug CLI, and tests. The bin crate stays thin — just rmcp glue and arg dispatch.

## 6. Dependencies

Workspace `[workspace.dependencies]`, inherited by both crates:

| Crate | Version | Used in | Purpose |
|---|---|---|---|
| `rmcp` | `3.1.1` (pinned) | bin only | MCP server + stdio transport |
| `tokio` | `1`, features `["rt-multi-thread","macros","process","io-util","sync","fs","time"]` | both | async runtime (rmcp is tokio-based) |
| `serde` | `1`, `["derive"]` | both | tool args/results + xcresult JSON |
| `serde_json` | `1` | both | JSON (de)serialization |
| `thiserror` | `2` | core | typed errors |
| `tracing` + `tracing-subscriber` | latest | both | structured file logging (never stdout/stderr — MCP channel) |
| `uuid` | `1`, `["v4"]` | core | `build_id` generation |
| `regex` | `1` | core | stderr diagnostic line parsing |
| `clap` | `4`, `["derive"]` | bin only | debug CLI arg parsing |
| `libc` | latest | core | `setsid()` in `pre_exec` for process-group isolation |
| `tempfile` | latest | core dev | unit test temp dirs |

No snapshot test library — assertions use `assert_eq!` / `serde_json::json!` against fixture data.

## 7. `xcode_list_schemes` Tool

### Signature

```
xcode_list_schemes(project_or_workspace: string) -> {
  project_or_workspace: string,   // echoed, canonicalized
  schemes: string[],              // sorted, deduped
  configurations: string[],       // from the "Build Configurations:" block
  targets: string[],              // from the "Targets:" block
  parse_warnings: string[]        // non-fatal (e.g. truncated -list output, unknown sections)
}
```

No `destination` or `configuration` — listing is destination-agnostic. Configs/targets are returned as a bonus since `xcodebuild -list` prints them anyway and they're useful to the LLM (e.g. knowing "Release" exists before calling `xcode_build`).

### Invocation

- `xcrun xcodebuild -list -project <path>` XOR `-workspace <path>` (extension detected; only one passed).
- Same process-supervision code path as `xcode_build`, but with a fixed 30s timeout (hardcoded, not configurable). `-list` is fast; if it hangs the project is likely corrupted.
- No `.xcresult` is generated for `-list`.
- Stderr captured. Non-zero exit OR any output on stderr → `Error::XcodeListFailed { exit_code, stderr_excerpt }`. We do **not** parse partial scheme output from a failed `-list`.

### Parsing

Pure function `parse_list_output(stdout: &str) -> Result<ListInfo, Error>`, unit-tested against fixtures.

Canonical `xcodebuild -list` output (Xcode 26.x):

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

If no build configuration is specified ...
```

Section order has varied across Xcode versions (Schemes/Targets/Configs vs. Targets/Schemes/Configs). Parser rules:

- A line ending in `:` (`Schemes:`, `Targets:`, `Build Configurations:`) starts a section.
- Following indented lines (leading whitespace) belong to that section, trimmed.
- Blank line or new section header ends the current section.
- Trailing informational paragraph ("If no build configuration...") ignored.
- Empty sections allowed (e.g. a project with no schemes → `schemes: []` + a `parse_warning`).
- If **none** of the three expected headers appear → `Error::UnrecognizedListFormat` (signal to update the parser).
- Schema validation: schemes/targets/configs capped at 1024 entries each; entries capped at 128 chars, charset `^[A-Za-z0-9_ .-]{1,128}$`. An entry violating charset is dropped + added to `parse_warnings` (robust to weird-but-harmless names) rather than failing the whole call.

### Security interplay

Paths passed to `-list` go through `validate_project_or_workspace` (§11). Returned scheme names are *informational*; when the LLM later passes one to `xcode_build`, it gets re-validated there. Listing a scheme doesn't trust it.

## 8. `xcode_build` Tool, Process Supervision & Timeout

### Signature

```
xcode_build(
  project_or_workspace: string,                  // required, validated
  scheme: string,                                // required, validated
  action?: "build" | "clean" | "clean+build",    // default "build"
  configuration?: "Debug" | "Release",           // default not set → xcodebuild picks
  destination?: string,                          // validated, default not set
  timeout_secs?: u32                             // default 1800, max 7200
) -> {
  build_id: string,                              // UUID v4
  status: "Succeeded" | "Failed" | "TimedOut" | "Canceled",
  exit_code: i32 | null,                         // null if killed
  duration_secs: f64,
  xcresult_path: string,                         // under result_dir
  log_path: string,                              // under log_dir
  result_bundle_written: bool,                   // false on early-exit failures
  error_count: u32,                              // full diagnostics via xcode_get_build_errors
  warning_count: u32,
  truncated_stderr_excerpt: string | null        // last 2KB of stderr when status is Failed/TimedOut AND result_bundle_written==false; null otherwise
}
```

### Status semantics

- `Succeeded` — exit 0.
- `Failed` — exit non-zero. Result bundle may or may not exist.
- `TimedOut` — killed by timeout.
- `Canceled` — reserved for v2 (MCP cancellation). Never returned in v1.
- The server **never** raises an MCP error for a build that *ran*. A failed build is a successful tool call with `status: "Failed"`. MCP errors are reserved for invalid args, root violations, internal panics, or the build queue being shut down.

### Build lifecycle (`xcode.rs::run_build`, async)

> **Revision (2026-08-20):** Originally the server allocated a per-build
> `-derivedDataPath` under `result_dir/DerivedData/<build_id>` and `rm -rf`'d
> it after each build. Changed to **inherit Xcode's configured default
> DerivedData location** (typically `~/Library/Developer/Xcode/DerivedData/`,
> or whatever the user set in Xcode → Settings → Locations) so MCP builds
> reuse the IDE's build cache instead of compiling from scratch each time.
> Consequence: cleanup was removed (deleting shared IDE state would force a
> full rebuild in Xcode), and concurrent IDE-vs-MCP builds on the same
> project can collide on `build.db` — see §15 for the v2 concurrency impact.
> If stale state is suspected, callers should use `action: "clean+build"`.

1. **Reserve `build_id` + paths.** Generate UUID. Allocate:
   - `xcresult_path = result_dir/<build_id>.xcresult`
   - `log_path = log_dir/<build_id>.log`
   Pre-create the log file (truncate). The `.xcresult` dir is created by xcodebuild.
   **No `-derivedDataPath`** — xcodebuild writes to its configured default
   (inherits the user's Xcode setting), so MCP builds share the IDE cache.

2. **Acquire the global build permit.** `tokio::sync::Semaphore` with 1 permit, held for the whole build. Serialized execution. Reads (`xcode_list_schemes`, `xcode_get_build_errors`) don't touch this permit; only `xcode_build` does. Awaiting on the permit has no separate timeout — the caller's `timeout_secs` covers the whole call including wait time, so a stuck queue can't silently extend a build.

3. **Spawn xcodebuild in a new process group.**
   - `Command::new("xcrun")`, args: `["xcodebuild", "-scheme", <scheme>]`, `-project` XOR `-workspace` `<validated_path>`, `-configuration <cfg>` if set, `-destination <dest>` if set, `-resultBundlePath <xcresult_path>`, `-quiet`, then `build` / `clean` / `clean build` (two action args for `clean+build` — single invocation, matches Xcode semantics). **No `-derivedDataPath`** — inherits Xcode's configured default.
   - Process group via `Command::pre_exec` closure calling `libc::setsid()` before exec. Child becomes session leader → its pid == pgid. Wrapped in `unsafe` with a comment.
   - `stdout` and `stderr` piped. Two async tasks read each to completion, appending to a shared `Arc<Mutex<Vec<u8>>>` and writing line-buffered to the log file. The log file is the durable record; the in-memory buffer feeds the truncated excerpt.

4. **Wait with timeout.** `tokio::time::timeout(timeout_secs, child.wait())`. The two reader tasks (step 3) run independently; they finish naturally when the pipes hit EOF on child exit. We do **not** `await` the readers inside the timeout future — we await `child.wait()` only, then join the reader `JoinHandle`s afterwards (they will be done or finish within microseconds of the child exiting).

   - **Child exits within timeout (`Ok`):** read exit code. Determine `status`. Join both reader tasks (brief; they finish on EOF). Proceed to step 6.
   - **Timeout fires (`Err`):** kill the process group:
     - `kill -TERM -<pgid>` (negative pid = group). pgid == child pid (we used `setsid`).
     - `tokio::time::timeout(5s, child.wait())` to allow graceful shutdown.
     - If still alive after 5s: `kill -KILL -<pgid>`, then `child.wait()` (must succeed).
     - Join reader tasks (the kills close the pipes → EOF → readers finish). Mark `status = TimedOut`, `exit_code = null`.

5. **No derived-data cleanup.** The server uses Xcode's default DerivedData location (shared with the IDE), so it never `rm -rf`s it — deleting the user's IDE build cache would force a full rebuild in Xcode. If stale state is suspected, callers use `action: "clean+build"`, which lets xcodebuild manage its own DerivedData lifecycle. The `.xcresult` and `.log` stay (needed for `xcode_get_build_errors` and debugging).

6. **Register in the store.** Push `BuildRecord` into the ring buffer (§9). Best-effort: if the store is full, oldest evicted; its on-disk `.xcresult`/`.log` are **not** deleted (only the in-memory pointer is dropped — files remain queryable via direct path).

7. **Compute `error_count`/`warning_count`.** Best-effort: invoke the diagnostic-extraction path (§9) once, count by severity, store counts on the record. Do **not** embed full diagnostics in the `xcode_build` response (keeps it small; the LLM calls `xcode_get_build_errors` for detail). If extraction fails, counts are 0 and a `parse_warning` is recorded in the log — the build response itself still succeeds.

### Error handling in `run_build`

- Spawn failure (e.g. `xcrun` missing) → MCP error `XcodeSpawnFailed`.
- Arg validation failure → MCP error `InvalidArgument` (raised *before* acquiring the permit — no queue contamination).
- Log file open failure → MCP error `Internal` (server misconfigured; don't start the build).
- Panic in reader tasks → caught via `JoinHandle`, logged, build continues with whatever was captured.

### No progress, no cancellation in v1

The build is fire-and-await with a hard timeout. A v2 could add `notifications/progress` by parsing `xcodebuild`'s `% done` lines, but that's out of scope now.

## 9. `xcode_get_build_errors` Tool, Diagnostic Merging & Store

### Signature

```
xcode_get_build_errors(build_id?: string) -> {
  build_id: string,
  build_status: "Succeeded" | "Failed" | "TimedOut" | "Canceled" | "Unknown",
  source: "xcresult" | "stderr_only" | "none",
  errors:   Diagnostic[],
  warnings: Diagnostic[],
  notes:    Diagnostic[],
  parse_warnings: string[]
}

Diagnostic = {
  file: string | null,           // absolute, when available
  line: u32 | null,
  column: u32 | null,
  severity: "error" | "warning" | "note",
  message: string,               // single line; multi-line msgs joined with " ⏎ "
  category: string | null,       // e.g. "Swift Compiler Error", "Linker", "Build System"
  fix_its: FixIt[] | null,
  source: "xcresult" | "stderr"
}

FixIt = {
  message: string,
  range: { start_line, start_col, end_line, end_col } | null
}
```

`source` on the response tells the LLM whether it got the rich xcresult diagnostics or fell back to stderr-only. `none` means a `Succeeded` build or a timed-out build that wrote nothing — `errors`/`warnings`/`notes` are empty arrays, not omitted.

### Resolution flow

1. **Resolve `build_id`.**
   - If provided: look up in the in-memory ring buffer. If not in memory, check whether `<result_dir>/<build_id>.xcresult` exists on disk — if yes, synthesize a transient `BuildRecord` (status unknown → `"Unknown"`, paths from disk) and proceed (stateless durability). If neither, `Error::BuildNotFound`.
   - If omitted: use the most recent record in the ring buffer. If the buffer is empty AND exactly one `.xcresult` exists under `result_dir`, use it (convenient for fresh server starts); otherwise `Error::NoBuildAvailable { hint }`.

2. **Load diagnostics (hybrid).**

   **Path A — xcresult (primary).** Run `xcrun xcresulttool get build-results --format json --path <xcresult_path>`. Does not require a scheme or project — operates purely on the result bundle, so it works for any build whose `.xcresult` survived, including transient records synthesized from disk. Wrapped under the same process-supervision code as `xcodebuild` but with a fixed 60s timeout (parsing should be fast; if it hangs, the `.xcresult` is likely corrupted). Non-zero exit or empty JSON → fall through to Path B, record a `parse_warning`.

   **Path B — stderr fallback.** Read `<log_dir>/<build_id>.log` (captured xcodebuild stderr, tee'd during §8). Run the stderr parser.

3. **Merge** (`diagnostic.rs::merge_diagnostics`):
   - Run both parsers; collect into two `Vec<Diagnostic>` tagged with `source`.
   - Dedup key: `(file, line, column, severity, normalized_message)` where `normalized_message` = message trimmed + collapse internal whitespace. If an xcresult diagnostic and a stderr diagnostic share a key, **keep the xcresult one** (richer — has `category` and `fix_its`).
   - If `result_bundle_written == false` (early exit) OR xcresult parse failed → `source = "stderr_only"`.
   - If both produced nothing and `build_status` is `Succeeded` or `Unknown` → `source = "none"`.
   - Order: errors first (by file, then line, then column), then warnings (same order), then notes. Stable sort preserves xcresult-before-stderr on ties.

### xcresult JSON parsing (`result_bundle.rs`)

Pure fn `parse_build_results(json: &str) -> Result<ParsedDiagnostics, Error>`.

Modern `xcresulttool get build-results --format json` schema (schema version 0.1.0, verified against Xcode 26.x) returns an `actions` array; each `action` has `_results` → `issues` → a flat list of `Issue` entries:

```json
{
  "issueType": "BuildError" | "AnalyzerError" | "BuildWarning" | "AnalyzerWarning" | "Note",
  "message": "...",
  "category": "Swift Compiler Error" | "Linker" | "Build System" | ...,
  "documentLocationInCreatingWorkspace": { "url": "file:///path/file.swift#Line=42&Column=8" },
  "fixIts": [{ "message": "...", "range": {...} }, ...]
}
```

Parser rules:
- Walk `actions[*]._results.issues`. `issueType` → severity: `BuildError`/`AnalyzerError` → `error`; `BuildWarning`/`AnalyzerWarning` → `warning`; `Note` → `note`; unknown type → `note` + `parse_warning`.
- `file`/`line`/`column` extracted from `documentLocationInCreatingWorkspace.url` (a `file://` URL with `#Line=`/`Column=` fragment). Parsed with URL-decode + regex, not a full URL crate. Missing/malformed URL → all three `null`, diagnostic still emitted (some build-system errors have no location).
- `fixIts` optional; absent → `null`.
- Defensive parsing throughout: missing fields → `null`/empty, never a hard parse failure. The only hard failure is JSON that isn't an object at the top level or lacks `actions` entirely → `Error::UnrecognizedResultFormat` → triggers stderr fallback.

### stderr parsing (`diagnostic.rs::parse_stderr`)

Parses Apple's classic line format `<file>:<line>:<col>: <severity>: <message>` and the no-column variant `<file>:<line>: <severity>: <message>` (linker errors often omit column).

Regex:

```
^(?P<file>[^:\n]+):(?P<line>\d+):(?:(?P<col>\d+):)?\s*(?P<sev>error|warning|note|fatal error):\s*(?P<msg>.*)$
```

Rules:
- `file` is whatever xcodebuild printed — typically already absolute, but not canonicalized (we don't know the project root at parse time, and the path might be a derived-data temp). Passed through as-is.
- Multiline messages (continuation lines indented or starting with whitespace, no leading `file:line:`) are appended to the previous diagnostic's `message` with ` ⏎ ` separator.
- Lines that don't match and don't continue a previous diagnostic are skipped (xcodebuild prints noise: `Command PhaseScriptExecution failed...`, `** BUILD FAILED **`, etc.). Actual errors are always in the `file:line:col:` format.
- `category` inferred: `Linker` if `file` ends in `.o`/`.a`/`.dylib`/`.framework` or message mentions `linker`/`ld:`; `Build System` if no file/line; otherwise `Compiler`. Best-effort.
- `fix_its` always `null` from stderr — that metadata only exists in the xcresult.

### The store (`store.rs`)

```rust
pub struct BuildStore {
    records: VecDeque<BuildRecord>,   // ring buffer
    cap: usize,                        // default 32, env XCODE_MCP_STORE_CAP
    lock: Mutex<()>,                   // serializes push + "most recent" lookup
}

pub struct BuildRecord {
    build_id: String,
    status: BuildStatus,
    exit_code: Option<i32>,
    duration_secs: f64,
    project_or_workspace: PathBuf,
    scheme: String,
    xcresult_path: PathBuf,
    log_path: PathBuf,
    result_bundle_written: bool,
    error_count: u32,
    warning_count: u32,
    stderr_excerpt: Option<String>,
    created_at: SystemTime,
}
```

- `push(record)`: if `len == cap`, pop front. Popped record's on-disk files are **not** deleted (only the memory pointer is evicted; path-based lookup still works).
- `most_recent() -> Option<&BuildRecord>`: back of the deque.
- `get(build_id) -> Option<&BuildRecord>`: linear scan (32 entries max — fine).
- No on-disk index. The filesystem *is* the durable index: `<result_dir>/<build_id>.xcresult` and `<log_dir>/<build_id>.log` are the source of truth; the store is a convenience cache for "most recent" and in-memory metadata.
- **Cleanup:** best-effort `prune()` runs after each `push`: scans `result_dir` for `.xcresult` entries older than `XCODE_MCP_RESULT_TTL_HOURS` (default 24) and removes them + their matching `.log`. Failures logged, never propagated. Bounds disk usage without the server tracking every file forever.

## 10. Server Entrypoint & Transport

- `main.rs`: read env (`XCODE_MCP_ROOT`, `XCODE_MCP_RESULT_DIR`, `XCODE_MCP_LOG_DIR`, caps, TTLs) → construct `BuildStore` + `ServerState` (Arc-wrapped) → dispatch on first arg: `debug` → CLI, `serve` (or no arg, the default) → MCP server.
- `server.rs`: implement rmcp 3.1.1 `ServiceExt`. Three tools registered: `xcode_list_schemes`, `xcode_build`, `xcode_get_build_errors`. Each handler is a thin wrapper: deserialize args via serde, call into `xcode-mcp-core`, serialize the result. All heavy lifting is in core; the handler is ≤30 lines per tool.
- Transport: stdio only. rmcp's stdio serve over stdin/stdout.
- **Logging must not touch stdout/stderr** — those are the MCP channel. `tracing-subscriber` writes to `$LOG_DIR/server.log` (rolling). On startup, log the resolved config (root, result dir, log dir, store cap, TTL) at INFO.
- On panic in a tool handler: catch via `std::panic::catch_unwind` (handler boundary), return an MCP error `Internal`, log the backtrace. A panic must never tear down the stdio channel.

## 11. Security Model & Validation

`XCODE_MCP_ROOT` is the single trust boundary.

### Startup (server mode only; debug CLI skips env checks but still does per-call validation)

- `XCODE_MCP_ROOT` must be set to an absolute path and exist (canonicalized via `std::fs::canonicalize`). Server refuses to start otherwise — fail-fast with a clear error.
- `XCODE_MCP_RESULT_DIR` (optional, default `$ROOT/.xcode-mcp-results`) and `XCODE_MCP_LOG_DIR` (optional, default `$ROOT/.xcode-mcp-logs`) — both must, if set, resolve **under** the canonical root. Server refuses to start if not.
- Server creates `result_dir` and `log_dir` at startup if missing.

### Per-call validation (in `security.rs`, pure functions returning `Result<PathBuf, Error>` / `Result<String, Error>`)

| Input | Rule |
|---|---|
| `project_or_workspace` path | Must end in `.xcodeproj` or `.xcworkspace`. `std::fs::canonicalize` it; the canonical path must equal the root or have the canonical root as a prefix (component-wise `Path::components` comparison — not string prefix, to avoid `/root/evil` matching `/root`). Reject symlinks escaping root (handled by `canonicalize`). Reject if path doesn't exist. |
| `scheme` | Regex `^[A-Za-z0-9_ .-]{1,128}$`. Rejects quotes, `--`, `;`, `\|`, `&`, `>`, `<`, `$`, backticks, newlines. Hard-cap length 128. |
| `configuration` | Must be exactly `Debug` or `Release`. |
| `action` | Enum — serde-validated, only `build` / `clean` / `clean+build`. |
| `destination` | Regex `^[A-Za-z0-9_ ./=,-]{1,256}$`. Same metachar blacklist as scheme. Allows `generic/platform=iOS`, `platform=macOS`, `id=ABCD-123`, etc. Rejects shell metachars. |
| `timeout_secs` | `Option<u32>`, range `1..=7200`. Default 1800 (30 min). |
| `build_id` (for `xcode_get_build_errors`) | Regex `^[0-9a-fA-F-]{1,64}$` (UUID format, no slashes → no path traversal). |

### Process invocation (in `xcode.rs`)

- Always `tokio::process::Command::new("xcrun")` with explicit `.arg()` calls — never a shell string. `xcrun xcodebuild ...` so the active Xcode is used.
- All flag values (`scheme`, `configuration`, `destination`, paths) come **only** from validated inputs — never echoed from raw user JSON.
- Fixed mandatory flags set by the server: `-resultBundlePath <under result_dir>`, `-quiet` (suppress noise; diagnostics come from the xcresult, not stdout). **No `-derivedDataPath`** — inherits Xcode's configured default so MCP builds reuse the IDE build cache.
- Child run in its own process group via `setsid()` in `pre_exec` (§8). On timeout: SIGTERM the group, 5s grace, SIGKILL. Prevents orphaned `swiftc`/`ld`/`clang` children.

### Explicitly out of scope (defended against by design)

- No `extra_args` / passthrough — can't inject `-derivedDataPath /etc/...` or `&& rm -rf`.
- No `-workspace`/`-project` guessing — caller must supply one; both rejected if supplied (ambiguous).
- No `xcodebuild` actions beyond build/clean (no `test`, `archive`, `install` — different failure modes & side effects).

**Why this is enough:** every value the server passes to `xcodebuild` is either a server-chosen constant, a path verified to be under root, or a string matching a strict charset. There's no shell, no `sh -c`, no value composition. The only remaining risk — a malicious `.xcodeproj` running a build phase — is inherent to `xcodebuild` itself and out of our threat model (the root already trusted the project).

## 12. Testing

### Strategy

- **Unit tests** for scheme-list output parsing, stderr-regex diagnostic parsing, and xcresult JSON parsing, fed by **recorded fixtures** committed under `tests/fixtures/`.
- **Unit tests** for path/scheme/configuration validation (security).
- **A small committed fixture .xcodeproj** under `tests/fixtures/MiniApp/` (a trivial Swift app with one deliberate compile error) used by **optional** integration tests gated behind `--features xcode-mcp-core/live-xcode` AND `XCODE_MCP_LIVE_TESTS=1` env var (double gate — prevents a stray `--all-features` from firing slow Xcode builds).
- Fast unit tests run by default; slow live tests opt-in.

### Fixture coverage

`tests/fixtures/list/`:
- `typical.txt` — happy path, all three sections, standard order.
- `no_schemes.txt` — header present, empty list.
- `reordered.txt` — Targets before Schemes (older Xcode).
- `extra_sections.txt` — unknown section preserved in `parse_warnings`.
- `malformed.txt` — no recognizable headers → `UnrecognizedListFormat`.
- `workspace.txt` — `-workspace` variant output.

`tests/fixtures/result_bundle/`:
- `build_results_typical.json` — 2 errors, 3 warnings, 1 note, mix of with/without location, one with fixIts.
- `build_results_no_issues.json` — successful build, empty `issues`.
- `build_results_malformed_url.json` — bad `documentLocationInCreatingWorkspace.url` → location `null`, diagnostic still emitted.
- `build_results_unknown_issue_type.json` — unknown `issueType` → `note` + parse_warning.
- `build_results_missing_actions.json` → `UnrecognizedResultFormat`.

`tests/fixtures/stderr/`:
- `compiler_error.txt` — single-file swift error with column.
- `linker_error.txt` — `ld:` style, no column, multiple lines.
- `mixed.txt` — interleaved errors/warnings/notes + noise lines.
- `multiline.txt` — continuation lines folded correctly.
- `no_diagnostics.txt` — pure noise (`** BUILD FAILED **` only) → empty result.

### Live integration tests (gated)

- Build `MiniApp` (good) → assert `Succeeded` + `result_bundle_written == true`.
- Build `MiniAppBroken` → assert `Failed` + `error_count > 0`.
- Build with `timeout_secs = 1` against a deliberately slow project → assert `TimedOut` + `exit_code == null`.
- Build `MiniAppBroken`, call `xcode_get_build_errors` → assert ≥1 error with expected `file`/`line`/`message`; assert `source == "xcresult"`; assert a known warning from the fixture also appears.

### `xcode-mcp debug` CLI

A thin non-MCP front-end onto `xcode-mcp-core`, used by humans, the live integration tests, and the MCP Inspector workflow.

```
xcode-mcp debug list-schemes <project_or_workspace>
xcode-mcp debug build \
  --project <path> --scheme <name> \
  [--action build|clean|clean+build] \
  [--configuration Debug|Release] \
  [--destination <dest>] \
  [--timeout-secs <n>]
xcode-mcp debug build-errors [<build_id>]
xcode-mcp debug serve                           # alias for running the MCP server
xcode-mcp debug inspector-help                  # prints MCP Inspector instructions
```

- Skips `XCODE_MCP_ROOT` env enforcement (so you can point at any project on disk for ad-hoc testing), but still runs **all** per-call validation (scheme charset, configuration enum, etc.) — the validation logic is the security boundary, not the env check.
- Output is JSON to stdout (one document per subcommand) — same shapes the MCP tools return, so the CLI doubles as a contract test fixture: the live integration tests assert on the parsed JSON.
- Errors → human-readable message to stderr + non-zero exit. Never raw panics.

## 13. MCP Inspector Instructions

Also printable via `xcode-mcp debug inspector-help`, and in README §7.

```bash
# 1. Build the server
cargo build --release

# 2. Launch the MCP Inspector against it
npx @modelcontextprotocol/inspector \
  ./target/release/xcode-mcp serve

# 3. In the Inspector UI:
#    - Transport: STDIO
#    - Command:   ./target/release/xcode-mcp
#    - Args:      serve
#    - Env:       XCODE_MCP_ROOT=/path/to/your/projects
#    - Click "Connect"

# 4. Set the env var in the Inspector's "Environment" panel BEFORE connecting
#    (the server fail-fasts at startup if XCODE_MCP_ROOT is unset).

# 5. Verify each tool:
#    - xcode_list_schemes: pass a .xcodeproj path → see schemes/configs/targets
#    - xcode_build:        pass path + scheme → see build_id + status
#    - xcode_get_build_errors: pass the build_id from step above → see diagnostics

# 6. To test the debug CLI alongside Inspector:
#    ./target/release/xcode-mcp debug list-schemes /path/to/App.xcodeproj
```

## 14. Verification Checklist

How we know each of the 15 requirements is met:

| # | Item | Verified by |
|---|---|---|
| 1 | rmcp 3.1.1 MCP server | pinned in `Cargo.toml`; server boots under Inspector |
| 2 | stdio transport | `rmcp::serve` over stdin/stdout; no TCP/HTTP code |
| 3 | `xcode_list_schemes` | tool registered; live test asserts `MiniApp` scheme |
| 4 | `xcode_build` | tool registered; live test asserts `Succeeded`/`Failed` |
| 5 | `xcode_get_build_errors` | tool registered; live test asserts diagnostics |
| 6 | `.xcodeproj` + `.xcworkspace` | validation accepts both; fixture tests for both extensions |
| 7 | stdout/stderr capture | tee'd to log file; `truncated_stderr_excerpt` in response |
| 8 | `.xcresult` generation | `-resultBundlePath` always set; `result_bundle_written` field |
| 9 | modern `xcresulttool get build-results` | `result_bundle.rs` parses schema 0.1.0; fixture tests |
| 10 | compiler/linker/build diagnostics | `category` field from xcresult + inferred from stderr |
| 11 | timeout + termination | `run_build` kills process group on timeout; live test asserts `TimedOut` |
| 12 | `XCODE_MCP_ROOT` security | `security.rs` validates; unit tests for path traversal / symlink escape |
| 13 | no shell invocation | all `Command::new("xcrun").arg(...)`; no `sh -c`; grep-asserted in tests |
| 14 | unit tests for diagnostic + scheme parsing | `tests/diagnostic_parse.rs`, `tests/scheme_parse.rs` with fixtures |
| 15 | MCP Inspector instructions | README §7 + `debug inspector-help` subcommand |

## 15. Open Questions / Future Work

- **v2:** MCP progress notifications (parse `xcodebuild` `% done` lines) and cancellation (`notifications/cancelled`).
- **v2:** concurrent builds. The current design inherits Xcode's default DerivedData location (shared with the IDE), so concurrent builds on the same project would collide on `build.db`. v2 concurrency will require reintroducing a **server-managed** per-build `-derivedDataPath` (UUID-based, not user-supplied) gated behind a concurrency flag. Design deferred to v2.
- **v2:** `xcodebuild test` / `archive` support (different failure modes, different result bundle schema).
- **v2:** `xcode_list_destinations` tool wrapping `xcodebuild -showdestinations`.
- **Possibly v1.1:** snapshot tests via `insta` if fixture assertion verbosity becomes a maintenance burden.
