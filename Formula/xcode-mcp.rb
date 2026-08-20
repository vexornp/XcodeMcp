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
  sha256 "2f701b172b81a240477cfe826eeba0a55e053baaf710c66b015752f60561c304"
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
