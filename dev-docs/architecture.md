# Architecture

This document explains how cpemu is put together as a whole system and, more
importantly, *why* it is built this way. The individual pieces are documented
elsewhere ([cpu-core.md](cpu-core.md) for the CPU); this is the big picture.

## The goal that drives every decision

cpemu exists to **run real vintage CP/M software** — Microsoft BASIC, Turbo
Pascal, WordStar, Zork — and to do so **authentically**, by booting the genuine
Digital Research CP/M 2.2 and running it on emulated hardware. "Authentic" and
"runs real software" sometimes pull in different directions; where they do, this
document records which way we leaned and why.

## The layers of a CP/M system

To understand the design you have to understand how a real CP/M machine is
layered, because cpemu mirrors that layering. From the top down:

```
┌─────────────────────────────────────────────┐
│ Application (MBASIC, WordStar, your program) │  ← the software we want to run
├─────────────────────────────────────────────┤
│ CCP   — the command shell (the A> prompt)    │  ┐
├─────────────────────────────────────────────┤  │ genuine Digital Research
│ BDOS  — kernel: files, console, system calls │  ┘ binaries, unmodified
├─────────────────────────────────────────────┤
│ BIOS  — machine-specific console & disk I/O  │  ← cpemu provides this
├─────────────────────────────────────────────┤
│ Hardware — CPU, RAM, console, disk           │  ← cpemu emulates this
└─────────────────────────────────────────────┘
```

The crucial fact: **CCP and BDOS are identical across all CP/M machines, but the
BIOS is different on every machine.** Digital Research shipped CCP and BDOS; each
computer manufacturer wrote their own BIOS to match their hardware. That single
fact shapes most of cpemu's architecture.

## Decision 1: Genuine CCP+BDOS, high-level-emulated BIOS

cpemu runs the **real, unmodified** CCP and BDOS binaries. It does **not**
re-implement them in Rust. This is what makes it "really CP/M" rather than a
CP/M-compatible interface.

The BIOS is the exception. Instead of running a BIOS as Z80 code talking to
emulated I/O chips, cpemu installs a tiny guest BIOS **stub**: each BIOS entry
point is a few bytes that hand control to a Rust function (a "trap"). The Rust
side then does the real work — putting a character on your terminal, reading a
sector from a disk-image file.

Why high-level-emulate the BIOS specifically?

- **It is the natural seam.** BDOS already expects to call *through* the BIOS for
  all hardware access. Trapping at exactly those entry points is clean and
  historically faithful to the interface, even though the implementation is Rust.
- **It is where the host integration lives.** Console characters need to reach
  your terminal; disk sectors need to come from host files. That glue is far
  simpler in Rust than as emulated Z80 device drivers.
- **It avoids emulating peripheral chips** (UARTs, disk controllers) that add a
  lot of work for no benefit to our goal.

What we deliberately keep authentic: the *file-system semantics* live in the
genuine BDOS, not in our code. cpemu serves raw disk **sectors**; BDOS interprets
the CP/M directory and allocation structures itself, exactly as on real hardware.
This matters because subtle BDOS behaviors that vintage software depends on come
for free instead of having to be re-created.

**How the trap actually works (PC-based, not port-based).** A natural guess is
that the BIOS stub would do `OUT (port),A` and the machine's I/O handler would
service it. That does not work: a BIOS function needs the CPU's registers (for
example `CONOUT` emits the character in register `C`), but `OUT (n),A` conveys
only `A` to `Bus::output`, which has no access to the CPU — and giving the bus a
CPU reference is impossible, because the CPU already borrows the bus for the
duration of an instruction. So instead an **executor** owns the CPU, the machine,
and the BIOS handler as three separate objects, and checks the program counter
each step. When PC lands exactly on a BIOS vector address, the executor calls the
handler with full access to both the CPU and memory, then acts on the result:
`Return` (pop the return address — the common case), `Jump` (transfer control,
for the boot vectors that never return), or `Stop`. The guest never executes any
BIOS code; it just jumps to where the BIOS would live and Rust does the work.
This is implemented in `crates/cpemu/src/{machine,executor}.rs`; see
[development.md](development.md) and the module docs for details.

## Decision 2: Boot the way a real machine boots

cpemu does not simply copy CCP and BDOS into memory at startup. It emulates the
real cold-boot sequence: a small boot loader reads CCP and BDOS off the disk
image's reserved **system tracks** and jumps to them, just as a vintage machine's
boot ROM did. Warm boots (which CP/M performs after a program exits) reload the
CCP the same way.

Why bother, when a direct memory copy would be simpler? Two reasons:

- It is the authentic experience, which is an explicit project goal.
- It forces the disk-image layout to be real (system tracks included), which is
  what lets cpemu work with genuine disk images rather than a bespoke format.

The cost is that the disk-image format and the boot loader must be designed
together; this is handled in the boot epic.

## Decision 3: The CPU is a Z80, and it lives in its own crate

The CPU is a **Z80**, which is a strict superset of the Intel 8080. CP/M itself
was written for the 8080, but the majority of interesting CP/M software (Turbo
Pascal, many games) uses Z80-only instructions. Implementing the Z80 gets us both
for the price of one instruction set.

