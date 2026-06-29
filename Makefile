.PHONY: help db up down logs test test-e2e build clean nuke fmt clippy check seed

help: ## Show this help
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | awk 'BEGIN {FS = ":.*?## "}; {printf "  \033[36m%-15s\033[0m %s\n", $$1, $$2}'

db: ## Start PostgreSQL via docker compose
	docker compose up -d postgres

up: ## Start all services
	docker compose up --build -d

down: ## Stop all services
	docker compose down -v

logs: ## Tail logs from all services
	docker compose logs -f

seed: ## Seed the database with sample data
	./scripts/seed-db.sh

test: ## Run unit + integration tests
	cargo test --workspace --lib

test-e2e: ## Run full E2E test suite (requires running services)
	./scripts/test-e2e.sh

build: ## Build all binaries in release mode
	cargo build --release --workspace

clean: ## Clean Rust build artifacts
	cargo clean

nuke: clean ## Full cleanup including Docker volumes
	docker compose down -v

fmt: ## Format all Rust code
	cargo fmt --all

clippy: ## Run clippy lints
	cargo clippy --workspace --all-targets -- -D warnings
	cargo clippy -p live-search --features ssr --all-targets -- -D warnings

check: ## Type-check all Rust code
	cargo check --workspace --all-targets
	cargo check -p live-search --features ssr --all-targets
	rustup target add wasm32-unknown-unknown
	cargo check -p live-search --target wasm32-unknown-unknown --features hydrate --lib
