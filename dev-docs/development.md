# Development workflow

This document covers the practical side of working on CPeMulator: the toolchain, how
to build and check your work, how tasks are tracked, and the conventions the
project follows.

## Toolchain

- **Rust**, edition 2024 (developed against Rust 1.95). A recent stable toolchain
  with the `rustfmt` and `clippy` components is all you need.
- **GNU Make**, for the convenience targets in the `Makefile`.
- The **`wasm32-unknown-unknown`** target (`rustup target add
  wasm32-unknown-unknown`), because `make check` also confirms the emulator
  library still compiles for the browser.

No other system dependencies are required to build and test the current code. (A
Z80 assembler — Macro Assembler AS — will be needed later, when the boot epic
assembles the vendored CP/M source; it is not needed yet.)

## The developer gate: `make check`

One command runs everything CI runs, in the same order:

```bash
make check
```

which is:

```bash
cargo fmt --all -- --check      # formatting must be clean
cargo clippy --workspace --all-targets -- -D warnings   # no clippy warnings
cargo test --workspace          # all tests pass
cargo check -p cpemu --lib --target wasm32-unknown-unknown   # still builds for the browser
```

The last step exists because the browser build (WebAssembly) shares the emulator
library with the CLI. Native-only code (the host-terminal console with its
`crossterm` raw mode and reader thread, and the speed throttle, which uses
`std::time::Instant` and `thread::sleep`) is compiled only for native targets
(`#[cfg(not(target_arch = "wasm32"))]`). The check fails if something native-only
slips into the shared library. It only proves the library *compiles*: some `std`
calls compile for wasm32 and then panic when run (the clock, sleeping, threads),
so keep them out of shared code even when the check passes.

Run it before every commit. If `make check` is green, CI will be green. The other
targets are conveniences:

| Target | Does |
|--------|------|
| `make fmt` | Auto-format the workspace (fixes what `fmt-check` complains about). |
| `make lint` | Just the clippy step. |
| `make test` | Just the tests. |
| `make build` | `cargo build --workspace`. |
| `make wasm-check` | Just the browser-build compile check. |
| `make check` | The full gate (fmt-check + lint + test + wasm-check). The default target. |

### Lints are errors

Clippy warnings are treated as errors (`-D warnings`) in both `make check` and
CI. Workspace-wide lint configuration lives in the root `Cargo.toml` under
`[workspace.lints]`, and each crate opts in with `[lints] workspace = true`. If a
lint is genuinely wrong for a specific spot, allow it narrowly and locally with a
comment explaining why — do not loosen the workspace policy.

## Continuous integration

Every push and pull request runs the `.github/workflows/ci.yml` workflow, which
installs a stable toolchain and the wasm32 target and runs the same
fmt/clippy/test/wasm-check gate as `make check`, with cargo caching. CI runs on
GitHub Actions.

## Issue tracking: beads (`bd`)

Task tracking uses **beads** (`bd`), not markdown TODO lists or any other tool.
This is a hard project rule. The essentials:

```bash
bd ready              # what's available to work on (nothing blocking it)
bd show <id>          # full detail of an issue, including design notes & deps
bd update <id> --claim   # claim a task before starting it
bd close <id> --reason="..."   # close it when done, with a reason
bd remember "insight"     # store a durable, project-wide fact
bd memories <keyword>     # search those facts
```

Work is organized as **epics** (large components) with **first-layer children**
(concrete tasks), wired with dependencies so `bd ready` surfaces the right next
thing. The breakdown and its rationale are in [roadmap.md](roadmap.md).

A few conventions specific to this project:

- **Create or claim the task before writing code**, so the work is always tracked.
- **Record decisions where they belong.** A decision tied to a task goes on that
  task (`bd update <id> --append-notes=...` or `--design`). A durable project-wide
  fact goes in `bd remember`. A cross-cutting design rationale goes in
  [`dev-docs/`](README.md). The aim is that nothing important has to be
  re-derived later.
- Avoid `bd edit` (it opens an interactive editor).

## Git and commit conventions

- **Commit often: one logical change per commit.** Each task (or coherent sub-part
  of one) is its own commit. This keeps history readable and reviewable.
- Reference the bead in the commit subject, e.g. `(cpemu-yd5.2)`.
- Write commit *bodies* that explain what changed and why — the same clarity-over-
  brevity standard as the docs. A reader six months from now should understand the
  change from the message alone.
- The default branch is `main`, hosted privately at `github.com/deg/CPeMulator`.

The beads issue database also syncs to the git remote (via Dolt) so issue state
travels with the repository.

## Documentation is part of the work

Per a standing project instruction, the root `README.md` and this `dev-docs/`
directory are maintained **as the code changes**, not in a later pass. When you
implement or change something:

- Update the relevant `dev-docs/` document.
- Update the status tables in [roadmap.md](roadmap.md) and the root `README.md` if
  the state of a component changed.
- Favor **clarity and education over conciseness** — explain the *why*, not just
  the *what*.

## Validating the CPU

The CPU's real correctness bar is the standard exerciser programs, run by the
harness in `crates/cpu/tests/exercisers.rs`. It stands up just enough of a CP/M
environment (a 64K RAM bus, a trapped `CALL 5` BDOS for console output, warm-boot
completion detection) to run vintage `.COM` test programs against the CPU in
isolation.

- **`prelim`** — Frank Cringle's preliminary test. Small and fast; it runs as a
  normal test in `make check` and CI.
- **`zexdoc`** — the documented-instruction exerciser and the **v1 gate**. It
  runs billions of T-states (~31s in release), so it is `#[ignore]`d. Run it with:
  ```bash
  cargo test -p cpemu-cpu --release --test exercisers -- --ignored --nocapture
  ```
  It currently **passes** every group.
- **`zexall`** — the undocumented-behavior exerciser (stretch goal). Same command.
  It passes every group except `BIT n,(HL)`, which needs the WZ/MEMPTR register.

Below the exercisers, day-to-day development relies on fast, focused tests:

- **Unit tests in `alu.rs`** for flag behavior, tested in isolation.
- **Whole-program tests** in each CPU module that load a short machine-code
  program into a `PlainBus`, run it to `HALT`, and assert the resulting state.

When adding instructions, add tests at both levels (see
[cpu-core.md](cpu-core.md#how-to-extend-the-instruction-set)). The exercisers are
the final gate, but fast, focused tests are what make day-to-day development sane.
