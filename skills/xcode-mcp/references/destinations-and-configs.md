# Destinations & Configurations

Common `destination` and `configuration` values for `xcode_build`. Load this when confirming build intent with the user (Step 3 of the workflow).

## `configuration`

| Value | When |
|---|---|
| `"Debug"` | Day-to-day development, debugging, unit tests. Default for most schemes. |
| `"Release"` | App Store / TestFlight builds, performance testing. Strips debug symbols, enables optimization. |

If omitted, xcodebuild uses the scheme's default configuration (usually `Debug`). Pass it explicitly when the user asks for a "release build" or "archive build".

> Only `Debug` and `Release` are accepted. Custom configurations (e.g. `Staging`) are rejected by the server's validator — if a project defines one, ask the user to build via `Debug`/`Release` or reconfigure.

## `destination`

The `destination` flag tells xcodebuild **what to build for**. Omitting it often causes xcodebuild to silently build for the macOS host — usually wrong for iOS projects. Always set it.

### iOS apps
| destination | Use when |
|---|---|
| `generic/platform=iOS` | **Default for iOS app builds.** Builds for any iOS device, no specific device required. Use for "build the app". |
| `platform=iOS,name=My iPhone` | Build for a specific connected device. Requires the device plugged in. |
| `generic/platform=iOS Simulator` | Build for the simulator without targeting a specific simulator. |
| `platform=iOS Simulator,name=iPhone 15` | Build for a specific simulator. Requires the simulator name to match an installed one. |

### macOS apps
| destination | Use when |
|---|---|
| `platform=macOS` | Mac app, Mac Catalyst, or building the macOS variant of a multiplatform app. |
| `platform=macOS,arch=arm64` | Force a specific architecture (Apple Silicon). Rarely needed. |

### watchOS / tvOS / visionOS
| destination | Use when |
|---|---|
| `generic/platform=watchOS` | watchOS app |
| `generic/platform=tvOS` | tvOS app |
| `generic/platform=visionOS` | visionOS app (Xcode 15+) |

### Tests
| destination | Use when |
|---|---|
| `platform=iOS Simulator,name=iPhone 15` | Running unit/UI tests (tests need a concrete simulator, not `generic`). |

## Charset & length

`destination` is validated against `^[A-Za-z0-9_ ./=\-,]{1,256}$`. This covers all the values above. If a value is rejected as `invalid destination`, it usually contains an unexpected character (e.g. a smart quote from copy-paste) — retype it.

## Choosing defaults (quick guide)

When the user says something vague, propose these and confirm:

| User says | Propose |
|---|---|
| "build the app" (iOS) | scheme from `list_schemes`, `configuration=Debug`, `destination=generic/platform=iOS` |
| "build for simulator" | `destination=platform=iOS Simulator,name=iPhone 15` (confirm simulator name) |
| "release build" / "archive" | `configuration=Release`, `destination=generic/platform=iOS` |
| "build the Mac app" | `destination=platform=macOS` |
| "run the tests" | scheme `<App>Tests`, `destination=platform=iOS Simulator,name=...` |

Always confirm with the user before building — these are proposals, not defaults to silently apply (Step 3 gate).
