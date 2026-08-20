# This formula is intended for a Homebrew tap repo (homebrew-xcode-mcp).
# Copy this file to Formula/xcode-mcp.rb in the tap repo.
#
# Users install via:
#   brew tap YOUR_USERNAME/xcode-mcp
#   brew install xcode-mcp
#
# Or one-liner:
#   brew install YOUR_USERNAME/xcode-mcp/xcode-mcp

class XcodeMcp < Formula
  desc "MCP server that drives xcodebuild and parses build diagnostics"
  homepage "https://github.com/YOUR_USERNAME/XcodeMcp"
  url "https://github.com/YOUR_USERNAME/XcodeMcp/archive/refs/tags/v0.1.0.tar.gz"
  sha256 "REPLACE_WITH_TARBALL_SHA256"
  license "MIT"
  head "https://github.com/YOUR_USERNAME/XcodeMcp.git", branch: "main"

  depends_on "rust" => :build
  on_macos do
    depends_on "xcodebuild" => :test
  end

  def install
    system "cargo", "install", *std_cargo_args(path: "crates/xcode-mcp")
  end

  test do
    assert_match "xcode-mcp", shell_output("#{bin}/xcode-mcp --version")
  end
end
