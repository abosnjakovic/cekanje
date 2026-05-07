#!/usr/bin/env bash
# test_release.sh — full local simulation. No network mutations.
#
# - Builds release binaries for available targets.
# - Tar archives them under target/release-archives/.
# - Runs cargo publish --dry-run.
# - Generates target/homebrew/cekanje.rb.

set -euo pipefail

CRATE="cekanje"

version=$(grep '^version' Cargo.toml | head -1 | cut -d'"' -f2)
echo "→ Simulating release of ${CRATE} ${version}"

targets=(aarch64-apple-darwin x86_64-apple-darwin x86_64-unknown-linux-gnu)

archives_dir="target/release-archives"
mkdir -p "${archives_dir}"

for target in "${targets[@]}"; do
  if ! rustup target list --installed | grep -qx "${target}"; then
    echo "→ Skipping ${target} (toolchain not installed locally)"
    continue
  fi
  echo "→ Building ${target}"
  cargo build --release --target "${target}"

  archive="${archives_dir}/${CRATE}-${version}-${target}.tar.gz"
  tar -C "target/${target}/release" -czf "${archive}" "${CRATE}"
  echo "  wrote ${archive}"
done

echo "→ cargo publish --dry-run"
cargo publish --dry-run

echo "→ Homebrew formula dry-run"
scripts/publish_homebrew.sh --dry-run

echo "✓ Local release simulation complete."
