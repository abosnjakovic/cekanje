# cekanje Development Makefile

-include scripts/.env
export

# Colors
RED    := \033[0;31m
GREEN  := \033[0;32m
YELLOW := \033[0;33m
BLUE   := \033[0;34m
NC     := \033[0m

CRATE   := cekanje
VERSION := $(shell grep '^version' Cargo.toml | head -1 | cut -d'"' -f2)

.DEFAULT_GOAL := help

.PHONY: help
help:
	@echo "$(BLUE)$(CRATE) — Development$(NC)"
	@echo ""
	@echo "$(GREEN)Setup:$(NC)"
	@echo "  make setup            - Copy scripts/.env.example to scripts/.env"
	@echo "  make check-env        - Verify CRATES_IO_TOKEN + HOMEBREW_TAP_TOKEN"
	@echo ""
	@echo "$(GREEN)Quality:$(NC)"
	@echo "  make lint             - cargo fmt + cargo clippy --all-targets -- -D warnings"
	@echo "  make test             - cargo test"
	@echo "  make doc              - cargo doc with -D warnings"
	@echo "  make check            - lint + test + doc (mirrors CI)"
	@echo ""
	@echo "$(GREEN)Build:$(NC)"
	@echo "  make build            - Debug build"
	@echo "  make build-release    - Release build for host"
	@echo "  make build-apple      - Release build for aarch64-apple-darwin"
	@echo "  make build-all        - Release build for all release targets"
	@echo "  make create-archives  - tar host release binary into target/release-archives/"
	@echo ""
	@echo "$(GREEN)Local install:$(NC)"
	@echo "  make install-local    - cargo install --path . --force, then symlink cek → cekanje"
	@echo "  make uninstall-local  - Remove cek symlink and cargo-uninstall the crate"
	@echo ""
	@echo "$(GREEN)Dry-runs:$(NC)"
	@echo "  make test-crates      - cargo publish --dry-run"
	@echo "  make test-homebrew    - Generate target/homebrew/$(CRATE).rb"
	@echo "  make test-release     - Full local release simulation"
	@echo ""
	@echo "$(GREEN)Publish (interactive):$(NC)"
	@echo "  make publish-crates   - Publish to crates.io"
	@echo "  make publish-homebrew - Update Formula/$(CRATE).rb and push to main"
	@echo ""
	@echo "$(GREEN)release-plz (manual):$(NC)"
	@echo "  make release-plz-update - Preview version bump + changelog (no PR)"
	@echo "  make release-plz-pr     - Open release PR on GitHub"
	@echo ""
	@echo "$(GREEN)Utilities:$(NC)"
	@echo "  make run              - cargo run"
	@echo "  make clean            - cargo clean"
	@echo ""
	@echo "$(YELLOW)Current version: $(VERSION)$(NC)"

# ── Setup ────────────────────────────────────────────────────────────────

.PHONY: setup
setup:
	@if [ -f scripts/.env ]; then \
		echo "$(YELLOW)scripts/.env already exists; not overwriting.$(NC)"; \
	else \
		cp scripts/.env.example scripts/.env; \
		echo "$(GREEN)✓ Wrote scripts/.env (edit it to fill in tokens)$(NC)"; \
	fi

.PHONY: check-env
check-env:
	@missing=""; \
	[ -n "$$CRATES_IO_TOKEN" ]    || missing="$$missing CRATES_IO_TOKEN"; \
	[ -n "$$HOMEBREW_TAP_TOKEN" ] || missing="$$missing HOMEBREW_TAP_TOKEN"; \
	if [ -n "$$missing" ]; then \
		echo "$(RED)Missing:$$missing$(NC)"; \
		echo "Set them in scripts/.env (see scripts/.env.example)."; \
		exit 1; \
	fi; \
	echo "$(GREEN)✓ All required tokens present$(NC)"

# ── Quality ──────────────────────────────────────────────────────────────

.PHONY: lint
lint:
	@echo "$(BLUE)cargo fmt$(NC)"
	cargo fmt
	@echo "$(BLUE)cargo clippy --all-targets -- -D warnings$(NC)"
	cargo clippy --all-targets -- -D warnings
	@echo "$(GREEN)✓ Lint passed$(NC)"

.PHONY: test
test:
	cargo test --verbose

.PHONY: doc
doc:
	RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --document-private-items

.PHONY: check
check: lint test doc
	@echo "$(GREEN)✓ All checks passed$(NC)"

# ── Build ────────────────────────────────────────────────────────────────

.PHONY: run
run:
	cargo run

.PHONY: build
build:
	cargo build

.PHONY: build-release
build-release:
	cargo build --release

.PHONY: build-apple
build-apple:
	cargo build --release --target aarch64-apple-darwin

.PHONY: build-all
build-all:
	cargo build --release --target aarch64-apple-darwin
	cargo build --release --target x86_64-apple-darwin
	cargo build --release --target x86_64-unknown-linux-gnu

.PHONY: create-archives
create-archives: build-release
	@mkdir -p target/release-archives
	@tar -C target/release -czf "target/release-archives/$(CRATE)-$(VERSION)-host.tar.gz" $(CRATE)
	@echo "$(GREEN)✓ target/release-archives/$(CRATE)-$(VERSION)-host.tar.gz$(NC)"

# ── Local install ────────────────────────────────────────────────────────

.PHONY: install-local
install-local:
	cargo install --path . --force
	@bin_dir="$${CARGO_HOME:-$$HOME/.cargo}/bin"; \
		ln -sf $(CRATE) "$$bin_dir/cek"; \
		echo "$(GREEN)✓ Installed $(CRATE) $(VERSION) to $$bin_dir (cek → $(CRATE))$(NC)"; \
		case ":$$PATH:" in \
			*":$$bin_dir:"*) ;; \
			*) echo "$(YELLOW)⚠  $$bin_dir is not on \$$PATH — add it to use cek/cekanje$(NC)" ;; \
		esac

.PHONY: uninstall-local
uninstall-local:
	@bin_dir="$${CARGO_HOME:-$$HOME/.cargo}/bin"; \
		rm -f "$$bin_dir/cek"
	cargo uninstall $(CRATE) || true
	@echo "$(GREEN)✓ Uninstalled $(CRATE)$(NC)"

# ── Dry-runs ─────────────────────────────────────────────────────────────

.PHONY: test-crates
test-crates:
	./scripts/publish_crates.sh --dry-run

.PHONY: test-homebrew
test-homebrew:
	./scripts/publish_homebrew.sh --dry-run

.PHONY: test-release
test-release:
	./scripts/test_release.sh

# ── Publish ──────────────────────────────────────────────────────────────

.PHONY: publish-crates
publish-crates: check-env
	./scripts/publish_crates.sh

.PHONY: publish-homebrew
publish-homebrew: check-env
	./scripts/publish_homebrew.sh

# ── release-plz (manual) ─────────────────────────────────────────────────

.PHONY: release-plz-update
release-plz-update:
	@command -v release-plz >/dev/null 2>&1 || { \
		echo "$(RED)release-plz not installed. Run: cargo install release-plz$(NC)"; exit 1; }
	release-plz update

.PHONY: release-plz-pr
release-plz-pr:
	@command -v release-plz >/dev/null 2>&1 || { \
		echo "$(RED)release-plz not installed. Run: cargo install release-plz$(NC)"; exit 1; }
	@command -v gh >/dev/null 2>&1 || { \
		echo "$(RED)gh CLI required for token.$(NC)"; exit 1; }
	release-plz release-pr --git-token "$$(gh auth token)"

# ── Utilities ────────────────────────────────────────────────────────────

.PHONY: clean
clean:
	cargo clean
