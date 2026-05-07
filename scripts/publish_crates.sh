#!/bin/bash
# Publish to crates.io. Run with --dry-run to validate without publishing.
set -e

DEBUG_MODE=false
DRY_RUN_FLAG=""

for arg in "$@"; do
    case $arg in
        --dry-run) DRY_RUN_FLAG="--dry-run" ;;
        --debug) DEBUG_MODE=true ;;
        --help)
            echo "Usage: $0 [--dry-run] [--debug]"
            exit 0
            ;;
    esac
done

if [ "$DEBUG_MODE" = true ]; then
    set -x
fi

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
BLUE='\033[0;34m'
NC='\033[0m'

if [ -f "scripts/.env" ]; then
    # shellcheck source=/dev/null
    source scripts/.env
fi

if [[ "$DRY_RUN_FLAG" == "--dry-run" ]] || [[ "$DRY_RUN" == "true" ]]; then
    DRY_RUN_FLAG="--dry-run"
    echo -e "${BLUE}=== crates.io publish (DRY RUN) ===${NC}"
else
    echo -e "${YELLOW}=== crates.io publish (REAL) ===${NC}"
fi
echo ""

if [ -z "$CRATES_IO_TOKEN" ] && [ -z "$DRY_RUN_FLAG" ]; then
    echo -e "${RED}CRATES_IO_TOKEN not set${NC}"
    echo "Get one at https://crates.io/settings/tokens"
    exit 1
fi

current_version=$(grep '^version' Cargo.toml | head -1 | cut -d'"' -f2)
package_name=$(grep '^name' Cargo.toml | head -1 | cut -d'"' -f2)

echo -e "${BLUE}Package:${NC} ${package_name} ${current_version}"
echo ""

echo -e "${BLUE}Step 1: Check crates.io for existing version${NC}"
if curl -s "https://crates.io/api/v1/crates/${package_name}" | grep -q "\"name\":\"${package_name}\""; then
    latest_version=$(curl -s "https://crates.io/api/v1/crates/${package_name}" | grep -o '"max_version":"[^"]*' | cut -d'"' -f4)
    echo "  Latest published: ${latest_version}"
    if [ "$latest_version" == "$current_version" ] && [ -z "$DRY_RUN_FLAG" ]; then
        echo -e "${RED}Version ${current_version} already published${NC}"
        exit 1
    fi
else
    echo "  Package not on crates.io yet (will be created on first publish)"
fi
echo ""

echo -e "${BLUE}Step 2: Validate Cargo.toml metadata${NC}"
required=("name" "version" "edition" "description" "license")
for field in "${required[@]}"; do
    if ! grep -q "^${field}" Cargo.toml; then
        echo -e "${RED}✗ Missing required field: ${field}${NC}"
        exit 1
    fi
done
echo -e "${GREEN}✓ Required fields present${NC}"
echo ""

echo -e "${BLUE}Step 3: Release build${NC}"
cargo build --release --quiet
echo -e "${GREEN}✓ Build OK${NC}"
echo ""

echo -e "${BLUE}Step 4: Run tests${NC}"
cargo test --quiet || echo -e "${YELLOW}⚠ Some tests failed (continuing)${NC}"
echo ""

echo -e "${BLUE}Step 5: Package${NC}"
set +e
if cargo package --list > /tmp/cekanje-package-list.txt 2>/dev/null; then
    echo "  Files to include: $(wc -l < /tmp/cekanje-package-list.txt)"
fi
set -e
echo ""

echo -e "${BLUE}Step 6: Publish${NC}"
if [ -n "$DRY_RUN_FLAG" ]; then
    cargo publish --dry-run --allow-dirty
    echo ""
    echo -e "${GREEN}✓ Dry-run successful${NC}"
    echo "Run 'make publish-crates' to publish for real."
else
    read -p "Publish ${package_name} v${current_version}? (y/N) " -n 1 -r
    echo
    if [[ $REPLY =~ ^[Yy]$ ]]; then
        cargo publish --allow-dirty --token "${CRATES_IO_TOKEN}"
        echo -e "${GREEN}✓ Published${NC}"
        echo "  https://crates.io/crates/${package_name}"
    else
        echo -e "${YELLOW}Cancelled${NC}"
    fi
fi
