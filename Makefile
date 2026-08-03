SHELL := /bin/bash
PATH := $(HOME)/.cargo/bin:$(PATH)

.PHONY: dev build test lint fmt fmt-check clean frontend

## dev — launch the Tauri app (starts Vite dev server, then runs the Rust app)
dev:
	@echo "Starting Vite dev server on :1420 ..."
	@(cd app-frontend && npm run dev -- --port 1420 --strictPort) &
	@echo "Running emdash-app (wait for window) ..."
	cargo run -p emdash-app

## build — compile the workspace (requires app-frontend/dist for emdash-app)
build:
	@test -d app-frontend/dist || (echo "app-frontend/dist missing — run 'make frontend' first"; exit 1)
	cargo build

## frontend — install deps + build the web UI into app-frontend/dist
frontend:
	cd app-frontend && npm install && npm run build

## test — run all workspace tests (requires app-frontend/dist for emdash-app)
test:
	@test -d app-frontend/dist || (echo "app-frontend/dist missing — run 'make frontend' first"; exit 1)
	cargo test --workspace

## lint — clippy with warnings-as-errors (requires app-frontend/dist for emdash-app)
lint:
	@test -d app-frontend/dist || (echo "app-frontend/dist missing — run 'make frontend' first"; exit 1)
	cargo clippy --workspace --all-targets -- -D warnings

## fmt — format all Rust code
fmt:
	cargo fmt --all

## fmt-check — verify formatting (CI gate)
fmt-check:
	cargo fmt --all --check

## clean — remove build artifacts
clean:
	cargo clean
	rm -rf app-frontend/node_modules app-frontend/dist

## check — full merge gate (fmt + clippy + test)
check: fmt-check lint test
	@echo "✅ Merge gate passed"
