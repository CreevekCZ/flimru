CARGO ?= cargo
AUDIT_VERSION := 0.22.2

.PHONY: check fmt lint test audit setup-tools build

check: fmt lint test audit

fmt:
	$(CARGO) fmt --all -- --check

lint:
	$(CARGO) clippy --locked --all-targets --all-features -- -D warnings

test:
	$(CARGO) test --locked --all-features

audit:
	$(CARGO) audit --deny warnings

setup-tools:
	$(CARGO) install cargo-audit --version $(AUDIT_VERSION) --locked

build:
	$(CARGO) build --locked --release
