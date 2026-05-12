class Cekanje < Formula
  desc "tmux notifier daemon for Claude Code sessions"
  homepage "https://github.com/abosnjakovic/cekanje"
  version "0.2.1"
  license "MIT"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/abosnjakovic/cekanje/releases/download/v0.2.1/cekanje-0.2.1-aarch64-apple-darwin.tar.gz"
      sha256 "dd24f7df4dba08c67c7a75d802d2b7697fcd0514779d839ddda50baee8ad8b09"
    else
      url "https://github.com/abosnjakovic/cekanje/releases/download/v0.2.1/cekanje-0.2.1-x86_64-apple-darwin.tar.gz"
      sha256 "b9fcc39066d029636267ccdceb64d6fd780bb4c36898f96e9f2bede2c2dee133"
    end
  end

  on_linux do
    url "https://github.com/abosnjakovic/cekanje/releases/download/v0.2.1/cekanje-0.2.1-x86_64-unknown-linux-gnu.tar.gz"
    sha256 "199ac4f98fd30d11244d37648b5c3ed98fb48e901a78cc45313cb024ab48019c"
  end

  def install
    bin.install "cekanje"
    bin.install_symlink "cekanje" => "cek"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/cekanje --version")
  end
end
