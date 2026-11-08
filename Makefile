.PHONY: help secrets up down logs dev-api dev-web test test-backend test-frontend test-contracts eval lint smoke k8s-dev

help: ## Show targets
	@grep -E '^[a-zA-Z_-]+:.*?## ' $(MAKEFILE_LIST) | awk 'BEGIN {FS = ":.*?## "}; {printf "  %-16s %s\n", $$1, $$2}'

secrets: ## Create .env with generated secrets
	sh scripts/gen-secrets.sh

up: secrets ## Build and start the full stack (http://localhost:8080)
	docker compose up --build -d
	@echo "Open http://localhost:8080"

down: ## Stop the stack
	docker compose down

logs: ## Follow API logs
	docker compose logs -f api

dev-api: ## Run the API locally (SQLite, no Redis/chain needed)
	cd backend && cargo run -p claim-api

dev-web: ## Run the frontend dev server (proxies /api to :8000)
	cd frontend && npm run dev

test: test-backend test-frontend ## Run all tests

test-backend: ## Rust fmt, clippy, tests
	cd backend && cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace

test-frontend: ## Lint, typecheck, unit tests
	cd frontend && npm run lint && npm run typecheck && npm test

test-contracts: ## Foundry tests
	cd blockchain && forge test -vv

eval: ## Run the evaluation gate over the bundled cases
	cd backend && cargo run -q -p claim-engine --bin claim-eval

smoke: ## End-to-end smoke test against a running stack
	sh scripts/smoke.sh http://localhost:8080

k8s-dev: ## Render the dev Kubernetes overlay
	kubectl kustomize infra/k8s/overlays/dev
