# Tools Reference

Exact schemas and return shapes for the four `xcode-mcp` tools. Load this when you need parameter names, types, required flags, or the JSON shape of a result.

## Table of Contents
1. `xcode_list_schemes`
2. `xcode_build`
3. `xcode_get_build_errors`
4. `xcode_pod`
5. Common error responses

---

## 1. `xcode_list_schemes`

List schemes, targets, and configurations for a project or workspace.

### Parameters
| Parameter | Type | Required | Notes |
|---|---|---|---|
| `project_or_workspace` | string | yes | Must be `.xcodeproj`/`.xcworkspace`, must canonicalize under `XCODE_MCP_ROOT` |

### Returns — `ListInfo`
```json
{
  "schemes": ["App", "AppTests", "AppUITests"],
  "configurations": ["Debug", "Release"],
  "targets": ["App", "AppTests"],
  "parse_warnings": []
}
```
- `schemes` — pick build `scheme` from THIS list. Never guess.
- `configurations` — valid values for `xcode_build.configuration`.
- `targets` — informational; not a build parameter.
- `parse_warnings` — non-fatal parser notes; usually empty. If `schemes` is empty AND this is a CocoaPods project, run `xcode_pod` (Pods.xcworkspace may be missing/stale).

---

## 2. `xcode_build`

Run an `xcodebuild` and return a `build_id` + status. Serializes globally (one build at a time across all callers).

### Parameters
| Parameter | Type | Required | Notes |
|---|---|---|---|
| `project_or_workspace` | string | yes | Under `XCODE_MCP_ROOT` |
| `scheme` | string | yes | From `xcode_list_schemes`. Charset: `[A-Za-z0-9_ .-]`, max 128 |
| `action` | enum | no | `"build"` (default) \| `"clean"` \| `"clean+build"` |
| `configuration` | enum | no | `"Debug"` \| `"Release"`. Omit → scheme's default |
| `destination` | string | no | e.g. `generic/platform=iOS`, `platform=macOS`. Charset: `[A-Za-z0-9_ ./=\-,]`, max 256 |
| `timeout_secs` | integer | no | Default 1800, range 1..=7200 |
| `pod_action` | enum | no | `"install"` \| `"update"`. When set, runs pod in the project's parent dir BEFORE building |
| `pod_timeout_secs` | integer | no | Default 600, range 1..=3600 (separate from `timeout_secs` — pod is network-bound) |

### Returns — `BuildOutput`
```json
{
  "build_id": "550e8400-e29b-41d4-a716-446655440000",
  "status": "Failed",
  "exit_code": 1,
  "duration_secs": 42.7,
  "xcresult_path": "/.../.xcode-mcp-results/<build_id>.xcresult",
  "log_path": "/.../.xcode-mcp-logs/<build_id>.log",
  "result_bundle_written": true,
  "error_count": 3,
  "warning_count": 5,
  "truncated_stderr_excerpt": "...",
  "pod": { "action": "install", "status": "Succeeded", "exit_code": 0, "duration_secs": 12.1, "log_path": "...", "stderr_excerpt": null }
}
```

### `status` values
| status | Meaning | Next step |
|---|---|---|
| `Succeeded` | Build exit 0 | Still call `xcode_get_build_errors` for warnings (Corollary) |
| `Failed` | Build exit non-zero | Call `xcode_get_build_errors` with `build_id` |
| `TimedOut` | Killed at `timeout_secs` | Raise timeout (≤7200) or ask user. Don't retry blindly |
| `PodFailed` | `pod_action` step failed; xcodebuild did NOT run | Read `pod.stderr_excerpt`; fix Podfile; do not call `get_build_errors` (no build ran) |

`error_count` / `warning_count` are a quick signal only. The authoritative list is from `xcode_get_build_errors`.

`truncated_stderr_excerpt` is populated when the build exited too early to produce a result bundle — useful when `result_bundle_written: false`.

---

## 3. `xcode_get_build_errors`

Return structured diagnostics for a build. Call after EVERY build.

### Parameters
| Parameter | Type | Required | Notes |
|---|---|---|---|
| `build_id` | string | no | From `BuildOutput.build_id`. Omit → most recent build in the server's ring buffer |

