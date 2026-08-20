# This formula is intended for a Homebrew tap repo (homebrew-xcode-mcp).
# Copy this file to Formula/xcode-mcp.rb in the tap repo.
#
# Users install via:
#   brew tap vexornp/xcode-mcp
#   brew install xcode-mcp
#
# Or one-liner:
#   brew install vexornp/xcode-mcp/xcode-mcp

class XcodeMcp < Formula
  desc "MCP server that drives xcodebuild and parses build diagnostics"
  homepage "https://github.com/vexornp/XcodeMcp"
  url "https://github.com/vexornp/XcodeMcp/archive/refs/tags/v0.1.1.tar.gz"
  sha256 "b4e53a474b9e61bc2bb1f581c8220fa99527ceb0c5330e28107677ef2b2516f6"
  license "MIT"
  head "https://github.com/vexornp/XcodeMcp.git", branch: "main"

  depends_on "rust" => :build

  def install
    system "cargo", "install", *std_cargo_args(path: "crates/xcode-mcp")
  end

  test do
    assert_match "xcode-mcp", shell_output("#{bin}/xcode-mcp --version")
  end
end
