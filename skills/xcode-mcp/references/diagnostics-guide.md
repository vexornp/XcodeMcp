# Diagnostics Guide

How to read the `diagnostics[]` array returned by `xcode_get_build_errors`. Load this when interpreting a build's diagnostics or building a fix plan.

## Table of Contents
1. The `Diagnostic` object
2. Severity
3. `source`: `xcresult` vs `stderr`
4. Fix-its
5. `category` cheat sheet
6. `parse_warnings`
7. Building a fix plan

---

## 1. The `Diagnostic` object

```json
{
  "file": "Sources/App.swift",
  "line": 42,
  "column": 3,
  "severity": "error",
  "message": "cannot find 'fetchUser' in scope",
  "category": "Swift Compiler Error",
  "fix_its": [
    {
      "message": "Replace 'fetchUser' with 'fetchUser()'",
      "range": { "start_line": 42, "start_col": 3, "end_line": 42, "end_col": 12 }
    }
  ],
  "source": "xcresult"
}
```

| Field | Type | Notes |
|---|---|---|
| `file` | string \| null | Source file path. May be relative to project or absolute. `null` for project-level errors (e.g. signing). |
| `line` | u32 \| null | 1-indexed. `null` if no specific line. |
| `column` | u32 \| null | 1-indexed. |
| `severity` | enum | `error` / `warning` / `note` (see §2) |
| `message` | string | The diagnostic text. Quote verbatim to the user. |
| `category` | string \| null | Compiler/linker category (see §5). Useful for grouping. |
| `fix_its` | array \| null | Suggested replacements (see §4). `null` or empty = none. |
| `source` | enum | `xcresult` (structured) or `stderr` (regex-parsed fallback) |

Report to the user as `file:line:col — message`. This is the jump-to-location format.

## 2. Severity

| severity | Action |
|---|---|
| `error` | Blocks a successful build. MUST be resolved. Triage these first. |
| `warning` | Build can still succeed. Report to user; ask before fixing. Do not silently change code to clear warnings. |
| `note` | Attached context for an error/warning (e.g. "previous declaration here"). Not actionable alone — follow it to its parent error. |

Order of operations: fix all `error`s, then surface `warning`s to the user, then ignore `note`s (they're informational).

## 3. `source`: `xcresult` vs `stderr`

Diagnostics come from one of two sources. The `source` field tells you which.

| source | When produced | Fidelity |
|---|---|---|
| `xcresult` | `xcresulttool get build-results` on the result bundle. The primary path. | High: includes `fix_its`, precise columns, `category`. |
| `stderr` | Regex parse of xcodebuild stderr. Used as fallback when the build exits too early to produce a result bundle (e.g. config error, missing file). | Lower: no fix-its, column often missing, `category` inferred. |

If ALL diagnostics are `source: "stderr"`, the build likely failed before compilation (e.g. "scheme not found", "no such file", signing/config error). These are usually a **configuration** problem, not a code problem — re-check scheme/destination/configuration, not the source files.

## 4. Fix-its

```json
"fix_its": [{
  "message": "Replace 'fetchUser' with 'fetchUser()'",
  "range": { "start_line": 42, "start_col": 3, "end_line": 42, "end_col": 12 }
}]
```

- `message` describes the suggested change. **Quote it verbatim** — it's already phrased as an action.
- `range` is the region to replace (1-indexed lines/cols).
- Fix-its are **suggestions**, not commands. Before applying: read the surrounding code and confirm the fix addresses the root cause, not just the symptom. If two fix-its conflict, pick the one matching user intent.
- Only `xcresult`-sourced diagnostics carry fix-its. `stderr` diagnostics never do.

## 5. `category` cheat sheet

Common `category` values and what they usually mean:

| category | Usually means |
|---|---|
| `Swift Compiler Error` | Type/scope/syntax error in Swift. Read `message`, jump to `file:line`. |
| `Swift Compiler Warning` | Non-blocking Swift warning. |
| `Objective-C Compiler Error` | Clang error in `.m`/`.mm`/`.h`. |
| `Linker Error` | Undefined symbol, duplicate symbol, missing framework. Often a missing import or Podfile issue — NOT a syntax error. Check linked frameworks / pod install. |
| `Build System Error` | xcodebuild itself complains (circular dependency, missing target). Often config-level. |
| `Signing Error` | Code signing / provisioning profile. Not a code fix — ask the user about the team/profile. |
| `File Not Found Error` | A referenced file is missing or wasn't added to the target. Check project membership, not just disk. |
| `null` | Uncategorized. Use `severity` + `message`. |

Linker/Signing/File-Not-Found errors are the trap: they look like code errors but the fix is in project config, Pods, or signing — NOT the source file at the cited line. Reach for `xcode_pod` or ask the user before editing code.

## 6. `parse_warnings`

```json
{ "diagnostics": [...], "parse_warnings": ["regex failed: ..."] }
```

Non-fatal notes from the diagnostic parser itself, not from the build. Almost always empty. If present, the `diagnostics[]` list may be incomplete — surface this to the user and consider reading `BuildOutput.log_path` directly for the raw build log.

## 7. Building a fix plan

1. **Group by file, sort by line** — fix top-down within a file; one fix often clears cascading errors below it.
2. **Distinguish code errors from config errors** (§5). Don't edit source for a Linker/Signing error.
3. **Apply fix-its first** when they match root cause — they're compiler-authored and usually correct.
4. **One change, then rebuild** — don't batch speculative fixes; the rebuild is the proof. Loop back to `xcode_build`.
5. **Watch error count trend** — if `error_count` rises after a change, revert. If it falls but new errors appear, those were hidden behind the first — expected, keep going.
6. **Stop after ~5 iterations** without progress — summarize the stuck error and ask the user. Don't thrash.
