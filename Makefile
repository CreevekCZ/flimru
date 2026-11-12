CARGO ?= cargo
PYTHON ?= python3
AUDIT_VERSION := 0.22.2

.PHONY: check fmt lint test audit setup-tools build release release-draft packaging-test

check: fmt lint test audit packaging-test

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

# Both targets only build local assets; neither publishes a release.
release:
	$(PYTHON) scripts/prepare_release.py

release-draft:
	$(PYTHON) scripts/prepare_release.py --allow-dirty

packaging-test:
	$(PYTHON) -m unittest discover -s scripts -p 'test_*.py'
