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

Legend: [x] done · [~] in progress · [ ] not started · [.] deferred

### [x] Scaffolding & CI — `cpemu-kfp`
The workspace, tooling, and groundwork.
- [x] Two-crate workspace skeleton (`cpemu-cpu` + `cpemu`)
- [x] Verify CP/M license + vendor genuine CCP+BDOS source (`third-party/cpm22/`)
- [x] Lint / format / test + GitHub Actions CI

### [~] Z80/8080 CPU core — `cpemu-yd5`
A correct Z80 (superset of the 8080), proven against the standard exercisers.
- [x] CPU scaffolding — registers, flags, `Bus`, observable `StepResult`, T-states
- [x] 8080 base instruction set (the full unprefixed opcode map, Z80 flag semantics)
- [x] Z80 extensions — `CB` (bit/rotate/shift), `ED` (block ops, 16-bit ADC/SBC,
  `LD I/R`, `RRD`/`RLD`), `DD`/`FD` (`IX`/`IY` indexed, incl. `DD CB`)
- [x] Interrupts — maskable IM 0/1/2 + NMI delivery, `IFF1`/`IFF2`, and the
  one-instruction `EI` shadow (the CPU mechanism; the machine drives it)
- [x] Disassembler (shared with the monitor)
- [x] Exerciser harness — **zexdoc passes** (every documented-instruction group,
  ~31s in release). `prelim` runs in CI; `zexall` is one group short (the
  `BIT n,(HL)` undocumented-flag case needs the WZ/MEMPTR register — a stretch
  goal, tracked separately).

### [x] Virtual machine & memory — `cpemu-g47`
- [x] 64 KB `Memory` and the memory bus
- [x] I/O port bus (open-bus) and the **PC-based BIOS-trap mechanism** — an
  `Executor` owns the CPU, `Machine`, and BIOS handler and traps when PC lands on
  a vector (`Return`/`Jump`/`Stop`); see
  [architecture.md](architecture.md#decision-1-genuine-ccpbdos-high-level-emulated-bios)
- [x] Machine-config seam (`MachineConfig`) — the memory map in one place, with a
  `virtual_default()` for our machine and room for a future real one

### [x] HLE BIOS & authentic boot — `cpemu-pal`
**The genuine CP/M 2.2 boots to `A>`.** (`crates/cpemu/src/bios.rs`)
- [x] Assemble + vendor genuine CCP/BDOS binaries (asl toolchain)
- [x] BIOS jump table + trap dispatch (all 17 vectors via `CpmBios`)
- [x] System-track boot loader (cold/warm boot; WBOOT reloads CCP; `C` = drive)
- [x] Console BIOS calls (CONST/CONIN/CONOUT/LIST/PUNCH/READER)
- [x] Disk BIOS calls (SELDSK→DPH/DPB, SETTRK/SETSEC/SETDMA/READ/WRITE/SECTRAN)

### [~] Disk subsystem — `cpemu-9br`
- [x] Virtual disk abstraction (8" SSSD geometry, sectors, `DPB_8INCH`)
- [ ] `.dsk` sector-image load/save (mount real images)
- [ ] CP/M directory + allocation tooling (place files on a disk)
- [ ] Drive mapping / mounting (multiple drives)

### [~] Console & terminal — `cpemu-7m4`
- [x] Console abstraction (`Console` trait; scripted console for headless/tests)
- [x] Interactive raw-TTY terminal front end (`TerminalConsole`; `cargo run`)
- [ ] Selectable ADM-3A/VT100 emulation

### [ ] Debugger / monitor — `cpemu-6o2`
The interactive frontend: breakpoints, single-step, register/memory inspect+edit,
disassembly, instruction trace. Depends on the CPU (and its disassembler).

### [ ] CLI & config — `cpemu-4ur`
Argument parsing, config file, disk mounts, machine selection, throttle controls.

### [.] Host-directory drives — `cpemu-ntn` (deferred)
Mapping a host directory to a CP/M drive. Deferred because, with a genuine BDOS,
it requires synthesizing a live CP/M file system — the project's single biggest
hidden cost. See
[architecture.md](architecture.md#decision-5-disk-model--real-images-now-host-directories-later).

### [ ] Integration & vintage-software validation — `cpemu-uvv`
Boot CP/M to the `A>` prompt, then run real software (MBASIC, Turbo Pascal, Zork,
WordStar) as acceptance tests.

## Deferred decisions

Decisions we consciously postponed rather than guessed at. Each is recorded on
its bead so it resurfaces at the right time.

- **CP/M binary form** — *resolved.* The CCP/BDOS are assembled from the vendored
  source at the memory-map origins (CCP `0xE400`, BDOS `0xEC00`) with Macro
  Assembler AS, and the resulting `ccp.bin`/`bdos.bin` are **committed** so CI and
  normal builds need no assembler; `make cpm-binaries` regenerates them
  reproducibly. See [`third-party/cpm22/NOTICE.md`](../third-party/cpm22/NOTICE.md).
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
