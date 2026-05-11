class Cekanje < Formula
  desc "tmux notifier daemon for Claude Code sessions"
  homepage "https://github.com/abosnjakovic/cekanje"
  version "0.2.0"
  license "MIT"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/abosnjakovic/cekanje/releases/download/v0.2.0/cekanje-0.2.0-aarch64-apple-darwin.tar.gz"
      sha256 "9fd4fb0d4855f22194d5f7ca3045fbb05687bcbc5a8963f1c1c8cc9fe6971546"
    else
      url "https://github.com/abosnjakovic/cekanje/releases/download/v0.2.0/cekanje-0.2.0-x86_64-apple-darwin.tar.gz"
      sha256 "e942119db66f0aa5aafd3a7f3515d19d9271b8c7f1ac1648cf1b8788074be9c5"
    end
  end

  on_linux do
    url "https://github.com/abosnjakovic/cekanje/releases/download/v0.2.0/cekanje-0.2.0-x86_64-unknown-linux-gnu.tar.gz"
    sha256 "51f427424142a7f45e4ee152b4935df288e7019b8e971368ea9230e51af0578c"
  end

  def install
    bin.install "cekanje"
    bin.install_symlink "cekanje" => "cek"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/cekanje --version")
  end
end