### Returns
```json
{
  "build_id": "550e8400-...",
  "diagnostics": [ { /* Diagnostic, see diagnostics-guide.md */ } ],
  "parse_warnings": [],
  "source": "xcresult"
}
```
- If `build_id` is omitted and the ring buffer is empty → error.
- See `references/diagnostics-guide.md` for reading `diagnostics[]`.

---

## 4. `xcode_pod`

Run `pod install` / `pod update` in the project's **parent** directory (where `Podfile` must exist).

### Parameters
| Parameter | Type | Required | Notes |
|---|---|---|---|
| `project_or_workspace` | string | yes | Under `XCODE_MCP_ROOT`; parent dir must contain a `Podfile` |
| `action` | enum | yes | `"install"` (respects Podfile.lock; default for Podfile/Podfile.lock changes) \| `"update"` (re-resolves ALL pods — confirm with user; required for local dev pod source-file add/remove, see below) |
| `timeout_secs` | integer | no | Default 600, range 1..=3600 |

### When to use `install` vs `update`
- `install` — Podfile changed, Podfile.lock changed after a merge, `Pods.xcworkspace` missing, or a local dev pod's **podspec** changed. Re-reads the podspec; reproducible.
- `update` — a local development pod (`pod 'Foo', :path => 'local/Foo'`) **added or removed source files** (its `source_files` glob now matches different files), OR the user explicitly wants newer remote pod versions. `install` does NOT re-scan a dev pod's file manifest.

### ⛠️ Scoping limitation
`xcode_pod` runs an **unscoped** `pod update` — it has no pod-name parameter. A full `pod update` re-resolves every pod, so using it to refresh one local dev pod also bumps all remote pods (can break the build). For a scoped refresh, tell the user to run `pod update <PodName>` in the project's parent dir themselves, then continue. Only call `xcode_pod action=update` with explicit user acceptance of a full re-resolve.

### Returns — `PodOutput` (PascalCase)
```json
{
  "RunId": "...",
  "Action": "install",
  "WorkingDir": "/path/to/project/..",
  "Status": "Succeeded",
  "ExitCode": 0,
  "DurationSecs": 12.1,
  "LogPath": "/.../.xcode-mcp-logs/<run_id>.pod.log",
  "StderrExcerpt": null
}
```
> Note: `PodOutput` is serialized as PascalCase (unlike other tools which are snake_case). Field names are `Status`, `ExitCode`, etc.

`Status` values: `Succeeded` / `Failed` / `TimedOut`. On `Failed`, read `StderrExcerpt` and fix the Podfile before building.

---

## 5. Common error responses

The server distinguishes two failure kinds:

- **JSON-RPC error (`-32602`)** — bad params (missing required arg, unknown tool, invalid enum, path outside root, charset violation). Returned as a protocol error; the tool did not execute. Fix the arguments and retry.
- **Tool error (`isError: true`)** — the tool ran but failed (build failed, pod failed, path not found, config file missing). The response body is `Error: <message>`. For builds/pods, prefer reading the structured `BuildOutput`/`PodOutput` (which carries `status` + excerpts) over the error string.

### Setup errors (fail-fast at server startup)
- `Xcode projects root not configured` → set `XCODE_MCP_ROOT` env var OR create `~/.config/xcode-mcp/config` with `root = /path/to/projects`. The server exits at startup if neither is present.
- `root path does not exist` → the configured root doesn't exist on disk.

### Path errors
- `PathRejected: path ... is outside root ...` → the project path isn't under `XCODE_MCP_ROOT`. Move it, or reconfigure the root.
- `PathRejected: must be .xcodeproj or .xcworkspace` → wrong extension.
- `PathNotFound` → path doesn't exist on disk.

### Validation errors (`InvalidArgument`)
- `invalid scheme` / `invalid destination` / `invalid build_id` → charset/length check failed. These are almost always a typo or a guessed value — go back to `xcode_list_schemes`.
- `configuration must be Debug or Release` / `action must be build/clean/clean+build` / `pod action must be install or update` → wrong enum value.
- `timeout_secs must be 1..=7200` / `pod_timeout_secs must be 1..=3600` → out of range.
