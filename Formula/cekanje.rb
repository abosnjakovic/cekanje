class Cekanje < Formula
  desc "tmux notifier daemon for Claude Code sessions"
  homepage "https://github.com/abosnjakovic/cekanje"
  version "0.1.1"
  license "MIT"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/abosnjakovic/cekanje/releases/download/v0.1.1/cekanje-0.1.1-aarch64-apple-darwin.tar.gz"
      sha256 "8b35ced813583273c56cb41c747d75e794f3957bd48dc88c72e2850f89b96bde"
    else
      url "https://github.com/abosnjakovic/cekanje/releases/download/v0.1.1/cekanje-0.1.1-x86_64-apple-darwin.tar.gz"
      sha256 "a5cfa5c8d3f7391afbf63a3d10726197dfb2ba5f0970ab34638a8e261468193b"
    end
  end

  on_linux do
    url "https://github.com/abosnjakovic/cekanje/releases/download/v0.1.1/cekanje-0.1.1-x86_64-unknown-linux-gnu.tar.gz"
    sha256 "7d54c25715a8cb3379a34a7b67af25f05dee8c9b91dbfb1db1ceeb1338701471"
  end

  def install
    bin.install "cekanje"
    bin.install_symlink "cekanje" => "cek"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/cekanje --version")
  end
end
