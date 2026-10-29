# Homebrew's cargo ignores rust-toolchain.toml; prefer rustup's shim, which
# reads the pin (the same reason as aval's Makefile).
CARGO := $(shell command -v rustup >/dev/null 2>&1 && test -x "$(HOME)/.cargo/bin/cargo" && echo "$(HOME)/.cargo/bin/cargo" || echo cargo)

.PHONY: all check lint test build fmt

all: check

## Everything CI runs, in the order it runs it.
check: lint test

lint:
	$(CARGO) fmt --all --check
	$(CARGO) clippy --all-targets -- -D warnings

test:
	$(CARGO) test

build:
	$(CARGO) build --release

fmt:
	$(CARGO) fmt --all
