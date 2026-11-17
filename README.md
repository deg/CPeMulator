# CPeMulator — a CP/M 2.2 emulator in Rust

**CPeMulator** is a from-scratch emulator that runs the classic
[CP/M 2.2](https://en.wikipedia.org/wiki/CP/M) operating system and the vintage
software written for it — language interpreters, compilers, word processors, and
games from the late 1970s and early 1980s.

It does this the authentic way: it emulates a Zilog **Z80** CPU and a small
virtual machine, then boots the **genuine** Digital Research CP/M 2.2 command
processor (CCP) and kernel (BDOS) — the same binaries that ran on real hardware —
rather than re-implementing the operating system in Rust.

There are **two ways to run it**: in a web browser at
<https://cpemulator.degel.com/>, with nothing to install, or as a desktop program in
a terminal on your own computer. Both run the same emulator code and the same
genuine CP/M; see [Two ways to run it](#two-ways-to-run-it) for how they differ.

> **Status: it works.** The Z80 CPU core is complete and `zexdoc`-validated, and
> the **genuine, unmodified CP/M 2.2 boots to an interactive `A>` prompt** with a
> real disk image served through the HLE BIOS. You can run the classic software
> (Zork, Turbo Pascal, WordStar, MBASIC, dBASE …), mount persistent multi-drive
> disks and write files back to them, and break into a built-in debug monitor at
> any time (on the desktop). The fastest way in is
> **[CP/M in your browser](https://cpemulator.degel.com/)**, with nothing to install;
> on your own computer, it's `make play` (see [Quick start](#quick-start)).
> See [Project status](#project-status) for the full capability list and the few
> known limits. To work *on* CPeMulator, start with
> [`dev-docs/`](dev-docs/README.md).

---

## Table of contents

- [How this was built](#how-this-was-built)
- [Two ways to run it](#two-ways-to-run-it)
- [Try it in your browser](#try-it-in-your-browser)
- [Quick start](#quick-start)
- [What is CP/M, and why emulate it?](#what-is-cpm-and-why-emulate-it)
- [Design in one paragraph](#design-in-one-paragraph)
- [Project status](#project-status)
- [Running classic CP/M software](#running-classic-cpm-software)
- [Writing your own CP/M programs](#writing-your-own-cpm-programs)
- [Repository layout](#repository-layout)
- [For developers](#for-developers)
- [Licensing and third-party code](#licensing-and-third-party-code)

---

## How this was built

CPeMulator was written in a day, with [Claude](https://claude.com/claude-code),
as an experiment in "vibe coding" a non-trivial systems project: how far can a
human-plus-AI pair get on a real emulator in a single sitting? The bet was that a
*well-documented legacy system* is close to the ideal target for this. CP/M and
the Z80 are exhaustively specified — there are published instruction tables,
cycle counts, BDOS call conventions, decades-old manuals, and industry-standard
CPU test programs (`zexdoc`/`zexall`) that say objectively whether the core is
correct. That documentation is what makes the approach here — emulate the
hardware and boot the *genuine* CP/M binaries (see
[Design in one paragraph](#design-in-one-paragraph)) — tractable to build quickly
and verifiable as it goes, rather than a pile of plausible-looking guesses. The
result is a working machine that boots real CP/M and runs real vintage software;
the developer docs in [`dev-docs/`](dev-docs/README.md) record the design and the
reasoning behind it.

## Two ways to run it

| | In your browser | On your computer (desktop) |
|---|---|---|
| **Getting started** | Open <https://cpemulator.degel.com/> | Install Rust, then `make setup` and `make play` ([Quick start](#quick-start)) |
| **Software** | A fresh disk holds DRI's CP/M utilities; **Get software** fetches the classic library, which you then drop on the page | `make setup` downloads the classic library and installs it, ready to run |
| **Your files** | One 8 MB drive A:, kept in the browser; **Save disk** and **Load disk** move it as a `.dsk` file | `.dsk` files on your computer, several drives (A: to P:), written back when you quit |
| **Debug monitor** | Not yet | Yes: **Ctrl-\\** at any time |
| **Speed** | 2, 4, 8 or 25 MHz, or Max | Any clock (`--mhz=N`), or unthrottled (`--fast`) |
| **Keys** | Buttons for the control keys a browser keeps for itself (Ctrl-W, Ctrl-T, Ctrl-N) | Your terminal's own keyboard, plus `--arrows` for WordStar-style editors |

A disk saved in one opens in the other: the browser's **Save disk** writes a `.dsk`
that the desktop mounts with `--disk=`, and the desktop's images load in the browser.

## Try it in your browser

**<https://cpemulator.degel.com/>** runs the same emulator compiled to
WebAssembly, with nothing to install. It boots genuine CP/M 2.2 with Digital
Research's own utilities on drive A: (PIP, STAT, ED, ASM, LOAD, DDT, DUMP,
SUBMIT). Try `DIR`, `STAT`, or assemble DRI's sample program:
`ASM DUMP`, `LOAD DUMP`, `DUMP DUMP.COM`.

- **The software library:** **Get software** lists the desktop version's software
  (Zork, Turbo Pascal, WordStar, MBASIC, dBASE II, games, compilers). Click
  **Get all** (or pick programs), then drop the downloaded files on the page
  together: the page puts the right files on A:, installs WordStar for the
  screen, and marks each program **On disk**.
- **Your own programs:** **Your disk → Add files** (or drag files onto the page)
  puts `.COM` and data files on A:, or a `.zip` archive, which is unpacked onto the
  disk (old PKZIP 1.x archives too). Vintage software isn't hosted on the site,
  but anything you have can be added; see
  [Finding more software](#finding-more-software) for where to get it.
- **Keep your work with Save disk.** It saves drive A: as a `.dsk` file, which
  **Load disk** brings back and the desktop version mounts with `--disk=`. The
  disk is also kept in your browser between visits (nothing is sent anywhere),
  but browsers may clear their storage on their own, so the saved file is your
  real copy. The status line says when the disk has changed since you saved it.
- **Control keys the browser keeps for itself** (Ctrl-W, Ctrl-T, Ctrl-N on Windows
  and Linux, which WordStar and Turbo Pascal need): use the **Ctrl-W**,
  **Ctrl-T** and **Ctrl-N** buttons under the screen, or **Ctrl + …** then any key.
- **Speed** sets the emulated clock: 4 MHz by default, or **Max**.

The page's own help (below the screen) covers CP/M basics, wildcards and the
library. The desktop version adds a debug monitor, several drives and real files on
your computer; see [Two ways to run it](#two-ways-to-run-it). How the browser
build works is in [`dev-docs/browser.md`](dev-docs/browser.md).

## Quick start

To run it on your own computer (the desktop version), from a bare checkout to
running classic CP/M software in four steps:

```bash
# 1. Install a recent stable Rust toolchain (edition 2024; developed against
#    Rust 1.95). The easiest way is rustup: https://rustup.rs
#    Verify with:  cargo --version

# 2. Clone the repository
git clone https://github.com/deg/CPeMulator.git
cd CPeMulator

# 3. One-time data prep (needs network): downloads a library of classic CP/M
#    software and installs WordStar. Pulls from retroarchive.org, the Internet
#    Archive, and bdsoft.com; cached and tolerant of the occasionally-flaky
#    archive, so it's safe to re-run.
make setup

# 4. Boot CP/M with the whole library on drive A:
make play
```

You'll land at CP/M's `A>` prompt. Try `TYPE PROGRAMS.TXT` for a guided tour,
`DIR` to list the disk, or `ZORK1` to start playing. Break into the debug monitor
at any time with **Ctrl-\\** (`q` there quits). The rest of this README covers
[what's on the disk and how to run it](#running-classic-cpm-software); just want
a bare CP/M session with nothing loaded? Run `make run`.

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

CPeMulator takes the second approach.

## Design in one paragraph

CPeMulator emulates a **Z80 CPU** (a superset of the 8080, so it runs 8080 software
too) and a small **virtual machine**. At startup an emulated boot loader reads
the genuine CP/M 2.2 CCP and BDOS off a disk image's system tracks — exactly as a
real machine's boot ROM would — and jumps into them. The one piece CPeMulator
provides itself is the **BIOS**, the small hardware-specific layer CP/M uses for
console and disk I/O. Rather than running a BIOS as Z80 code, CPeMulator uses a tiny
guest BIOS *stub* that traps into Rust ("high-level emulation"), so console
characters go to your terminal (or the terminal on the web page) and disk sectors
are served from `.dsk` images (files on your computer, or a disk kept in the
browser). The result is genuine CP/M running on emulated hardware, with the I/O
plumbed cleanly into the host. The same code runs both versions: the browser's is
compiled to WebAssembly. For the reasoning behind these choices, see
[`dev-docs/architecture.md`](dev-docs/architecture.md).

## Project status

CPeMulator runs genuine CP/M 2.2 and the vintage software written for it. The core
pieces are built and tested:

- **Z80 CPU** — the full instruction set (8080 base plus the Z80 CB/ED/DD/FD
  prefixes), interrupts (IM 0/1/2), and a disassembler. Validated against the
  standard `zexdoc` exerciser, which it **passes**.
- **The machine** — RAM, the I/O bus, and the HLE-BIOS trap mechanism that boots
  the genuine Digital Research CCP and BDOS straight off a disk image.
- **Disk** — read/write `.dsk` images (8" floppy or 8 MB hard disk). On the
  desktop: multiple drives (A..P), with in-CP/M changes saved back atomically on
  exit. In the browser: drive A:, kept in the browser and saved as a `.dsk`.
- **Console** — your terminal as the CP/M console (or, in the browser, a terminal
  on the page), with TeleVideo TVI-950 → ANSI translation so full-screen programs
  display correctly.
- **Debug monitor** (desktop) — an interactive DDT-style debugger over the live
  Z80 (breakpoints, single-step/trace, disassembly, register/memory inspect+edit),
  reachable at any time with **Ctrl-\\**.
- **In a web browser** — the same emulator compiled to WebAssembly, with an
  xterm.js terminal, a software library, adding files, and the disk kept in the browser
  (see [Try it in your browser](#try-it-in-your-browser)), live at
  <https://cpemulator.degel.com/>.

**Known limitations.** `zexall` — the stricter exerciser that also checks the
undocumented flag bits — is one instruction group short: it needs the Z80's
internal WZ/MEMPTR register, which no real CP/M software depends on. Only one
machine profile exists today (`virtual`); the `--machine` flag is the seam for
future ones.

At startup a quick fixed-mix benchmark prints the host's raw Z80 throughput
(e.g. *"~1170 MHz, ~292× a 4 MHz Z80"*) — a reproducible figure, unlike the
workload-dependent `--fast` number. For the design reasoning and the live
roadmap, see [`dev-docs/`](dev-docs/README.md).

## Running classic CP/M software

The fun part. This section is about the **desktop version**: in the browser,
**Get software** offers the same library (see
[Try it in your browser](#try-it-in-your-browser)), and once a program is on the
disk it runs the same way in both. [Quick start](#quick-start) gets you to a
fully-loaded machine with `make setup` then `make play`; this section is what's
behind those two targets and everything you can do once you're there.

```bash
make setup    # once: download a library of classic CP/M software + install WordStar
make play     # boot CP/M with the whole library on drive A:
```

`make setup` runs `scripts/fetch-software.sh` — which pulls programs from the
[CP/M archive](http://www.retroarchive.org/cpm/) and the Internet Archive into a
(gitignored) `software/` directory and writes a `PROGRAMS.TXT` guide — then
`make wordstar`, which installs WordStar for the ANSI terminal. `make play`
(`scripts/load-all.sh`) copies everything onto drive A: and boots. The programs
(~2.5 MB) fit comfortably on the default **8 MB** drive (about one partition of a
~$10,000 1980 Corvus Winchester — the largest a single CP/M 2.2 drive can be),
leaving room to actually work. Non-program files from the archives — scanned
manuals, source, docs — are kept in `software/extras/` for browsing but are
**not** loaded onto the disk. The fetch tolerates the occasionally-flaky archive
and reuses cached downloads.

Tune how it runs by overriding `make` variables (`make help` lists them):

```bash
make play MHZ=2          # slow the CPU to 2 MHz (for fast action games)
make play FAST=1         # run unthrottled, at host speed
make play ARGS=--arrows  # map arrow keys to WordStar's cursor diamond (editors)
make run                 # a bare CP/M session, no extra software loaded
make run PROFILE=debug   # an unoptimized build (quicker to compile while hacking)
```

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

### Finding more software

The fetch script grabs a curated starter set, but there is far more out there.
To run anything else on the desktop, drop its `.COM` (and any data files) into
`software/` and re-run `make play`, or pass it on the command line. In the
browser, use **Your disk → Add files**, or drop the files (or a `.zip` of them) on
the page. Large, well-organized archives to dig through:

- [The *Humongous* CP/M Software Archives](http://www.classiccmp.org/cpmarchives/)
  — the Walnut Creek CP/M CD-ROM plus a huge collection of commercial software.
- [The Unofficial CP/M Web Site](http://www.cpm.z80.de/) — Digital Research's own
  languages, tools, source, and documentation, free and legal to download.
- [retroarchive.org/cpm](http://www.retroarchive.org/cpm/) — the commercial CP/M
  archive this project already fetches from, browsable by category.
- [The Internet Archive](https://archive.org/details/Humongous_CPM_Archive_Collection)
  — mirrored CP/M collections.

### Options

These are the desktop version's command-line flags (the browser has its Speed menu
and key buttons instead). Run `cargo run -- --help` for the full list. The most
useful:

| Flag | Effect |
|------|--------|
| `--mhz=N` | CPU clock in MHz (default 4). Lower it (`--mhz=2`) if a game runs too fast. On exit, the achieved clock is reported to confirm it held N MHz. |
| `--fast` | Run unthrottled; on exit, reports the equivalent clock for that run (e.g. *"ran unthrottled at ~995 MHz, equivalent to --mhz=995"*). Workload-dependent. |
| `--upper` | Fold typed input to UPPERCASE (for old programs that expect capitals). |
| `--no-map-delete` | Keep CP/M's authentic RUBOUT (Delete re-echoes the char). By default Delete is mapped to a clean Backspace. |
| `--arrows` | Map the arrow keys to WordStar's cursor diamond (Up=`^E` Down=`^X` Left=`^S` Right=`^D`), so they work in WordStar, the Turbo editor, and other WordStar-style editors. Off by default — those codes do other things at the `A>` prompt (`^S` pauses output, `^X` kills the line). |
| `--raw` | Disable TeleVideo/ADM-3A terminal translation. |
| `--floppy` | Use a small 8" floppy instead of the default 8 MB disk. |
| `--machine=NAME` | Select a machine profile (default `virtual`, the only one today; the seam for future machines). |
| `--disk=[X:]PATH` | Mount a `.dsk` image as drive `X` (A..P; default A) and **save changes back to it on exit** — so your in-CP/M work persists. Repeatable for several drives. A new `PATH` starts a fresh disk; an existing image's size selects its format. |
| `--config=PATH` | Read options from a config file before the command line (CLI flags override it). Defaults to `~/.config/cpemu/config`. One directive per line (`#` comments), each a flag without the `--` — e.g. `mhz=2`, `upper`, `disk=B:work.dsk`. |

Flags are forwarded by `load-all.sh` too, e.g. `./scripts/load-all.sh --mhz=2`.

With `--disk`, CPeMulator becomes a persistent, multi-drive machine:
`cargo run -- --disk=mine.dsk --disk=B:work.dsk` boots `mine.dsk` as A and
`work.dsk` as B, and anything you create inside CP/M (edit a file with `ED`, copy
across drives with `PIP B:=A:FILE`, write a program) is written back to the right
image when you quit. Saves are atomic (a temp file renamed over the image), so a
crash leaves the old image intact. Up to two 8 MB disks (or more smaller ones)
fit the BIOS work area.

### Keys

On the desktop (for the browser, see [Try it in your browser](#try-it-in-your-browser)):

- **Ctrl-\\** — break into the debug monitor (always works, even from a stuck program); `q` there quits the emulator, `g` resumes.
- **Ctrl-C** — CP/M's own warm boot, back to `A>` (when the running program lets it).
- To leave a *program*, use its own command — MBASIC `SYSTEM`, dBASE `QUIT`, etc.

### The debug monitor

The desktop version only, for now. Press **Ctrl-\\** at any time to pause CP/M
and drop into a built-in monitor
(a `-` prompt). It's a DDT-style machine-level debugger over the live Z80;
numbers are hexadecimal:

| Command | Does |
|---------|------|
| `r` | show registers (with decoded flags) |
| `d [addr] [n]` | disassemble `n` instructions (the `>` marks the PC) |
| `m [addr] [n]` | hex+ASCII memory dump |
| `e addr b b …` | write bytes to memory |
| `s [n]` | single-step `n` instructions, tracing each |
| `b [addr]` / `bc addr`\|`*` | set/list, or clear, breakpoints |
| `g` | resume CP/M | 
| `q` | quit the emulator |

Breakpoints set in the monitor fire during normal execution and drop you back
into it. Resuming with `g` returns to CP/M exactly where it was — the break
doesn't disturb the running session.

### Full-screen apps

Both versions translate **TeleVideo TVI-950** terminal codes to ANSI (plus the
ADM-3A's `^Z`/`^^` it shares lineage with), so full-screen programs (Ladder, the
Turbo Pascal editor, PACMAN95) display correctly, while plain text and modern
`ESC[` sequences pass straight through. The TVI-950 was one of the most common
CP/M-era terminals, and the software that ships here targets it — some of it
under the name "Microbee VDU", a TeleVideo-compatible screen. Software configured
for ANSI/VT100 needs no translation at all. `--raw` disables translation entirely.

## Writing your own CP/M programs

CPeMulator is a real CP/M machine, so you can develop *for* CP/M two ways.

**Inside the emulator**, the classic toolchains the fetch script installs run
as-is: the CP/M assembler (`ASM`) and `DDT` debugger, the BDS C compiler, Turbo
Pascal, and Microsoft BASIC. Write source with `ED` (or edit it on the host and
copy the file in), assemble or compile, and run — the whole early-1980s workflow,
end to end. On the desktop, mount a persistent disk with `--disk=work.dsk` so your
sources and binaries survive across sessions; in the browser, the disk is kept
between visits, and **Save disk** keeps a copy as a file.

**On the host**, modern cross-compilers target Z80/CP/M and emit `.COM` files you
can drop into `software/` and run (or add to the browser's disk with **Add
files**):

- [z88dk](https://z88dk.org/) — a C compiler, assembler, and linker for Z80
  machines that builds CP/M binaries directly (it wraps
  [SDCC](https://sdcc.sourceforge.net/) for code generation).
- [Macro Assembler AS](http://john.ccac.rwth-aachen.de:8000/as/) (`asl`) — the
  multi-target assembler this repo already uses to rebuild the vendored CP/M
  source; it assembles 8080/Z80 to a raw binary.

For the system-call interface, the canonical references — the *CP/M 2.2
Programmer's Guide* and the BDOS function list — are on
[the Unofficial CP/M Web Site](http://www.cpm.z80.de/), and the vendored CCP/BDOS
source under [`third-party/cpm22/`](third-party/cpm22/) is itself a worked
example of real CP/M code.

## Repository layout

```
CPeMulator/
├── Cargo.toml            Workspace manifest
├── README.md             You are here
├── Makefile              Tasks: make play / run (desktop), make web-serve (browser), make check
├── crates/
│   ├── cpu/              cpemu-cpu: the Z80/8080 CPU core (a standalone library)
│   ├── cpemu/            cpemu: the system layer and CLI binary
│   └── web/              cpemu-web: the WebAssembly wrapper for the browser build
├── web/                  The browser page (web/www), its build script and smoke test
├── third-party/
│   └── cpm22/            Vendored genuine CP/M 2.2 CCP+BDOS source + license,
│                         and DRI's CP/M 2.2 utilities (utilities/)
└── dev-docs/             Developer documentation (start at dev-docs/README.md)
```

The CPU lives in its own crate on purpose, for reasons explained in the
developer docs.

## For developers

This README is for *using* CPeMulator. Everything about *building on* it — the
system design and the reasoning behind it, the workspace split, how the CPU core
works inside, the day-to-day workflow, and the roadmap — lives in
[`dev-docs/`](dev-docs/README.md):

- [`dev-docs/README.md`](dev-docs/README.md) — index and orientation
- [`dev-docs/architecture.md`](dev-docs/architecture.md) — the system design and the decisions behind it
- [`dev-docs/cpu-core.md`](dev-docs/cpu-core.md) — how the CPU core works internally
- [`dev-docs/development.md`](dev-docs/development.md) — workflow, toolchain, `make` targets, testing, and conventions
- [`dev-docs/browser.md`](dev-docs/browser.md) — the browser (WebAssembly) build: how it works, building, deploying
- [`dev-docs/roadmap.md`](dev-docs/roadmap.md) — the plan, the epics, and current status

The short version for hacking: it's a standard Rust workspace —
`cargo build --workspace`, `cargo test --workspace`, and `make check` for the
full format-lint-test gate that CI enforces. For the browser version,
`make web-serve` builds and serves the page locally and `make web-smoke` tests it.

## Licensing and third-party code

The CPeMulator source is under the MIT license; see [`LICENSE`](LICENSE) for the
full text. The repository also vendors the
genuine Digital Research CP/M 2.2 CCP and BDOS source under
[`third-party/cpm22/`](third-party/cpm22/); CP/M and its derivatives are
redistributable under a grant from DRDOS Inc. (successor to Digital Research). See
[`third-party/cpm22/NOTICE.md`](third-party/cpm22/NOTICE.md) for provenance and
the full license text. DRI's CP/M 2.2 utility programs, which the browser build
puts on a fresh disk, are vendored under the same grant in
[`third-party/cpm22/utilities/`](third-party/cpm22/utilities/NOTICE.md). The
browser page also vendors [xterm.js](https://xtermjs.org/) (MIT) in
`web/www/vendor/xterm/`.