The CPU is also a **separate crate** (`crates/cpu`) from everything else
(`crates/cpemu`). This is a deliberate boundary, not just tidiness:

- **It can be validated in isolation.** The CPU's correctness is proven against
  industry-standard test programs (the "exercisers"; see below). A standalone
  crate guarantees those tests depend on nothing but the CPU.
- **The compiler enforces the layering.** The CPU cannot accidentally reach into
  the rest of the emulator, because it does not depend on it. The dependency
  arrow points one way only.
- **It stays dependency-light**, which keeps it fast to compile and easy to reason
  about.

We resisted splitting *everything* into separate crates. The rest of the system
(memory, BIOS, disk, console, monitor) lives as modules inside `crates/cpemu`
until a concrete need — like testing the disk subsystem against a fake CPU —
justifies promoting a module to its own crate. Premature crate-splitting adds
ceremony without benefit. See [cpu-core.md](cpu-core.md) for the crate's internals
and the `Bus` boundary it presents to the outside world.

## Decision 4: Correctness is proven, not assumed (the exercisers)

A CPU emulator that is 99% correct is useless: the 1% will corrupt a program in
some non-obvious way and you will chase the symptom forever. So the CPU's
definition of "done" is **passing the standard Z80/8080 test programs**, the same
ones the emulation community has used for decades.

- The v1 gate is **zexdoc**, which exhaustively checks every documented
  instruction's result and flags by comparing CRCs against known-good values.
- **zexall** additionally checks *undocumented* flag behavior (including the
  internal "MEMPTR"/WZ register). It is a known-hard milestone and is treated as
  a stretch goal, not a v1 requirement.
- A few simpler 8080-era programs (TST8080, 8080PRE, CPUTEST) serve as quick
  functional smoke tests during development.
- We specifically **do not** use 8080EXM as a gate: it asserts the 8080's flag
  semantics, which differ from the Z80's, so it would fail on a correct Z80 core.

This is why the CPU is built before anything else: nothing downstream can be
trusted until the CPU is.

## Decision 5: Disk model — real images now, host directories later

CP/M drives (A:, B:, …) will be backed by real `.dsk` **sector images** with
genuine CP/M directory and allocation structures, served sector-by-sector to the
genuine BDOS. This is the authentic model and what booting real images requires.

A tempting convenience feature — mapping a host directory directly to a CP/M
drive — was **deferred**. With a genuine BDOS, a live host-directory drive is not
a small feature: it requires continuously synthesizing a CP/M file system
(directory extents, an allocation bitmap, sector-level serving, and translating
writes back to host files) — effectively a miniature file system. It is the
single largest hidden cost in the project, so it waits until after the authentic
disk-image path works. (Tools like RunCPM make host-directory mapping look easy
precisely because they intercept at the BDOS level and never run a real BDOS;
cpemu's authenticity is what makes it harder.)

## Decision 6: Console — your terminal, with optional fidelity

CP/M programs expect to talk to a serial terminal. By default cpemu uses your
host terminal in raw mode and passes ANSI/VT100 escape sequences through, which
works well in any modern terminal. For screen-oriented applications that target
an *older* terminal, `terminal.rs` translates the **TeleVideo TVI-950** code set
(cursor addressing, the clear/erase forms, line insert/delete, half-intensity) —
plus the ADM-3A's `^Z`/`^^` — onto your host terminal's ANSI. The TVI-950 is what
the bundled full-screen software actually drives (some of it under the label
"Microbee VDU", a TeleVideo-compatible screen). `--raw` disables translation.

When a display bug only reproduces interactively (e.g. a game that seeds its RNG
from how long you wait before the first keypress, so it never reaches gameplay in
a batch run), set `CPEMU_TRACE_OUT=<path>` to tee every *raw* guest output byte
(before translation) to a file. Decoding that trace is how the TVI-950 code set
was pinned down — it shows exactly what the program emits, not what we assume.

## How a running system fits together (the eventual picture)

Once the pieces below the application exist, a session looks like this:

1. The **virtual machine** provides 64 KB of RAM, an I/O bus, and the mechanism
   by which the BIOS stub traps into Rust.
2. At startup the emulated **boot loader** reads CCP+BDOS off the mounted disk
   image's system tracks and jumps in.
3. **CCP** prints `A>` and reads a command. Disk and console access flow down
   through **BDOS** to the **BIOS stub**, which traps into Rust.
4. Rust services those traps: console characters go to your **terminal**; disk
   sectors come from the **disk subsystem** reading `.dsk` images.
5. The **CPU** drives all of this by executing the genuine Z80 code of CCP, BDOS,
   and whatever application you load — instruction by instruction.
6. A **monitor** can sit alongside, letting you breakpoint, single-step, and
   inspect the running machine.

The current code implements the CPU at the bottom of this stack; the layers above
are built in [roadmap](roadmap.md) order.

## Where decisions are recorded

Cross-cutting decisions live here. Decisions tied to a specific task are recorded
on that task in the beads issue tracker (`bd show <id>`), and durable project-wide
facts are stored with `bd remember`. When in doubt, prefer writing a decision down
to re-deriving it later.
