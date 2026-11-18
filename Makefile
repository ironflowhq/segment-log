# Shortcuts for the commands in CLAUDE.md and CI. `make check` is what CI runs on each platform.
.PHONY: check fmt lint test build musl release release-dry-run

check: lint test

fmt:
	cargo fmt --all

lint:
	cargo fmt --all --check
	cargo clippy --workspace --all-targets -- -D warnings

test:
	cargo test --workspace

build:
	cargo build --release

# The guest server binary, on Linux x86_64.
musl:
	rustup target add x86_64-unknown-linux-musl
	cargo build --release -p segment-log-server --target x86_64-unknown-linux-musl

# make release VERSION=0.1.0
release:
	scripts/release.sh $(VERSION)

release-dry-run:
	scripts/release.sh $(VERSION) --dry-run
