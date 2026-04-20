class ClaudeBoop < Formula
  desc "Cute notifications for Claude Code (permission prompts & generation complete)"
  homepage "https://github.com/akeenkarkare/homebrew-claude-boop"
  url "https://github.com/akeenkarkare/homebrew-claude-boop/archive/refs/tags/v0.1.0.tar.gz"
  sha256 "8b426af982ee65094ff5b1c3bc9aad234ade655ad9562eefff8fa41213f821b2"
  license "MIT"
  head "https://github.com/akeenkarkare/homebrew-claude-boop.git", branch: "main"

  depends_on "rust" => :build

  def install
    system "cargo", "install", *std_cargo_args
  end

  test do
    assert_match "claude-boop", shell_output("#{bin}/claude-boop --version")
  end
end
