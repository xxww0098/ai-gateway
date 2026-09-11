.PHONY: build run clean fmt lint test test-ignored gates gen-secrets frontend

BINARY=ai-gateway

build:
	cargo build --release
	cp target/release/gw-server $(BINARY)

run: build
	./$(BINARY) --config config.yaml

frontend:
	cd frontend && npm ci && npm run build

clean:
	$(RM) $(BINARY)
	$(RM) -r frontend/dist
	cargo clean

fmt:
	cargo fmt

lint:
	cargo clippy --workspace --all-targets -- -D warnings

# Unit tests only. Anything needing a live Postgres/Redis is #[ignore]d so a
# bare `make test` stays green on a machine with neither (rule 2.9: fail loud
# or ignore, never silently skip).
test:
	cargo test --workspace

# The integration tier. Requires the services named in the failure message.
test-ignored:
	GW_TEST_DATABASE_URL=$${GW_TEST_DATABASE_URL} cargo test --workspace -- --ignored

# Machine-checked architecture rules (module reachability, file ownership,
# dependency direction, profile pairing). See CONTRACT.md.
gates:
	cargo xtask ci

# Generate production secrets (JWT signing + credential encryption).
gen-secrets:
	./scripts/gen-secrets.sh
