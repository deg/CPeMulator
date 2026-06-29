# CPeMulator developer documentation

This directory is the home for documentation aimed at people working *on* CPeMulator
(as opposed to the root [`README.md`](../README.md), which is aimed at people who
want to know what CPeMulator is and how to use it).

These documents favor **clarity and education over brevity**. The goal is that a
developer new to emulation — or new to this codebase — can read them and
understand not just *what* the code does but *why* it is built the way it is.
When you make a design decision worth remembering, write it down here.

## Contents

| Document | What it covers |
|----------|----------------|
| [architecture.md](architecture.md) | The whole-system design: what we emulate, the big architectural choices (genuine CP/M + HLE BIOS, authentic boot, disk model), and the reasoning and trade-offs behind each. Start here. |
| [cpu-core.md](cpu-core.md) | How the `cpemu-cpu` crate works inside: the register model, flag handling, the `Bus` trait, the structured opcode decode, and how a single instruction executes. |
| [development.md](development.md) | Day-to-day workflow: the toolchain, `make` targets, CI, the issue tracker (beads), commit conventions, and how to validate the CPU. |
| [roadmap.md](roadmap.md) | The plan: the epics, their dependency order, current status, and notes on decisions that were deliberately deferred. |

## How these docs are maintained

Documentation is updated **as part of the work that changes the system**, not as
a later cleanup pass. If you implement a feature, extend or correct the relevant
document in the same change. The roadmap's status table and the root README's
status table should always reflect reality.

## A note on terminology

A few terms recur throughout these docs and the code:

- **CCP** — Console Command Processor: CP/M's shell (the `A>` prompt).
- **BDOS** — Basic Disk Operating System: CP/M's kernel — file system, console
  services, and the system-call interface programs use.
- **BIOS** — Basic Input/Output System: the small, machine-specific layer that
  BDOS calls to actually talk to hardware (console, disk). This is the part each
  computer vendor customized, and the part CPeMulator provides itself.
- **HLE** — High-Level Emulation: implementing a component's *behavior* in the
  host language (Rust) instead of executing its original machine code. CPeMulator's
  BIOS is high-level-emulated; its CCP and BDOS are not.
- **FCB** — File Control Block: the data structure CP/M programs use to describe
  an open file.
- **T-state** — one clock cycle of the Z80. Instruction timing is measured in
  T-states.
