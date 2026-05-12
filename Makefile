.PHONY: all lint lint-rust lint-rust-format lint-md lint-yaml lint-docker build build-rust build-docker test test-rust audit check fix fix-rust fix-md schema local-init local-start local-stop local-clean clean help

# Constants:
TARGET_FOLDER       = target
DEPLOY_FOLDER       = $(TARGET_FOLDER)/deploy
LOCAL_DEPLOY_FOLDER = $(DEPLOY_FOLDER)/local

# Docker images:
DOCKER_IMAGE ?= magnarr:local
HADOLINT_IMAGE ?= hadolint/hadolint:v2.12.0-alpine

BOLD   := \033[1m
COLOR_RESET  = $(call get_color,sgr0,)
COLOR_CYAN   = $(call get_color,setaf,6)
COLOR_GREEN  = $(call get_color,setaf,2)
COLOR_YELLOW = $(call get_color,setaf,3)

# Some colors (if supported)
define get_color
$(shell tput -Txterm $(1) $(2) 2>/dev/null || echo "")
endef

all: help

## Generate:
schema: ## Generate schema.graphql from the GraphQL schema
	@printf "$(COLOR_CYAN)$(BOLD)📄 Generating schema.graphql...$(COLOR_RESET)\n"
	cargo run --bin export-schema

## Lint:
lint: lint-rust lint-rust-format lint-md lint-yaml lint-docker ## Run all linters

lint-rust: ## Lint Rust code with Clippy
	@printf "$(COLOR_CYAN)$(BOLD)🦀 Linting Rust...$(COLOR_RESET)\n"
	cargo clippy -- -D warnings

lint-rust-format: ## Lint Rust formatting
	@printf "$(COLOR_CYAN)$(BOLD)🎨 Checking formatting...$(COLOR_RESET)\n"
	cargo fmt --check

lint-md: ## Lint Markdown
	@printf "$(COLOR_CYAN)$(BOLD)📝 Linting Markdown...$(COLOR_RESET)\n"
	npx markdownlint-cli2 "**/*.md"

lint-yaml: ## Lint YAML
	@printf "$(COLOR_CYAN)$(BOLD)📋 Linting YAML...$(COLOR_RESET)\n"
	yamllint .

lint-docker: ## Lint Dockerfile
	@printf "$(COLOR_CYAN)$(BOLD)🐳 Linting Dockerfile...$(COLOR_RESET)\n"
	docker run --rm -i -v "$$(pwd):/workdir" -w /workdir $(HADOLINT_IMAGE) hadolint Dockerfile

## Build:
build: build-rust ## Build rust (i.e. not docker)

build-rust: ## Build the magnarr binary
	@printf "$(COLOR_GREEN)$(BOLD)🔨 Building...$(COLOR_RESET)\n"
	cargo build

build-docker: ## Build the Docker image
	@printf "$(COLOR_GREEN)$(BOLD)🐳 Building Docker image...$(COLOR_RESET)\n"
	docker build --tag $(DOCKER_IMAGE) .

## Test:
test: test-rust ## Run all tests

test-rust: ## Run Rust tests
	@printf "$(COLOR_GREEN)$(BOLD)🧪 Running tests...$(COLOR_RESET)\n"
	cargo test

## Checks:
check: lint build test audit ## Run all checks

audit: ## Run security audit
	@printf "$(COLOR_YELLOW)$(BOLD)🔒 Running security audit...$(COLOR_RESET)\n"
	cargo audit

## Fix:
fix: fix-rust fix-md ## Auto-fix all (yaml has no auto-fixer)

fix-rust: ## Auto-fix Rust formatting and clippy lints
	@printf "$(COLOR_CYAN)$(BOLD)🔧 Fixing Rust...$(COLOR_RESET)\n"
	cargo fmt
	cargo clippy --fix --allow-dirty

fix-md: ## Auto-fix Markdown
	@printf "$(COLOR_CYAN)$(BOLD)🔧 Fixing Markdown...$(COLOR_RESET)\n"
	npx markdownlint-cli2 --fix "**/*.md"

## Local deployment:
local-init: ## Initialize the local deployment environment
	@printf "$(COLOR_GREEN)$(BOLD)🏭 Initializing local deployment...$(COLOR_RESET)\n"
	mkdir -p $(LOCAL_DEPLOY_FOLDER)/magnarr/data
	mkdir $(LOCAL_DEPLOY_FOLDER)/qBittorrent
	mkdir $(LOCAL_DEPLOY_FOLDER)/downloads
	mkdir $(LOCAL_DEPLOY_FOLDER)/media
	cp docker/qBittorrent.conf $(LOCAL_DEPLOY_FOLDER)/qBittorrent/qBittorrent.conf
	cp docker/magnarr-config.yaml $(LOCAL_DEPLOY_FOLDER)/magnarr/config.yaml

local-start: ## Start the local deployment environment
	@printf "$(COLOR_GREEN)$(BOLD)🚀 Starting local deployment...$(COLOR_RESET)\n"
	docker compose up --build -d

local-stop: ## Stop the local deployment environment
	@printf "$(COLOR_GREEN)$(BOLD)🛑 Stopping local deployment...$(COLOR_RESET)\n"
	docker compose down

local-clean: local-stop ## Clean the local deployment environment
	@printf "$(COLOR_GREEN)$(BOLD)🧹 Cleaning local deployment...$(COLOR_RESET)\n"
	rm -rf $(LOCAL_DEPLOY_FOLDER)/*
	docker compose rm -f

## Clean:
clean: local-clean ## Clean all build artifacts and local deployment
	@printf "$(COLOR_GREEN)$(BOLD)🧹 Cleaning all...$(COLOR_RESET)\n"
	rm -rf $(TARGET_FOLDER)

## Help:
help: ## Show this help.
	@echo ''
	@echo 'Usage:'
	@echo '  ${COLOR_YELLOW}make${COLOR_RESET} ${COLOR_GREEN}<target>${COLOR_RESET}'
	@echo ''
	@echo 'Targets:'
	@$(foreach V,$(sort $(.VARIABLES)), \
		$(if $(filter-out environment% default automatic,$(origin $V)), \
			$(if $(filter TOOL_%,$V), \
				export $V="$($V)";))) \
	awk 'BEGIN {FS = ":.*?## "} { \
		if (/^[a-zA-Z_-]+:.*?##.*$$/) {printf "    ${COLOR_YELLOW}%-20s${COLOR_GREEN}%s${COLOR_RESET}\n", $$1, $$2} \
		else if (/^## .*$$/) {printf "  ${COLOR_CYAN}%s${COLOR_RESET}\n", substr($$1,4)} \
		}' $(MAKEFILE_LIST) | envsubst
