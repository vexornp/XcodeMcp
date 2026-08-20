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

The server needs to know the root directory containing your `.xcodeproj`/`.xcworkspace` files. There are two ways to set this:

### Option 1 — Environment variable (per-process)

Set `XCODE_MCP_ROOT` in the `env` block of your MCP config:

```json
{
  "mcpServers": {
    "xcode": {
      "command": "xcode-mcp",
      "args": ["serve"],
      "env": { "XCODE_MCP_ROOT": "/path/to/your/projects" }
    }
  }
}
```

### Option 2 — Per-user config file (recommended for shared MCP configs)

If `XCODE_MCP_ROOT` is not set, the server reads `~/.config/xcode-mcp/config` (honoring `$XDG_CONFIG_HOME`). This lets each team member set their own path even when sharing a common `mcp.json`:

```bash
mkdir -p ~/.config/xcode-mcp
echo 'root = /Users/yourname/Developer/ios-projects' > ~/.config/xcode-mcp/config
```

The shared `mcp.json` then needs no `env` block at all:

```json
{
  "mcpServers": {
    "xcode": {
      "command": "xcode-mcp",
      "args": ["serve"]
    }
  }
}
```

Config file format (line-based, `#` for comments):
```ini
# my xcode-mcp config
root = /Users/yourname/Developer/ios-projects
```

The environment variable takes precedence over the config file if both are set.

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

## Releasing

Releases are automated via `.github/workflows/release.yml`, which fires on tag push (`v*`). It verifies the tag matches `Cargo.toml`, computes the tarball sha256, renders the Homebrew formula from `Formula/xcode-mcp.rb` (a template with `{{TAG}}` / `{{SHA256}}` placeholders), pushes the rendered formula to `vexornp/homebrew-xcode-mcp`, and creates the GitHub release.

### One-time setup

Create a fine-grained Personal Access Token with `Contents: write` on `vexornp/homebrew-xcode-mcp` (only):

1. https://github.com/settings/personal-access-tokens/new
2. Repository access → select `vexornp/homebrew-xcode-mcp`
3. Permissions → Repository permissions → Contents: `Read and write`
4. Generate, copy the token
5. Add it as a repository secret named `TAP_REPO_TOKEN` at https://github.com/vexornp/XcodeMcp/settings/secrets/actions

### Cutting a release

```bash
./scripts/bump-version.sh 0.2.0
```

This bumps `Cargo.toml`, refreshes `Cargo.lock` via `cargo check`, commits, tags `v0.2.0`, and pushes — which triggers the release workflow. Watch progress at https://github.com/vexornp/XcodeMcp/actions.

When the workflow completes, users upgrade with:

```bash
brew update && brew upgrade xcode-mcp
```

## Security Model

- `XCODE_MCP_ROOT` is the single trust boundary. All project paths must canonicalize under it.
- No shell invocation — all `xcodebuild` calls use `Command::new("xcrun").arg(...)`.
- No `extra_args` passthrough — fixed xcodebuild flag surface only.
- Scheme/destination/configuration values are charset-validated to prevent flag injection.
- Build timeout kills the entire process group (SIGTERM -> SIGKILL) to prevent orphaned compiler processes.

## Architecture

Cargo workspace:
- `xcode-mcp-core` (lib): all logic — security validation, scheme parsing, xcresult parsing, stderr parsing, diagnostic merging, process supervision, build store.
- `xcode-mcp` (bin): MCP server over stdio (using the `rmcp` crate) + debug CLI.

The MCP server uses the [`rmcp`](https://crates.io/crates/rmcp) crate (Rust MCP SDK) for spec-compliant JSON-RPC 2.0 over stdio. It implements `ServerHandler` with `initialize`, `tools/list`, and `tools/call`. Protocol errors (missing params, unknown tool) return JSON-RPC error responses (`-32602`), while tool execution failures use MCP's `isError` field so the caller sees the message.

Diagnostic sourcing is hybrid: primary `xcresulttool get build-results` JSON (structured, with fix-its), fallback stderr regex parsing (for early-exit failures that never produce a result bundle).
