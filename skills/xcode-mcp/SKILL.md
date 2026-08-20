---
name: xcode-mcp
description: "Drive the xcode-mcp MCP server to build Xcode iOS/macOS projects and read structured build errors. Use when the user asks to build, compile, run xcodebuild, check or fix build errors, list Xcode schemes, run pod install/update, refresh CocoaPods, build .xcodeproj or .xcworkspace, debug iOS/macOS build failures, get build diagnostics or fix-it suggestions, or iterate on a failing Xcode build. Wraps four MCP tools: xcode_list_schemes, xcode_build, xcode_get_build_errors, xcode_pod. Triggers: 'build the app', 'why did the Xcode build fail', 'compile the iOS project', 'run xcodebuild', 'pod install', 'get build errors', 'list schemes', 'fix compile errors', 'Xcode build failed', 'rebuild'."
---

# xcode-mcp

Guide for using the **xcode-mcp** MCP server (a local `xcodebuild` wrapper that returns structured diagnostics). The server exposes exactly four tools; this skill is the correct way to sequence them.

IRON LAW: NEVER CALL `xcode_build` WITHOUT FIRST CALLING `xcode_list_schemes`. Scheme names must be **discovered**, never guessed. A wrong scheme name burns a multi-minute build and returns no useful diagnostics.

COROLLARY: NEVER declare a build "done" or "fixed" without calling `xcode_get_build_errors`. The `status` string alone ("Succeeded" / "Failed") is not the diagnostics — the structured `diagnostics[]` array is.

## The Four Tools (mental model)

| Tool | One-liner | When |
|---|---|---|
| `xcode_list_schemes` | Discover what can be built | Before ANY build (Iron Law) |
| `xcode_pod` | Refresh `Pods.xcworkspace` | Only when Podfile/Podfile.lock changed |
| `xcode_build` | Run xcodebuild, get `build_id` | The build step |
| `xcode_get_build_errors` | Read structured diagnostics | After EVERY build (Corollary) |

Full schemas and return shapes: load `references/tools-reference.md`.

## Workflow

Copy this checklist and check off items as you complete them:

```
xcode-mcp Progress:

- [ ] Step 1: Locate project ⚠️ REQUIRED
  - [ ] 1.1 Identify .xcodeproj or .xcworkspace path
  - [ ] 1.2 Verify it's under the server's XCODE_MCP_ROOT
- [ ] Step 2: Discover schemes ⚠️ REQUIRED (IRON LAW)
  - [ ] 2.1 Call xcode_list_schemes
  - [ ] 2.2 Read schemes[], configurations[], targets[]
- [ ] Step 3: Confirm build intent with user ⛔ GATE
  - [ ] scheme, action, configuration, destination
  - [ ] Whether CocoaPods need refreshing
- [ ] Step 4 (conditional): Refresh CocoaPods
  - [ ] Podfile/Podfile.lock changed? → xcode_pod action=install
  - [ ] ⛔ Confirm with user before action=update (bumps pod versions)
- [ ] Step 5: Build
  - [ ] Call xcode_build with the confirmed params
  - [ ] Capture build_id + status from the result
- [ ] Step 6: Read diagnostics ⚠️ REQUIRED (COROLLARY)
  - [ ] Call xcode_get_build_errors with the build_id
  - [ ] Group diagnostics[] by severity (error / warning / note)
- [ ] Step 7: Report & iterate
  - [ ] If errors → propose targeted fixes, then loop to Step 5
  - [ ] If clean → report success and STOP
```

## Step 1: Locate project

Ask: Is the path a `.xcodeproj` or `.xcworkspace`? The server rejects anything else.

Ask: Does the path canonicalize **under** the server's `XCODE_MCP_ROOT`? This is the single trust boundary. If you don't know the root, ask the user — paths outside root fail with `PathRejected`. If the project uses CocoaPods, the buildable path is usually the `Pods.xcworkspace` (sibling of the `.xcodeproj`), not the `.xcodeproj` itself.

