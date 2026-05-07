class Cekanje < Formula
  desc "tmux notifier daemon for Claude Code sessions"
  homepage "https://github.com/abosnjakovic/cekanje"
  version "0.1.0"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/abosnjakovic/cekanje/releases/download/v0.1.0/cekanje-0.1.0-aarch64-apple-darwin.tar.gz"
      sha256 "af20ccb3d61f75df18dbf30681baa5eb205906eaed516cef1cf0acf1f7db6513"
    else
      url "https://github.com/abosnjakovic/cekanje/releases/download/v0.1.0/cekanje-0.1.0-x86_64-apple-darwin.tar.gz"
      sha256 "cafebe9095a3a27b64d2ae24a4fa9ec35a97282409c37237152dcd87006c4d17"
    end
  end

  def install
    bin.install "cekanje"
    bin.install_symlink "cekanje" => "cek"
  end

  test do
    system "#{bin}/cekanje", "--version"
  end
end
