# cekanje release-testing Makefile
# Local dry-runs and helpers that mirror the GitHub Actions release pipeline.

-include scripts/.env
export

RED := \033[0;31m
GREEN := \033[0;32m
YELLOW := \033[0;33m
BLUE := \033[0;34m
NC := \033[0m

.PHONY: help
help:
	@echo "$(BLUE)cekanje release-testing commands$(NC)"
	@echo ""
	@echo "$(GREEN)Setup:$(NC)"
	@echo "  make setup            - Copy scripts/.env.example to scripts/.env"
	@echo ""
	@echo "$(GREEN)Dev gates (mirror CI):$(NC)"
	@echo "  make lint             - cargo fmt + clippy -D warnings"
	@echo "  make test             - cargo test"
	@echo "  make doc              - cargo doc --no-deps"
	@echo ""
	@echo "$(GREEN)Build:$(NC)"
	@echo "  make build-release    - cargo build --release for host"
	@echo "  make build-apple      - cargo build --release for aarch64-apple-darwin"
	@echo "  make create-archives  - Tar host release binary into target/release-archives/"
	@echo ""
	@echo "$(GREEN)Dry-runs:$(NC)"
	@echo "  make test-crates      - cargo publish --dry-run"
	@echo "  make test-homebrew    - Generate Formula at target/homebrew/cekanje.rb"
	@echo "  make test-release     - Full local simulation of the release flow"
	@echo ""
	@echo "$(GREEN)Publish (real):$(NC)"
	@echo "  make publish-crates   - Publish to crates.io (prompts for confirmation)"
	@echo "  make publish-homebrew - Update Formula/cekanje.rb and push"
	@echo ""
	@echo "$(GREEN)Utilities:$(NC)"
	@echo "  make check-env        - Verify required tokens are set"
	@echo "  make clean            - cargo clean"

.PHONY: setup
setup:
	@if [ ! -f scripts/.env ]; then \
		cp scripts/.env.example scripts/.env; \
		echo "$(GREEN)✓ Created scripts/.env$(NC)"; \
		echo "$(YELLOW)⚠ Edit scripts/.env and fill in your tokens$(NC)"; \
	else \
		echo "$(YELLOW)scripts/.env already exists$(NC)"; \
	fi

.PHONY: check-env
check-env:
	@if [ -z "$(CRATES_IO_TOKEN)" ]; then echo "$(RED)✗ CRATES_IO_TOKEN not set$(NC)"; exit 1; fi
	@if [ -z "$(HOMEBREW_TAP_TOKEN)" ]; then echo "$(RED)✗ HOMEBREW_TAP_TOKEN not set$(NC)"; exit 1; fi
	@echo "$(GREEN)✓ Required tokens set$(NC)"

.PHONY: lint
lint:
	cargo fmt
	cargo clippy --all-targets -- -D warnings

.PHONY: test
test:
	cargo test

.PHONY: doc
doc:
	RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --document-private-items

.PHONY: build-release
build-release:
	cargo build --release

.PHONY: build-apple
build-apple:
	cargo build --release --target aarch64-apple-darwin

.PHONY: create-archives
create-archives: build-release
	@version=$$(grep '^version' Cargo.toml | head -1 | cut -d'"' -f2); \
	host_target=$$(rustc -vV | awk '/^host:/ {print $$2}'); \
	mkdir -p target/release-archives; \
	cd target/release && \
	tar czf ../release-archives/cekanje-$$version-$$host_target.tar.gz cekanje && \
	echo "$(GREEN)✓ Created target/release-archives/cekanje-$$version-$$host_target.tar.gz$(NC)"

.PHONY: test-crates
test-crates:
	@./scripts/publish_crates.sh --dry-run

.PHONY: test-homebrew
test-homebrew:
	@./scripts/publish_homebrew.sh --dry-run

.PHONY: test-release
test-release:
	@./scripts/test_release.sh

.PHONY: publish-crates
publish-crates: check-env
	@echo "$(YELLOW)⚠ Publishing to crates.io for real$(NC)"
	@read -p "Are you sure? (y/N) " -n 1 -r; echo; \
	if [[ $$REPLY =~ ^[Yy]$$ ]]; then ./scripts/publish_crates.sh; else echo "$(YELLOW)Cancelled$(NC)"; fi

.PHONY: publish-homebrew
publish-homebrew: check-env
	@echo "$(YELLOW)⚠ Updating Homebrew formula for real$(NC)"
	@read -p "Are you sure? (y/N) " -n 1 -r; echo; \
	if [[ $$REPLY =~ ^[Yy]$$ ]]; then ./scripts/publish_homebrew.sh; else echo "$(YELLOW)Cancelled$(NC)"; fi

.PHONY: clean
clean:
	cargo clean
