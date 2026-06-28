# cpemu — a CP/M 2.2 emulator in Rust

**cpemu** is a from-scratch emulator that runs the classic
[CP/M 2.2](https://en.wikipedia.org/wiki/CP/M) operating system and the vintage
software written for it — language interpreters, compilers, word processors, and
games from the late 1970s and early 1980s.

It does this the authentic way: it emulates a Zilog **Z80** CPU and a small
virtual machine, then boots the **genuine** Digital Research CP/M 2.2 command
processor (CCP) and kernel (BDOS) — the same binaries that ran on real hardware —
rather than re-implementing the operating system in Rust.

> **Status: early development, but it boots.** The Z80 CPU core is complete and
> `zexdoc`-validated, and **the genuine CP/M 2.2 now boots to the `A>` prompt** on
> the virtual machine — `DIR` reads the directory and a real disk image is served
> through the HLE BIOS. So far this runs headless (driven by tests); an
> interactive terminal front end is the next step. See
> [Project status](#project-status) for exactly what works. If you are a
> developer, start with [`dev-docs/`](dev-docs/README.md).

---

## Table of contents

- [What is CP/M, and why emulate it?](#what-is-cpm-and-why-emulate-it)
- [Design in one paragraph](#design-in-one-paragraph)
- [Project status](#project-status)
- [Building and testing](#building-and-testing)
- [Repository layout](#repository-layout)
- [Roadmap](#roadmap)
- [For developers](#for-developers)
- [Licensing and third-party code](#licensing-and-third-party-code)

---

## What is CP/M, and why emulate it?

CP/M (Control Program for Microcomputers) was one of the first operating systems
that was *not* tied to a single computer manufacturer. Introduced in 1974, it ran
on almost any machine built around an Intel 8080 or Zilog Z80 processor with at
least 16 KB of RAM. For roughly a decade it was the dominant OS for serious
microcomputers, and a huge body of software was written for it: Microsoft BASIC,
Turbo Pascal, WordStar, dBASE, and games like Zork.

Emulating CP/M lets that software run again on a modern machine. There are two
broad ways to build such an emulator:

1. **Re-implement the OS.** Intercept the program's operating-system calls and
   service them with native (Rust) code. This is simpler and is what tools like
   RunCPM do, but the thing running is not really CP/M — it is an imitation of
   its interface.
2. **Emulate the hardware and run the real OS.** Emulate the CPU and enough of a
   machine that the genuine, unmodified CP/M binaries boot and run on top of it.
   This is more work but it is *actually CP/M*.

cpemu takes the second approach.

## Design in one paragraph

cpemu emulates a **Z80 CPU** (a superset of the 8080, so it runs 8080 software
too) and a small **virtual machine**. At startup an emulated boot loader reads
the genuine CP/M 2.2 CCP and BDOS off a disk image's system tracks — exactly as a
real machine's boot ROM would — and jumps into them. The one piece cpemu
provides itself is the **BIOS**, the small hardware-specific layer CP/M uses for
console and disk I/O. Rather than running a BIOS as Z80 code, cpemu uses a tiny
guest BIOS *stub* that traps into Rust ("high-level emulation"), so console
characters go to your terminal and disk sectors are served from `.dsk` image
files. The result is genuine CP/M running on emulated hardware, with the I/O
plumbed cleanly into the host. For the reasoning behind these choices, see
[`dev-docs/architecture.md`](dev-docs/architecture.md).

## Project status

cpemu is built in dependency order: a correct CPU first, then the machine, then
the OS layer. Here is the honest state of each piece.

| Component                         | Status         |
|-----------------------------------|----------------|
| Z80 CPU — 8080 base instructions  | [x] done, tested |
| Z80 CPU — Z80 extensions (CB/ED/DD/FD) | [x] done, tested |
| Z80 CPU — interrupts (IM 0/1/2)   | [x] done, tested |
| Z80 CPU — disassembler            | [x] done, tested |
| Z80 CPU — validated vs. exercisers (zexdoc) | [x] **passes** (zexall: 1 group short) |
| Virtual machine & memory          | [x] done, tested |
| HLE BIOS & boot                   | [x] boots to `A>` |
| Disk subsystem                    | [~] 8" disk read/write (images/dir tooling next) |
| Console & terminal                | [~] headless console (interactive terminal next) |
| Debugger / monitor (the frontend) | [ ] not started  |
| Boot CP/M to the `A>` prompt      | [x] headless (`DIR` works) |

**The interactive terminal is not wired up yet**, so you drive CP/M through tests
for now (the boot integration tests in `crates/cpemu/src/bios.rs` boot the real
CCP/BDOS and run `DIR`). An interactive front end and a monitor come next.

## Building and testing

cpemu is a standard Rust workspace. You need a recent stable Rust toolchain
(edition 2024; developed against Rust 1.95).

```bash
# Build everything
cargo build --workspace

# Run the test suite (this is the best way to see progress today)
cargo test --workspace

# Run the developer gate — formatting, lints, and tests — exactly as CI does
make check
```

`cargo run` currently prints only a version banner; it is a placeholder until the
emulator can boot.

## Repository layout

```
cpemu/
├── Cargo.toml            Workspace manifest
├── README.md             You are here
├── Makefile              Developer tasks (make check, fmt, lint, test)
├── crates/
│   ├── cpu/              cpemu-cpu: the Z80/8080 CPU core (a standalone library)
│   └── cpemu/            cpemu: the system layer and CLI binary
├── third-party/
│   └── cpm22/            Vendored genuine CP/M 2.2 CCP+BDOS source + license
└── dev-docs/             Developer documentation (start at dev-docs/README.md)
```

The CPU lives in its own crate on purpose: it is the load-bearing component, it
is validated in isolation against industry-standard test programs, and keeping it
dependency-light means it never accidentally couples to the rest of the emulator.
See [`dev-docs/architecture.md`](dev-docs/architecture.md) for why the workspace
is split this way.

## Roadmap

Work is tracked as a set of epics (see [`dev-docs/roadmap.md`](dev-docs/roadmap.md)
for the live breakdown and rationale):

1. **CPU core** — a correct Z80, proven against the standard exercisers. *(in progress)*
2. **Virtual machine & memory** — RAM, the I/O bus, and the BIOS-trap mechanism.
3. **HLE BIOS & boot** — load and run the genuine CCP+BDOS off a disk image.
4. **Disk subsystem** — read/write real `.dsk` images.
5. **Console & terminal** — your terminal as the CP/M console.
6. **Debugger / monitor** — the interactive frontend.
7. **Integration** — boot to `A>` and run real vintage software.

## For developers

If you want to understand or contribute to cpemu, read the developer docs:

- [`dev-docs/README.md`](dev-docs/README.md) — index and orientation
- [`dev-docs/architecture.md`](dev-docs/architecture.md) — the system design and the decisions behind it
- [`dev-docs/cpu-core.md`](dev-docs/cpu-core.md) — how the CPU core works internally
- [`dev-docs/development.md`](dev-docs/development.md) — workflow, tooling, and conventions
- [`dev-docs/roadmap.md`](dev-docs/roadmap.md) — the plan and current status

## Licensing and third-party code

The cpemu source is under the MIT license. The repository also vendors the
genuine Digital Research CP/M 2.2 CCP and BDOS source under
[`third-party/cpm22/`](third-party/cpm22/); CP/M and its derivatives are
redistributable under a grant from DRDOS Inc. (successor to Digital Research). See
[`third-party/cpm22/NOTICE.md`](third-party/cpm22/NOTICE.md) for provenance and
the full license text.
