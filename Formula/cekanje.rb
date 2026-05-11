class Cekanje < Formula
  desc "tmux notifier daemon for Claude Code sessions"
  homepage "https://github.com/abosnjakovic/cekanje"
  version "0.1.3"
  license "MIT"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/abosnjakovic/cekanje/releases/download/v0.1.3/cekanje-0.1.3-aarch64-apple-darwin.tar.gz"
      sha256 "a6ac033790e35b271ce4dcbc0fc1f1993b48c9aa08d6ec719f57c250cd6d0a94"
    else
      url "https://github.com/abosnjakovic/cekanje/releases/download/v0.1.3/cekanje-0.1.3-x86_64-apple-darwin.tar.gz"
      sha256 "1a599dba148bf1c658b569cb12f78a13bd5d9989982113da8d8a4b141cf4e673"
    end
  end

  on_linux do
    url "https://github.com/abosnjakovic/cekanje/releases/download/v0.1.3/cekanje-0.1.3-x86_64-unknown-linux-gnu.tar.gz"
    sha256 "9d1aa5cd5bdaf047b585dd6b2db956912e885e7c2857515e90300a2af4f21545"
  end

  def install
    bin.install "cekanje"
    bin.install_symlink "cekanje" => "cek"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/cekanje --version")
  end
end
