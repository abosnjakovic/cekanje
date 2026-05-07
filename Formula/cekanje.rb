class Cekanje < Formula
  desc "tmux notifier daemon for Claude Code sessions"
  homepage "https://github.com/abosnjakovic/cekanje"
  version "0.1.1"
  license "MIT"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/abosnjakovic/cekanje/releases/download/v0.1.1/cekanje-0.1.1-aarch64-apple-darwin.tar.gz"
      sha256 "65abbcf24e7e50913dada018d3131eb347680bf003bea45b1d62ee439ba7fa00"
    else
      url "https://github.com/abosnjakovic/cekanje/releases/download/v0.1.1/cekanje-0.1.1-x86_64-apple-darwin.tar.gz"
      sha256 "7c009506f07dc1d1fbadacdde44fba6e01c98f6c7b03016dd0a57e8dbb0eda50"
    end
  end

  on_linux do
    url "https://github.com/abosnjakovic/cekanje/releases/download/v0.1.1/cekanje-0.1.1-x86_64-unknown-linux-gnu.tar.gz"
    sha256 "eadfa5bc52c73d38d24ed82e42f68f094fcc795010cba549a10f6e27f6faedb1"
  end

  def install
    bin.install "cekanje"
    bin.install_symlink "cekanje" => "cek"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/cekanje --version")
  end
end
