.PHONY: fmt fmt-check lint build test audit check fix

fmt:
	cargo fmt

fmt-check:
	cargo fmt --check

lint:
	cargo clippy -- -D warnings

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
