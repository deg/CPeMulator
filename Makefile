# cpemu — developer tasks. Mirrors what CI enforces.

.PHONY: all check fmt fmt-check lint test build clean

all: check

# Full local gate — run before pushing. Matches the CI workflow.
check: fmt-check lint test

# Auto-format the workspace.
fmt:
	cargo fmt --all

# Verify formatting without modifying files (CI gate).
fmt-check:
	cargo fmt --all -- --check

# Clippy across all crates and targets; warnings are errors.
lint:
	cargo clippy --workspace --all-targets -- -D warnings

# Run the full test suite.
test:
	cargo test --workspace

# Build everything.
build:
	cargo build --workspace

clean:
	cargo clean
