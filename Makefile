# cpemu — developer tasks. Mirrors what CI enforces.

.PHONY: all check fmt fmt-check lint test build clean wordstar setup fetch run play help

all: check

# ---------------------------------------------------------------------------
# Running cpemu. Override these on the command line, e.g.
#   make play MHZ=2             full suite, CPU slowed to 2 MHz (action games)
#   make play FAST=1            full suite, unthrottled (host speed)
#   make run  ARGS=--arrows     bare CP/M, arrow keys -> WordStar diamond
#   make run  PROFILE=debug     unoptimized build (faster to compile, slower run)
PROFILE ?= release   # release (optimized) | debug (unoptimized)
MHZ     ?=           # CPU clock in MHz (cpemu's default is 4)
FAST    ?=           # any value -> run unthrottled, overriding MHZ
ARGS    ?=           # extra cpemu flags (see `cargo run -- --help`)
_RELEASE := $(if $(filter debug,$(PROFILE)),,--release)
_SPEED   := $(if $(FAST),--fast,$(if $(MHZ),--mhz=$(MHZ),))
_CPEMU   := $(_SPEED) $(ARGS)
_DBG     := $(if $(filter debug,$(PROFILE)),CPEMU_DEBUG=1,)
# ---------------------------------------------------------------------------

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

# One-time data prep from a bare repo: download the software library, then
# install WordStar for the ANSI terminal. (Needs network; retroarchive.org for
# most items, the Internet Archive for WordStar, bdsoft.com for BDS C.)
setup:
	./scripts/fetch-software.sh
	$(MAKE) wordstar

# Just download the software library into ./software/ (gitignored).
fetch:
	./scripts/fetch-software.sh

# Install the fetched WordStar for the ANSI/VT-100 terminal, in place. The
# downloaded edition ships uninstalled; this drives its own WINSTALL inside the
# emulator so its full-screen editor works. Run after scripts/fetch-software.sh.
wordstar:
	cargo run --release -p cpemu --example install-wordstar -- software

# Boot a bare interactive CP/M session (no extra software loaded).
run:
	cargo run $(_RELEASE) -p cpemu -- $(_CPEMU)

# Boot CP/M with the whole fetched software suite on drive A: (run `make setup`
# first). Honors PROFILE/MHZ/FAST/ARGS (see the variables above).
play:
	$(_DBG) ./scripts/load-all.sh $(_CPEMU)

help:
	@echo "Setup (once, from a bare repo):"
	@echo "  make setup                 download the library + install WordStar"
	@echo "Run:"
	@echo "  make play                  boot CP/M with the whole software suite"
	@echo "  make run                   boot a bare CP/M session"
	@echo "Options (override on the command line):"
	@echo "  MHZ=2   slow the CPU      FAST=1   unthrottled      PROFILE=debug"
	@echo "  ARGS=--arrows  (extra cpemu flags; see 'cargo run -- --help')"
	@echo "  e.g.  make play MHZ=2      make run ARGS=--arrows"
	@echo "Dev gate:  make check        (fmt-check + clippy + tests)"

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
