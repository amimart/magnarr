.PHONY: fmt fmt-check lint lint-rust lint-md lint-yaml build test audit check fix fix-rust fix-md

BOLD  := \033[1m
RESET := \033[0m
CYAN  := \033[36m
GREEN := \033[32m
YELLOW := \033[33m

fmt:
	@printf "$(CYAN)$(BOLD)🎨 Formatting...$(RESET)\n"
	cargo fmt

fmt-check:
	@printf "$(CYAN)$(BOLD)🎨 Checking formatting...$(RESET)\n"
	cargo fmt --check

lint-rust:
	@printf "$(CYAN)$(BOLD)🦀 Linting Rust...$(RESET)\n"
	cargo clippy -- -D warnings

lint-md:
	@printf "$(CYAN)$(BOLD)📝 Linting Markdown...$(RESET)\n"
	npx markdownlint-cli2 "**/*.md"

lint-yaml:
	@printf "$(CYAN)$(BOLD)📋 Linting YAML...$(RESET)\n"
	yamllint .

## Run all linters
lint: lint-rust lint-md lint-yaml

build:
	@printf "$(GREEN)$(BOLD)🔨 Building...$(RESET)\n"
	cargo build

test:
	@printf "$(GREEN)$(BOLD)🧪 Running tests...$(RESET)\n"
	cargo test

audit:
	@printf "$(YELLOW)$(BOLD)🔒 Running security audit...$(RESET)\n"
	cargo audit

## Run all checks (mirrors CI)
check: fmt-check lint build test audit

## Auto-fix formatting and clippy lints
fix-rust:
	@printf "$(CYAN)$(BOLD)🔧 Fixing Rust...$(RESET)\n"
	cargo fmt
	cargo clippy --fix --allow-dirty

fix-md:
	@printf "$(CYAN)$(BOLD)🔧 Fixing Markdown...$(RESET)\n"
	npx markdownlint-cli2 --fix "**/*.md"

## Auto-fix all (yaml has no auto-fixer)
fix: fix-rust fix-md
