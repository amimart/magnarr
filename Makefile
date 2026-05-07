.PHONY: fmt fmt-check lint lint-rust lint-md lint-yaml build test audit check fix

fmt:
	cargo fmt

fmt-check:
	cargo fmt --check

lint-rust:
	cargo clippy -- -D warnings

lint-md:
	npx markdownlint-cli2 "**/*.md"

lint-yaml:
	yamllint .

## Run all linters
lint: lint-rust lint-md lint-yaml

build:
	cargo build

test:
	cargo test

audit:
	cargo audit

## Run all checks (mirrors CI)
check: fmt-check lint build test audit

## Auto-fix formatting and clippy lints
fix:
	cargo fmt
	cargo clippy --fix --allow-dirty
