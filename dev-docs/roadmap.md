# Roadmap

This is the plan for building cpemu: the epics, the order they happen in and why,
their current status, and the decisions that were deliberately deferred. The
authoritative, live source is the beads issue tracker (`bd ready`, `bd show
<id>`); this document is the human-readable map and is kept in sync as work lands.

## Build order and why

cpemu is built strictly **bottom-up**, because each layer depends on the
correctness of the one below it (see the layer diagram in
[architecture.md](architecture.md#the-layers-of-a-cpm-system)):

```
Scaffolding ─▶ CPU core ─▶ Virtual machine ─▶ HLE BIOS & boot ─▶ Disk
                  │                                │                │
                  └────────▶ Monitor              Console ─────────┤
                                                                   ▼
                                              CLI & config ─▶ Integration
```

Nothing above the CPU can be trusted until the CPU is proven correct, so the CPU
comes first and is validated hard. The OS layer (BIOS, boot) needs the machine
(RAM, I/O bus, trap mechanism) under it. Integration — actually booting CP/M and
running software — is necessarily last.

## Epics and status

Legend: ✅ done · 🚧 in progress · ⬜ not started · ⏸️ deferred

### ✅ Scaffolding & CI — `cpemu-kfp`
The workspace, tooling, and groundwork.
- ✅ Two-crate workspace skeleton (`cpemu-cpu` + `cpemu`)
- ✅ Verify CP/M license + vendor genuine CCP+BDOS source (`third-party/cpm22/`)
- ✅ Lint / format / test + GitHub Actions CI

### 🚧 Z80/8080 CPU core — `cpemu-yd5`
A correct Z80 (superset of the 8080), proven against the standard exercisers.
- ✅ CPU scaffolding — registers, flags, `Bus`, observable `StepResult`, T-states
- ✅ 8080 base instruction set (the full unprefixed opcode map, Z80 flag semantics)
- 🚧 Z80 extensions — `CB` (bit/rotate/shift), `ED` (block ops, 16-bit ADC/SBC,
  `LD I/R`), `DD`/`FD` (`IX`/`IY` indexed). *The largest single task.*
- ⬜ Interrupts — IM 0/1/2, `IFF` handling, IRQ/NMI delivery
- ⬜ Disassembler (shared with the monitor)
- ⬜ Exerciser harness — the real correctness gate (**zexdoc**); smoke suites.
  Blocked until the instruction set and interrupts are in.

### ⬜ Virtual machine & memory — `cpemu-g47`
- 64 KB RAM and the memory bus
- I/O port bus and the **BIOS-trap mechanism** the HLE BIOS stub uses
- A machine-config seam so a real historical machine could be added later

### ⬜ HLE BIOS & authentic boot — `cpemu-pal`
- BIOS jump table + trap dispatch
- System-track boot loader (cold/warm boot)
- Console BIOS calls (CONST/CONIN/CONOUT/LIST/PUNCH/READER)
- Disk BIOS calls (SELDSK/SETTRK/SETSEC/SETDMA/READ/WRITE/SECTRAN)

### ⬜ Disk subsystem — `cpemu-9br`
- Virtual disk abstraction (Disk Parameter Block, sector skew)
- `.dsk` sector-image read/write
- CP/M directory + allocation handling
- Drive mapping / mounting

### ⬜ Console & terminal — `cpemu-7m4`
- Raw TTY + ANSI passthrough
- Selectable ADM-3A/VT100 emulation

### ⬜ Debugger / monitor — `cpemu-6o2`
The interactive frontend: breakpoints, single-step, register/memory inspect+edit,
disassembly, instruction trace. Depends on the CPU (and its disassembler).

### ⬜ CLI & config — `cpemu-4ur`
Argument parsing, config file, disk mounts, machine selection, throttle controls.

### ⏸️ Host-directory drives — `cpemu-ntn` (deferred)
Mapping a host directory to a CP/M drive. Deferred because, with a genuine BDOS,
it requires synthesizing a live CP/M file system — the project's single biggest
hidden cost. See
[architecture.md](architecture.md#decision-5-disk-model--real-images-now-host-directories-later).

### ⬜ Integration & vintage-software validation — `cpemu-uvv`
Boot CP/M to the `A>` prompt, then run real software (MBASIC, Turbo Pascal, Zork,
WordStar) as acceptance tests.

## Deferred decisions

Decisions we consciously postponed rather than guessed at. Each is recorded on
its bead so it resurfaces at the right time.

- **CP/M binary form and memory origin** (boot epic `cpemu-pal`). We vendored the
  CCP/BDOS *source*, not a prebuilt binary, because the assembly origin depends on
  the virtual machine's memory map — which is not designed until the boot epic.
  When that map is fixed we will choose the origin, assemble (adding Macro
  Assembler AS to the toolchain), and decide whether to also commit a prebuilt
  binary. See [`third-party/cpm22/NOTICE.md`](../third-party/cpm22/NOTICE.md).
- **Host-directory drive strategy** (epic `cpemu-ntn`). When we do build it,
  whether to do live two-way file-system synthesis or the cheaper
  import-at-mount / export-at-unmount approach is itself an open choice.
- **Real-machine target.** The virtual machine is primary; whether/which
  historical machine (Altair, Kaypro, RC2014) to add behind the machine-config
  seam is left open.

## How "done" is defined for the hard parts

- **CPU**: passes **zexdoc**. zexall (undocumented flags + WZ/MEMPTR) is a stretch
  goal, not a v1 requirement. See
  [architecture.md](architecture.md#decision-4-correctness-is-proven-not-assumed-the-exercisers).
- **Boot**: reaches the genuine CCP `A>` prompt from a disk image via the
  system-track boot path.
- **Integration**: real vintage applications run.
