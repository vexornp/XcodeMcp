# Xcode MCP Server

A local MCP (Model Context Protocol) server that drives `xcodebuild` and parses build-failure diagnostics into structured data. Written in Rust.

## Requirements

- **Rust** 1.97+
- **Xcode** 15+ (any version with `xcrun xcresulttool get build-results`)
- **macOS** (depends on `xcrun`, `xcodebuild`, `xcresulttool`)

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

# Live integration tests (requires Xcode)
XCODE_MCP_LIVE_TESTS=1 cargo test --features xcode-mcp-core/live-xcode -- --test-threads=1
```

Live tests are double-gated: both the `live-xcode` feature flag AND the `XCODE_MCP_LIVE_TESTS=1` env var must be set. The test fixture (`MiniApp.xcodeproj`) is pre-generated and committed, so no external tools are needed.

## Security Model

- `XCODE_MCP_ROOT` is the single trust boundary. All project paths must canonicalize under it.
- No shell invocation — all `xcodebuild` calls use `Command::new("xcrun").arg(...)`.
- No `extra_args` passthrough — fixed xcodebuild flag surface only.
- Scheme/destination/configuration values are charset-validated to prevent flag injection.
- Build timeout kills the entire process group (SIGTERM -> SIGKILL) to prevent orphaned compiler processes.

## Architecture

Cargo workspace:
- `xcode-mcp-core` (lib): all logic — security validation, scheme parsing, xcresult parsing, stderr parsing, diagnostic merging, process supervision, build store.
- `xcode-mcp` (bin): JSON-RPC 2.0 over stdio MCP server + debug CLI.

The MCP server implements JSON-RPC 2.0 over stdio manually (the `rmcp` crate was unavailable in the build environment). It supports the standard MCP handshake (`initialize`, `tools/list`, `tools/call`, `ping`) and routes protocol errors (missing params, unknown tool) as JSON-RPC error responses, while tool execution failures use MCP's `isError` field.

Diagnostic sourcing is hybrid: primary `xcresulttool get build-results` JSON (structured, with fix-its), fallback stderr regex parsing (for early-exit failures that never produce a result bundle).
