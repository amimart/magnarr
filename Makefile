.PHONY: lint lint-rust lint-rust-format lint-md lint-yaml lint-docker build build-rust build-docker test audit check fix fix-rust fix-md schema local-init local-start local-stop local-clean clean help

BOLD   := \033[1m
RESET  := \033[0m
CYAN   := \033[36m
GREEN  := \033[32m
YELLOW := \033[33m

TARGET_FOLDER       = target
DEPLOY_FOLDER       = $(TARGET_FOLDER)/deploy
LOCAL_DEPLOY_FOLDER = $(DEPLOY_FOLDER)/local

DOCKER_IMAGE ?= magnarr:local
HADOLINT_IMAGE ?= hadolint/hadolint:v2.12.0-alpine

help: ## Show available targets
	@printf "$(BOLD)Available targets:$(RESET)\n"
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' Makefile | awk 'BEGIN {FS = ":.*?## "}; {printf "  $(CYAN)$(BOLD)%-12s$(RESET) %s\n", $$1, $$2}'

schema: ## Generate schema.graphql from the GraphQL schema
	@printf "$(CYAN)$(BOLD)📄 Generating schema.graphql...$(RESET)\n"
	cargo run --bin export-schema

lint-rust: ## Lint Rust code with Clippy
	@printf "$(CYAN)$(BOLD)🦀 Linting Rust...$(RESET)\n"
	cargo clippy -- -D warnings

lint-rust-format: ## Lint Rust formatting
	@printf "$(CYAN)$(BOLD)🎨 Checking formatting...$(RESET)\n"
	cargo fmt --check

lint-md: ## Lint Markdown
	@printf "$(CYAN)$(BOLD)📝 Linting Markdown...$(RESET)\n"
	npx markdownlint-cli2 "**/*.md"

lint-yaml: ## Lint YAML
	@printf "$(CYAN)$(BOLD)📋 Linting YAML...$(RESET)\n"
	yamllint .

lint-docker: ## Lint Dockerfile
	@printf "$(CYAN)$(BOLD)🐳 Linting Dockerfile...$(RESET)\n"
	docker run --rm -i -v "$$(pwd):/workdir" -w /workdir $(HADOLINT_IMAGE) hadolint Dockerfile

lint: lint-rust lint-rust-format lint-md lint-yaml lint-docker ## Run all linters

build-rust: ## Build the magnarr binary
	@printf "$(GREEN)$(BOLD)🔨 Building...$(RESET)\n"
	cargo build

build-docker: ## Build the Docker image
	@printf "$(GREEN)$(BOLD)🐳 Building Docker image...$(RESET)\n"
	docker build --tag $(DOCKER_IMAGE) .

build: build-rust ## Build rust (i.e. not docker)

test: ## Run tests
	@printf "$(GREEN)$(BOLD)🧪 Running tests...$(RESET)\n"
	cargo test

audit: ## Run security audit
	@printf "$(YELLOW)$(BOLD)🔒 Running security audit...$(RESET)\n"
	cargo audit

check: lint build test audit ## Run all checks

fix-rust: ## Auto-fix Rust formatting and clippy lints
	@printf "$(CYAN)$(BOLD)🔧 Fixing Rust...$(RESET)\n"
	cargo fmt
	cargo clippy --fix --allow-dirty

fix-md: ## Auto-fix Markdown
	@printf "$(CYAN)$(BOLD)🔧 Fixing Markdown...$(RESET)\n"
	npx markdownlint-cli2 --fix "**/*.md"

fix: fix-rust fix-md ## Auto-fix all (yaml has no auto-fixer)

local-init: ## Initialize the local deployment environment
	@printf "$(GREEN)$(BOLD)🏭 Initializing local deployment...$(RESET)\n"
	mkdir -p $(LOCAL_DEPLOY_FOLDER)/magnarr/data
	mkdir $(LOCAL_DEPLOY_FOLDER)/qBittorrent
	mkdir $(LOCAL_DEPLOY_FOLDER)/downloads
	mkdir $(LOCAL_DEPLOY_FOLDER)/media
	cp docker/qBittorrent.conf $(LOCAL_DEPLOY_FOLDER)/qBittorrent/qBittorrent.conf
	cp docker/magnarr-config.yaml $(LOCAL_DEPLOY_FOLDER)/magnarr/config.yaml

local-start: ## Start the local deployment environment
	@printf "$(GREEN)$(BOLD)🚀 Starting local deployment...$(RESET)\n"
	docker compose up --build -d

local-stop: ## Stop the local deployment environment
	@printf "$(GREEN)$(BOLD)🛑 Stopping local deployment...$(RESET)\n"
	docker compose down

local-clean: local-stop ## Clean the local deployment environment
	@printf "$(GREEN)$(BOLD)🧹 Cleaning local deployment...$(RESET)\n"
	rm -rf $(LOCAL_DEPLOY_FOLDER)/*
	docker compose rm -f

clean: local-clean ## Clean all build artifacts and local deployment
	@printf "$(GREEN)$(BOLD)🧹 Cleaning all...$(RESET)\n"
	rm -rf $(TARGET_FOLDER)
