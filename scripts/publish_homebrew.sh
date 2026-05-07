#!/bin/bash
# Generate (and optionally commit) the Homebrew formula in this repo's Formula/.
# Run with --dry-run to only generate at target/homebrew/cekanje.rb.
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

DRY_RUN_MODE=false
if [[ "$1" == "--dry-run" ]] || [[ "$DRY_RUN" == "true" ]]; then
    DRY_RUN_MODE=true
    echo -e "${BLUE}=== Homebrew formula (DRY RUN) ===${NC}"
else
    echo -e "${YELLOW}=== Homebrew formula (REAL) ===${NC}"
fi
echo ""

if [ "$DRY_RUN_MODE" = false ] && [ -z "$HOMEBREW_TAP_TOKEN" ]; then
    echo -e "${RED}HOMEBREW_TAP_TOKEN not set${NC}"
    exit 1
fi

VERSION=$(grep '^version' Cargo.toml | head -1 | cut -d'"' -f2)
PACKAGE_NAME=$(grep '^name' Cargo.toml | head -1 | cut -d'"' -f2)
TAG="v${VERSION}"
REPO="${TEST_REPO:-abosnjakovic/cekanje}"
REPO_OWNER=$(echo "$REPO" | cut -d'/' -f1)

echo -e "${BLUE}Package:${NC} ${PACKAGE_NAME} ${VERSION} (tag ${TAG}, repo ${REPO})"
echo ""

# Locate or create archives. Apple Silicon + Intel.
echo -e "${BLUE}Step 1: Locate release archives${NC}"
mkdir -p target/release-archives

find_archive() {
    local suffix=$1
    local name="cekanje-${VERSION}-${suffix}.tar.gz"
    for loc in target/release-archives target/test-release "target/${suffix}/release" .; do
        if [ -f "${loc}/${name}" ]; then
            echo "${loc}/${name}"
            return 0
        fi
    done
    return 1
}

AARCH64_ARCHIVE=$(find_archive aarch64-apple-darwin || true)
X86_ARCHIVE=$(find_archive x86_64-apple-darwin || true)

if [ -z "$AARCH64_ARCHIVE" ] && [ -f "target/aarch64-apple-darwin/release/cekanje" ]; then
    (cd target/aarch64-apple-darwin/release && tar czf ../../release-archives/cekanje-${VERSION}-aarch64-apple-darwin.tar.gz cekanje)
    AARCH64_ARCHIVE="target/release-archives/cekanje-${VERSION}-aarch64-apple-darwin.tar.gz"
fi
if [ -z "$X86_ARCHIVE" ] && [ -f "target/x86_64-apple-darwin/release/cekanje" ]; then
    (cd target/x86_64-apple-darwin/release && tar czf ../../release-archives/cekanje-${VERSION}-x86_64-apple-darwin.tar.gz cekanje)
    X86_ARCHIVE="target/release-archives/cekanje-${VERSION}-x86_64-apple-darwin.tar.gz"
fi

if [ "$DRY_RUN_MODE" = false ] && { [ -z "$AARCH64_ARCHIVE" ] || [ -z "$X86_ARCHIVE" ]; }; then
    echo -e "${RED}Missing macOS archives. Run cross-builds first or use --dry-run.${NC}"
    echo "  make build-apple      # aarch64"
    echo "  cargo build --release --target x86_64-apple-darwin"
    exit 1
fi

[ -n "$AARCH64_ARCHIVE" ] && echo "  aarch64: $AARCH64_ARCHIVE" || echo "  aarch64: (placeholder, dry-run)"
[ -n "$X86_ARCHIVE" ] && echo "  x86_64:  $X86_ARCHIVE" || echo "  x86_64:  (placeholder, dry-run)"
echo ""

echo -e "${BLUE}Step 2: SHA256${NC}"
if [ -n "$AARCH64_ARCHIVE" ]; then
    AARCH64_SHA256=$(shasum -a 256 "$AARCH64_ARCHIVE" | cut -d' ' -f1)
else
    AARCH64_SHA256="0000000000000000000000000000000000000000000000000000000000000000"
fi
if [ -n "$X86_ARCHIVE" ]; then
    X86_SHA256=$(shasum -a 256 "$X86_ARCHIVE" | cut -d' ' -f1)
else
    X86_SHA256="0000000000000000000000000000000000000000000000000000000000000000"
fi
echo "  aarch64: ${AARCH64_SHA256}"
echo "  x86_64:  ${X86_SHA256}"
echo ""

echo -e "${BLUE}Step 3: Generate formula${NC}"
mkdir -p target/homebrew
cat > target/homebrew/cekanje.rb << EOF
class Cekanje < Formula
  desc "tmux notifier daemon for Claude Code sessions"
  homepage "https://github.com/${REPO}"
  version "${VERSION}"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/${REPO}/releases/download/${TAG}/cekanje-${VERSION}-aarch64-apple-darwin.tar.gz"
      sha256 "${AARCH64_SHA256}"
    else
      url "https://github.com/${REPO}/releases/download/${TAG}/cekanje-${VERSION}-x86_64-apple-darwin.tar.gz"
      sha256 "${X86_SHA256}"
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
EOF
cat target/homebrew/cekanje.rb
echo ""

if [ "$DRY_RUN_MODE" = true ]; then
    echo -e "${GREEN}✓ Dry-run done. Formula at target/homebrew/cekanje.rb${NC}"
    exit 0
fi

echo -e "${BLUE}Step 4: Commit Formula/cekanje.rb to ${REPO}${NC}"
git config --global user.name "github-actions[bot]"
git config --global user.email "41898282+github-actions[bot]@users.noreply.github.com"

TAP_DIR="target/main-repo"
TAP_URL="https://x-access-token:${HOMEBREW_TAP_TOKEN}@github.com/${REPO}.git"

rm -rf "$TAP_DIR"
git clone "${TAP_URL}" "$TAP_DIR"
mkdir -p "$TAP_DIR/Formula"
cp target/homebrew/cekanje.rb "$TAP_DIR/Formula/cekanje.rb"

cd "$TAP_DIR"
git add Formula/cekanje.rb
if git diff --staged --quiet; then
    echo -e "${YELLOW}Formula unchanged, nothing to commit${NC}"
else
    git commit -m "Update ${PACKAGE_NAME} formula to ${VERSION}"
    read -p "Push to ${REPO}? (y/N) " -n 1 -r
    echo
    if [[ $REPLY =~ ^[Yy]$ ]]; then
        git push origin main
        echo -e "${GREEN}✓ Pushed${NC}"
    else
        echo -e "${YELLOW}Cancelled — local clone at ${TAP_DIR}${NC}"
    fi
fi
