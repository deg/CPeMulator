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

# Regenerate the genuine CP/M 2.2 CCP+BDOS binaries from the vendored source.
# Requires Macro Assembler AS (`asl` + `p2bin`) on PATH — see
# third-party/cpm22/NOTICE.md for how to build them. CI and normal builds do
# NOT need this: the binaries are committed. Origins must match the memory map
# in crates/cpemu/src/config.rs (CCP 0xE400, BDOS 0xEC00).
CPM := third-party/cpm22
.PHONY: cpm-binaries
cpm-binaries:
	cd $(CPM) && asl -D origin=0e400h -o ccp.p -L -OLIST ccp.lst ccp.asm
	cd $(CPM) && p2bin -l '$$00' -r '$$e400-$$ebff' ccp.p ccp.bin
	cd $(CPM) && asl -D origin=0ec00h -o bdos.p -L -OLIST bdos.lst bdos.asm
	cd $(CPM) && p2bin -l '$$00' -r '$$ec00-$$f9ff' bdos.p bdos.bin
	cd $(CPM) && rm -f ccp.p bdos.p ccp.lst bdos.lst
	@echo "Regenerated $(CPM)/ccp.bin (expect 2048) and bdos.bin (expect 3584):"
	@wc -c $(CPM)/ccp.bin $(CPM)/bdos.bin
