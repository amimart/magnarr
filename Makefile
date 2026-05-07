.PHONY: fmt fmt-check lint lint-rust lint-md lint-yaml build test audit check fix fix-rust fix-md help

BOLD   := \033[1m
RESET  := \033[0m
CYAN   := \033[36m
GREEN  := \033[32m
YELLOW := \033[33m

help: ## Show available targets
	@printf "$(BOLD)Available targets:$(RESET)\n"
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' Makefile | awk 'BEGIN {FS = ":.*?## "}; {printf "  $(CYAN)$(BOLD)%-12s$(RESET) %s\n", $$1, $$2}'

fmt: ## Format code
	@printf "$(CYAN)$(BOLD)🎨 Formatting...$(RESET)\n"
	cargo fmt

fmt-check: ## Check formatting without modifying files
	@printf "$(CYAN)$(BOLD)🎨 Checking formatting...$(RESET)\n"
	cargo fmt --check

lint-rust: ## Lint Rust (clippy)
	@printf "$(CYAN)$(BOLD)🦀 Linting Rust...$(RESET)\n"
	cargo clippy -- -D warnings

lint-md: ## Lint Markdown
	@printf "$(CYAN)$(BOLD)📝 Linting Markdown...$(RESET)\n"
	npx markdownlint-cli2 "**/*.md"

lint-yaml: ## Lint YAML
	@printf "$(CYAN)$(BOLD)📋 Linting YAML...$(RESET)\n"
	yamllint .

lint: lint-rust lint-md lint-yaml ## Run all linters

build: ## Build the project
	@printf "$(GREEN)$(BOLD)🔨 Building...$(RESET)\n"
	cargo build

test: ## Run tests
	@printf "$(GREEN)$(BOLD)🧪 Running tests...$(RESET)\n"
	cargo test

audit: ## Run security audit
	@printf "$(YELLOW)$(BOLD)🔒 Running security audit...$(RESET)\n"
	cargo audit

check: fmt-check lint build test audit ## Run all checks (mirrors CI)

fix-rust: ## Auto-fix Rust formatting and clippy lints
	@printf "$(CYAN)$(BOLD)🔧 Fixing Rust...$(RESET)\n"
	cargo fmt
	cargo clippy --fix --allow-dirty

fix-md: ## Auto-fix Markdown
	@printf "$(CYAN)$(BOLD)🔧 Fixing Markdown...$(RESET)\n"
	npx markdownlint-cli2 --fix "**/*.md"

fix: fix-rust fix-md ## Auto-fix all (yaml has no auto-fixer)
