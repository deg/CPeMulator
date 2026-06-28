# cpemu — a CP/M 2.2 emulator in Rust

**cpemu** is a from-scratch emulator that runs the classic
[CP/M 2.2](https://en.wikipedia.org/wiki/CP/M) operating system and the vintage
software written for it — language interpreters, compilers, word processors, and
games from the late 1970s and early 1980s.

It does this the authentic way: it emulates a Zilog **Z80** CPU and a small
virtual machine, then boots the **genuine** Digital Research CP/M 2.2 command
processor (CCP) and kernel (BDOS) — the same binaries that ran on real hardware —
rather than re-implementing the operating system in Rust.

> **Status: early development, but it boots — and you can type at it.** The Z80
> CPU core is complete and `zexdoc`-validated, and **the genuine CP/M 2.2 boots to
> an interactive `A>` prompt**: run `cargo run` and you get a live CP/M session in
> your terminal (try `DIR`; quit with Ctrl-\\). A real disk image is served
> through the HLE BIOS. Still missing: tools to put your own files/programs on the
> disk, and a debugger. See [Project status](#project-status) for exactly what
> works. If you are a developer, start with [`dev-docs/`](dev-docs/README.md).

---

## Table of contents

- [What is CP/M, and why emulate it?](#what-is-cpm-and-why-emulate-it)
- [Design in one paragraph](#design-in-one-paragraph)
- [Project status](#project-status)
- [Running classic CP/M software](#running-classic-cpm-software)
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
| Disk subsystem                    | [x] 8"/8 MB read/write, `.dsk` mount/save, multiple drives (A..P) |
| Console & terminal                | [x] interactive raw-TTY + ADM-3A emulation |
| Debugger / monitor                | [~] execution core (breakpoints/step/continue); inspect/disasm/UI next |
| Boot CP/M to the `A>` prompt      | [x] interactive (`cargo run`) |

**`cargo run` boots an interactive CP/M session** in your terminal. The boot
integration tests in `crates/cpemu/src/bios.rs` also drive the real CCP/BDOS
headless (booting and running `DIR`). You can load your own files (positional
args) and mount persistent `.dsk` images on several drives (`--disk`). The
debugger's execution core (breakpoints, single-step, continue) is in place; its
interactive frontend (register/memory inspect, disassembly, tracing) is next.

## Running classic CP/M software

The fun part. Two scripts take you from nothing to a fully-loaded CP/M machine:

```bash
./scripts/fetch-software.sh   # once: download a library of classic CP/M software
./scripts/load-all.sh         # boot CP/M with the whole library on drive A:
```

`fetch-software.sh` pulls a library of programs from the
[CP/M archive](http://www.retroarchive.org/cpm/) into a (gitignored) `software/`
directory and writes a `PROGRAMS.TXT` guide; `load-all.sh` copies them onto drive
A: and boots. The programs (~2.5 MB) fit comfortably on the default **8 MB** drive
(about one partition of a ~$10,000 1980 Corvus Winchester — the largest a single
CP/M 2.2 drive can be), leaving room to actually work. Non-program files that come
in the archives — scanned manuals, source, docs — are kept in `software/extras/`
for browsing but are **not** loaded onto the disk. Both scripts are forgiving:
the fetch tolerates the occasionally-flaky archive and reuses cached downloads.

Once you're at the `A>` prompt:

```text
A>TYPE PROGRAMS.TXT     a guide to everything on the disk
A>DIR                   list the files
A>ZORK1                 play Zork I  (then ZORK2, ZORK3)
A>ADVENTUR              Colossal Cave Adventure
A>SARGON                a game of chess
A>DBASE                 dBASE II  (QUIT to exit)
A>MBASIC STAR.BAS       Star Trek  (answer questions with a CAPITAL Y)
```

Included: the Zork trilogy and Enchanter, Colossal Cave Adventure, Sargon chess,
Ladder, Catchum, Microsoft BASIC, Turbo Pascal, muLISP, the BDS C compiler,
dBASE II, Multiplan, and the standard CP/M utilities (STAT, PIP, ED, DDT, ASM …),
plus a pile of BASIC games (`MBASIC <name>.BAS`).

You can also load specific files yourself — any arguments are copied onto drive
A: before boot:

```bash
cargo run -- software/ZORK1.COM software/ZORK1.DAT          # just Zork I
cargo run -- --mhz=2 software/LADDER.COM software/LADDER.DAT  # slow it down
```

### Options

Run `cargo run -- --help` for the full list. The most useful:

| Flag | Effect |
|------|--------|
| `--mhz=N` | CPU clock in MHz (default 4). Lower it (`--mhz=2`) if a game runs too fast. |
| `--fast` | Run unthrottled. |
| `--upper` | Fold typed input to UPPERCASE (for old programs that expect capitals). |
| `--no-map-delete` | Keep CP/M's authentic RUBOUT (Delete re-echoes the char). By default Delete is mapped to a clean Backspace. |
| `--raw` | Disable ADM-3A terminal translation. |
| `--floppy` | Use a small 8" floppy instead of the default 8 MB disk. |
| `--disk=[X:]PATH` | Mount a `.dsk` image as drive `X` (A..P; default A) and **save changes back to it on exit** — so your in-CP/M work persists. Repeatable for several drives. A new `PATH` starts a fresh disk; an existing image's size selects its format. |

Flags are forwarded by `load-all.sh` too, e.g. `./scripts/load-all.sh --mhz=2`.

With `--disk`, cpemu becomes a persistent, multi-drive machine:
`cargo run -- --disk=mine.dsk --disk=B:work.dsk` boots `mine.dsk` as A and
`work.dsk` as B, and anything you create inside CP/M (edit a file with `ED`, copy
across drives with `PIP B:=A:FILE`, write a program) is written back to the right
image when you quit. Saves are atomic (a temp file renamed over the image), so a
crash leaves the old image intact. Up to two 8 MB disks (or more smaller ones)
fit the BIOS work area.

### Keys

- **Ctrl-\\** — quit the emulator (always works, even from a stuck program).
- **Ctrl-C** — CP/M's own warm boot, back to `A>` (when the running program lets it).
- To leave a *program*, use its own command — MBASIC `SYSTEM`, dBASE `QUIT`, etc.

### Full-screen apps

The console translates **ADM-3A** terminal codes to ANSI, so full-screen programs
(Ladder, the Turbo Pascal editor, Multiplan) display correctly, while plain text
and modern `ESC[` sequences pass straight through. WordStar and SuperCalc load
but need their own terminal-install pass first (a future item).

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

To boot CP/M interactively, `cargo run` (quit with **Ctrl-\\**). See
[Running classic CP/M software](#running-classic-cpm-software) for the full
experience, and `cargo run -- --help` for every option.

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

The cpemu source is under the MIT license; see [`LICENSE`](LICENSE) for the
full text. The repository also vendors the
genuine Digital Research CP/M 2.2 CCP and BDOS source under
[`third-party/cpm22/`](third-party/cpm22/); CP/M and its derivatives are
redistributable under a grant from DRDOS Inc. (successor to Digital Research). See
[`third-party/cpm22/NOTICE.md`](third-party/cpm22/NOTICE.md) for provenance and
the full license text.
