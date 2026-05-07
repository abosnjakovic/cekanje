#!/usr/bin/env bash
# publish_crates.sh — local crates.io dry-run / publish helper.
#
# Usage:
#   scripts/publish_crates.sh --dry-run   # cargo publish --dry-run
#   scripts/publish_crates.sh             # interactive real publish

set -euo pipefail

CRATE="cekanje"

dry_run=0
if [[ "${1-}" == "--dry-run" ]]; then
  dry_run=1
fi

version=$(grep '^version' Cargo.toml | head -1 | cut -d'"' -f2)
echo "Crate:   ${CRATE}"
echo "Version: ${version}"

# Discoverability check. Tolerate 404 — first-publish case.
if ! status=$(curl -sS -o /dev/null -w '%{http_code}' \
  "https://crates.io/api/v1/crates/${CRATE}"); then
  echo "warn: could not reach crates.io; continuing." >&2
elif [[ "${status}" == "404" ]]; then
  echo "Note: ${CRATE} not yet on crates.io — first publish."
elif [[ "${status}" == "200" ]]; then
  echo "Note: ${CRATE} is registered on crates.io."
else
  echo "warn: unexpected crates.io HTTP ${status}; continuing." >&2
fi

if (( dry_run )); then
  echo "Running cargo publish --dry-run…"
  cargo publish --dry-run
  echo "✓ Dry-run passed."
  exit 0
fi

if [[ -z "${CRATES_IO_TOKEN-}" ]]; then
  echo "error: CRATES_IO_TOKEN not set (see scripts/.env.example)." >&2
  exit 1
fi

read -r -p "Publish ${CRATE} ${version} to crates.io? (y/N) " reply
if [[ ! "${reply}" =~ ^[Yy]$ ]]; then
  echo "Cancelled."
  exit 0
fi

CARGO_REGISTRY_TOKEN="${CRATES_IO_TOKEN}" cargo publish
echo "✓ Published ${CRATE} ${version}."
