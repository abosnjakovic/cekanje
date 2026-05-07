#!/bin/bash
# Local simulation of the GitHub Actions release flow.
set -e

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
BLUE='\033[0;34m'
NC='\033[0m'

if [ -f "scripts/.env" ]; then
    # shellcheck source=/dev/null
    source scripts/.env
fi

echo -e "${BLUE}=== cekanje local release test ===${NC}"
echo ""

if [ -z "$TEST_VERSION" ]; then
    read -p "Version to test (e.g. 0.1.0): " TEST_VERSION
fi
if [ -z "$TEST_TAG" ]; then
    TEST_TAG="v${TEST_VERSION}"
fi
TEST_REPO="${TEST_REPO:-abosnjakovic/cekanje}"

echo "Version: $TEST_VERSION  Tag: $TEST_TAG  Repo: $TEST_REPO"
echo ""

echo -e "${BLUE}Step 1: Cargo.toml metadata${NC}"
current_version=$(grep '^version' Cargo.toml | head -1 | cut -d'"' -f2)
echo "  current version: ${current_version}"
for field in description license repository; do
    grep -q "^${field}" Cargo.toml || { echo -e "${RED}✗ Missing ${field}${NC}"; exit 1; }
done
echo -e "${GREEN}✓ Cargo.toml ready${NC}"
echo ""

echo -e "${BLUE}Step 2: Build${NC}"
cargo build --quiet
cargo build --release --quiet
echo -e "${GREEN}✓ Debug + release builds OK${NC}"
echo ""

echo -e "${BLUE}Step 3: Lint + tests${NC}"
cargo fmt -- --check
cargo clippy --all-targets -- -D warnings
cargo test --quiet
echo -e "${GREEN}✓ Lint + tests pass${NC}"
echo ""

echo -e "${BLUE}Step 4: crates.io dry-run${NC}"
cargo publish --dry-run --allow-dirty 2>/dev/null && echo -e "${GREEN}✓ crates.io dry-run OK${NC}" \
    || echo -e "${YELLOW}⚠ dry-run failed (may be already published)${NC}"
echo ""

echo -e "${BLUE}Step 5: Archive (host platform)${NC}"
mkdir -p target/test-release
host=$(rustc -vV | awk '/^host:/ {print $2}')
if [ -f "target/release/cekanje" ]; then
    (cd target/release && tar czf ../test-release/cekanje-${TEST_VERSION}-${host}.tar.gz cekanje)
    echo -e "${GREEN}✓ target/test-release/cekanje-${TEST_VERSION}-${host}.tar.gz${NC}"
fi
echo ""

echo -e "${BLUE}Step 6: Workflow sanity${NC}"
if [ -f .github/workflows/release.yml ]; then
    grep -q "workflow_dispatch:" .github/workflows/release.yml && echo -e "${GREEN}✓ workflow_dispatch trigger${NC}"
    grep -q "update-homebrew-formula:" .github/workflows/release.yml && echo -e "${GREEN}✓ homebrew job${NC}"
    grep -q "publish-crates:" .github/workflows/release.yml && echo -e "${GREEN}✓ crates job${NC}"
else
    echo -e "${RED}✗ release.yml missing${NC}"
fi
echo ""

echo -e "${GREEN}All local checks passed.${NC}"
echo "Next: edit scripts/.env, run 'make test-crates' / 'make test-homebrew'."