## Step 2: Discover schemes ⚠️ REQUIRED

Call `xcode_list_schemes` with `project_or_workspace`. Read the result:

```json
{ "schemes": ["App", "AppTests"], "configurations": ["Debug", "Release"], "targets": ["App"], "parse_warnings": [] }
```

Pick the scheme from `schemes[]` — do not invent one. If the scheme the user named isn't in the list, **stop and report** the available schemes instead of guessing. If `schemes` is empty, the project may need `pod install` first (CocoaPods projects often hide schemes until `Pods.xcworkspace` is built) — go to Step 4.

## Step 3: Confirm build intent ⛔ GATE

Before spending minutes on a build, confirm with the user:
- **scheme** — from Step 2
- **action** — `build` (default) / `clean` / `clean+build` (use `clean+build` only when stale DerivedData is suspected; it's slower)
- **configuration** — `Debug` / `Release` (omit to let xcodebuild pick from the scheme)
- **destination** — see `references/destinations-and-configs.md`. For iOS apps prefer `generic/platform=iOS`; for macOS prefer `platform=macOS`. Omitting destination often yields "builds for the current Mac" silently.

⚠️ Do NOT proceed to build without user confirmation of these four values.

## Step 4 (conditional): Refresh CocoaPods

Run pod when any of these are true:
- Podfile or Podfile.lock changed (added/removed pod entries, merge, etc.)
- `schemes` was empty in Step 2 (CocoaPods project with missing/stale `Pods.xcworkspace`)
- A **local development pod** (declared `pod 'Foo', :path => 'local/Foo'`) added or removed source files, or its `.podspec` changed

### `install` vs `update` — pick by trigger

| Trigger | Action | Why |
|---|---|---|
| Podfile / Podfile.lock changed | `install` | Respects `Podfile.lock`, reproducible. **Default.** |
| Schemes empty (Pods.xcworkspace missing) | `install` | Just (re)generate the workspace |
| Local dev pod's **podspec** changed (new deps, version bump) | `install` | `install` re-reads the podspec; enough |
| Local dev pod **added/removed source files** (its `source_files` glob now matches different files) | `update` | `install` does NOT re-scan the dev pod's file manifest — only `update` refreshes it. ⛔ Confirm first (see below) |
| User wants newer remote pod versions | `update` | Bumps versions per `Podfile` constraints. ⛔ Confirm first |

### ⛔ The `update` confirmation gate

`update` is the riskier action. Before running it, confirm with the user because:

**Scoping limitation:** the MCP tool runs an **unscoped** `pod update` (no pod name). A full `pod update` re-resolves EVERY pod, so bumping a local dev pod also bumps remote pods to their latest allowed versions — which can break the build in unrelated ways.

For the local-dev-pod case, offer the user two paths:
1. **Scoped update (safer, manual):** the user runs `pod update <PodName>` in the project's parent dir in their own terminal — refreshes only that dev pod's file list. Then continue to Step 5 without calling `xcode_pod`.
2. **Full update via MCP:** call `xcode_pod` with `action: "update"` — accepts that all pods re-resolve. Only do this with explicit user OK.

Ask: "A local pod changed its source files, which needs `pod update` to re-scan. The MCP tool can only run an unscoped `pod update` (all pods re-resolve). Want to (a) run `pod update KKExtensionBase` yourself for a scoped update, or (b) have me run a full `pod update` here?"

### Failure handling
The tool runs in the project's **parent** directory (where `Podfile` lives). On failure (`status: "Failed"`), read `stderr_excerpt`, do NOT proceed to build — fix the Podfile/podspec issue first. When `xcode_build` is called with `pod_action` set, it runs pod then builds in one call; prefer the separate `xcode_pod` tool when you want to inspect pod output before building.

## Step 5: Build

Call `xcode_build` with the confirmed params. Capture from the result:
- `build_id` — **required** for Step 6
- `status` — `Succeeded` / `Failed` / `TimedOut` / `PodFailed`
- `error_count`, `warning_count` — quick signal, NOT the full diagnostics
- `truncated_stderr_excerpt` — early-exit hint when no result bundle was produced

If `status` is `TimedOut`: raise `timeout_secs` (max 7200) or ask the user — don't just retry. If `PodFailed`: pod step failed, see Step 4.

## Step 6: Read diagnostics ⚠️ REQUIRED

Call `xcode_get_build_errors` with the `build_id` from Step 5 (omit `build_id` to get the most recent build). Read `diagnostics[]`:

```json
{ "diagnostics": [{
  "file": "Sources/App.swift", "line": 42, "column": 3,
  "severity": "error", "message": "cannot find 'foo' in scope",
  "category": "Swift Compiler Error",
  "fix_its": [{ "message": "Replace with 'foo()'", "range": {...} }],
  "source": "xcresult"
}], "parse_warnings": [] }
```

Load `references/diagnostics-guide.md` for how to map severities, `source` (`xcresult` vs `stderr`), fix-its, and `parse_warnings` to a fix plan.

## Step 7: Report & iterate

Report errors as `file:line:col — message` (the format a developer can jump to). Quote fix-its verbatim when present. Then:
- **Errors present** → propose the smallest code change that fixes the root cause, apply it, loop back to **Step 5** (a rebuild is the only proof).
- **Only warnings** → report them; ask the user whether to fix or ignore. Do not silently "clean up" warnings.
- **Clean** → say so and STOP. Do not trigger extra builds.

## Anti-Patterns

- **Guessing scheme names** — e.g. calling `xcode_build` with scheme `"App"` without listing first. (Violates Iron Law.)
- **Trusting `status: "Succeeded"` as "no errors"** — always read `xcode_get_build_errors`. (Violates Corollary.)
- **Passing a path outside `XCODE_MCP_ROOT`** — fails with `PathRejected`; the root is a hard boundary.
- **Building the `.xcodeproj` for a CocoaPods project** — use `Pods.xcworkspace`, or schemes will be missing/wrong.
- **Defaulting to `pod update` for routine changes** — Podfile/Podfile.lock changes need `install`, not `update`. Reserve `update` for: (a) local dev pod source-file add/remove, (b) podspec changes that `install` won't pick up, or (c) an explicit user request to bump remote versions. Always confirm before `update`.
- **Omitting `destination` for iOS apps** — silently builds for macOS host. Use `generic/platform=iOS`.
- **Retrying a `TimedOut` build with the same timeout** — raise `timeout_secs` or ask; retrying wastes minutes.
- **Trying to pass extra xcodebuild flags** — the flag surface is fixed; there is no `extra_args`. Work within `action`/`configuration`/`destination`.
- **Re-running `xcode_build` repeatedly without reading errors** — each call is a full build. Always read diagnostics between iterations.
- **Looping more than ~5 fix iterations without progress** — stop, summarize the stuck error, and ask the user.

## Caveats

- **IDE-vs-MCP build collision.** MCP builds inherit Xcode's configured DerivedData location (they do **not** pass `-derivedDataPath`), so they share the same build cache as the Xcode IDE. If a build fails immediately with a `database is locked` / `unable to attach to build system` style error and `xcode_get_build_errors` returns no diagnostics, the Xcode IDE may be building the same project — wait for the IDE build to finish and retry. The server's build permit serializes MCP-vs-MCP builds; it cannot serialize IDE-vs-MCP.

## Pre-Delivery Checklist

Before telling the user the build is fixed/done:
- [ ] `xcode_get_build_errors` was called on the final build's `build_id`
- [ ] `diagnostics[]` contains zero `severity: "error"` entries
- [ ] Every error from the previous iteration is gone (not just new ones appeared)
- [ ] Fix-its, when applied, were actually applied to source — not just described
- [ ] Warnings reported to the user (not silently ignored)
- [ ] No more than ~5 rebuild iterations; if exceeded, escalated to the user
- [ ] Final summary states the scheme + destination + configuration that built clean
