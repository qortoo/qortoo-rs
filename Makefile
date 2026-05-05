.PHONY: install
install:
	cargo install cargo-tarpaulin

.PHONY: lint
lint:
	cargo +nightly fmt --all --check
	cargo check --all-features --tests
	cargo clippy --workspace --all-targets --tests --all-features -- -D warnings

.PHONY: tarpaulin
tarpaulin:
	cargo tarpaulin -o html -o xml -o Lcov --tests --all-features --engine Llvm --fail-under 90 --output-dir ./coverage
	open coverage/tarpaulin-report.html

.PHONY: doc
doc:
	cargo doc --no-deps --open

# ── Observability stack (Prometheus / Grafana / Tempo / Loki / Pyroscope) ───────
.PHONY: obs-up
obs-up:
	docker compose -f qortoo-rs-docker/docker-compose.yml up -d

.PHONY: obs-down
obs-down:
	docker compose -f qortoo-rs-docker/docker-compose.yml down

.PHONY: obs-down-v
obs-down-v:
	docker compose -f qortoo-rs-docker/docker-compose.yml down -v

.PHONY: obs-logs
obs-logs:
	docker compose -f qortoo-rs-docker/docker-compose.yml logs -f
