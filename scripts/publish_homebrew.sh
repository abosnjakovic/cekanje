#!/usr/bin/env bash
# publish_homebrew.sh — generate Homebrew formula locally.
#
# Usage:
#   scripts/publish_homebrew.sh --dry-run   # write target/homebrew/cekanje.rb
#   scripts/publish_homebrew.sh             # interactive: regenerate + commit + push to main
#
# In dry-run mode, SHA256s are computed from existing release tarballs on
# GitHub if they exist for the current Cargo version; otherwise zeros.

set -euo pipefail

CRATE="cekanje"
REPO="abosnjakovic/${CRATE}"

dry_run=0
if [[ "${1-}" == "--dry-run" ]]; then
  dry_run=1
fi

version=$(grep '^version' Cargo.toml | head -1 | cut -d'"' -f2)
tag="v${version}"
base="https://github.com/${REPO}/releases/download/${tag}"

zero_sha="0000000000000000000000000000000000000000000000000000000000000000"

fetch_sha() {
  local url="$1"
  local sha
  if sha=$(curl -fsSL "${url}" 2>/dev/null | sha256sum | cut -d' ' -f1); then
    echo "${sha}"
  else
    echo "${zero_sha}"
  fi
}

aarch64_mac=$(fetch_sha "${base}/${CRATE}-${version}-aarch64-apple-darwin.tar.gz")
x86_mac=$(fetch_sha     "${base}/${CRATE}-${version}-x86_64-apple-darwin.tar.gz")
x86_linux=$(fetch_sha   "${base}/${CRATE}-${version}-x86_64-unknown-linux-gnu.tar.gz")

write_formula() {
  local out="$1"
  mkdir -p "$(dirname "${out}")"
  cat > "${out}" <<EOF
class Cekanje < Formula
  desc "tmux notifier daemon for Claude Code sessions"
  homepage "https://github.com/${REPO}"
  version "${version}"
  license "MIT"

  on_macos do
    if Hardware::CPU.arm?
      url "${base}/${CRATE}-${version}-aarch64-apple-darwin.tar.gz"
      sha256 "${aarch64_mac}"
    else
      url "${base}/${CRATE}-${version}-x86_64-apple-darwin.tar.gz"
      sha256 "${x86_mac}"
    end
  end

  on_linux do
    url "${base}/${CRATE}-${version}-x86_64-unknown-linux-gnu.tar.gz"
    sha256 "${x86_linux}"
  end

  def install
    bin.install "cekanje"
    bin.install_symlink "cekanje" => "cek"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/cekanje --version")
  end
end
EOF
}

if (( dry_run )); then
  out="target/homebrew/${CRATE}.rb"
  write_formula "${out}"
  echo "✓ Wrote ${out}"
  echo "  aarch64_mac: ${aarch64_mac}"
  echo "  x86_mac:     ${x86_mac}"
  echo "  x86_linux:   ${x86_linux}"
  exit 0
fi

if [[ -z "${HOMEBREW_TAP_TOKEN-}" ]]; then
  echo "error: HOMEBREW_TAP_TOKEN not set (see scripts/.env.example)." >&2
  exit 1
fi

if [[ "${aarch64_mac}" == "${zero_sha}" || "${x86_mac}" == "${zero_sha}" || "${x86_linux}" == "${zero_sha}" ]]; then
  echo "error: at least one release tarball is missing for ${tag}." >&2
  echo "       publish the GitHub release first, then re-run." >&2
  exit 1
fi

write_formula "Formula/${CRATE}.rb"
echo "Generated Formula/${CRATE}.rb"

read -r -p "Commit and push to main? (y/N) " reply
if [[ ! "${reply}" =~ ^[Yy]$ ]]; then
  echo "Cancelled."
  exit 0
fi

git add "Formula/${CRATE}.rb"
git commit -m "chore: update homebrew formula to ${version}"
git push origin HEAD:main
echo "✓ Pushed formula update for ${version}."
