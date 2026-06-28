# The CPU core (`cpemu-cpu`)

This document explains how the Z80/8080 CPU core works internally — enough that
you can read the code, extend the instruction set, or debug a failing test with
confidence. It assumes you know roughly what a CPU does (fetch an instruction,
execute it, repeat) but not the specifics of the Z80.

The crate lives in [`crates/cpu/`](../crates/cpu/) and is intentionally a
standalone library with no dependencies. The reasons for that isolation are in
[architecture.md](architecture.md#decision-3-the-cpu-is-a-z80-and-it-lives-in-its-own-crate);
this document is about what is *inside* it.

## File map

| File | Responsibility |
|------|----------------|
| `lib.rs` | Crate root; declares the modules and re-exports the public API (`Cpu`, `Bus`, `PlainBus`, `Registers`, `flag`, `StepResult`, `Stop`). |
| `registers.rs` | The register file: the 8-bit registers, 16-bit pairs, the shadow set, and flag bit definitions and helpers. |
| `bus.rs` | The `Bus` trait (the CPU's only window to memory and I/O) and `PlainBus`, a simple 64 KB implementation for tests. |
| `alu.rs` | Arithmetic/logic operations and — the hard part — their exact flag effects. |
| `cpu.rs` | The `Cpu` state and the fetch/decode/execute step, including the structured opcode decode. |

## The register model

The Z80's registers are modeled as individual fields in `Registers`
(`registers.rs`):

- Eight 8-bit registers: `a f b c d e h l`. `f` is the flags register.
- A **shadow set** (`a_alt f_alt b_alt …`) reached with the `EX AF,AF'` and `EXX`
  instructions, which swap the main and shadow registers wholesale.
- 16-bit registers: `ix iy sp pc`.
- `i` (interrupt vector) and `r` (memory refresh).

The 8-bit registers combine into 16-bit **pairs** — `AF`, `BC`, `DE`, `HL` — via
accessor methods (`bc()`, `set_bc(v)`, …) rather than being stored as 16-bit
values. Storing the 8-bit halves and computing the pair on demand keeps the very
common 8-bit register access trivial, which is the right trade for a Z80.

### The flags register

`f` is a raw `u8`. Each status flag is a single bit, defined as a mask in the
`flag` module:

| Bit | Mask  | Name | Meaning |
|-----|-------|------|---------|
| 7   | `S`   | Sign | high bit of the result |
| 6   | `Z`   | Zero | result was zero |
| 5   | `Y`   | (undocumented) | copy of bit 5 of the result |
| 4   | `H`   | Half-carry | carry out of bit 3 (used by `DAA`) |
| 3   | `X`   | (undocumented) | copy of bit 3 of the result |
| 2   | `PV`  | Parity/Overflow | parity (logic ops) or signed overflow (arithmetic) |
| 1   | `N`   | Add/Subtract | set after a subtraction (used by `DAA`) |
| 0   | `C`   | Carry | carry/borrow out of the operation |

Two things worth internalizing:

- **`Y` and `X` are real and we maintain them from the start.** They are
  "undocumented" only in the sense that Zilog never specified them, but they have
  consistent behavior (they copy bits 5 and 3 of the result) and the zexall test
  depends on them. They are cheap to set, so we always do.
- **`PV` is overloaded.** For logical operations it means *parity* (is the number
  of set bits even?); for arithmetic it means *signed overflow*. The ALU sets the
  right meaning per operation.

We deliberately use a plain `u8` for flags rather than a "lazy flags" scheme
(where you store operands and compute flags only when read). Lazy flags buy speed
we do not need and cost correctness we do; an emulator's flags must be exactly
right, and the straightforward representation is the one least likely to be wrong.

## The `Bus`: the CPU's window to the world

The CPU never touches memory or I/O directly. It goes through the `Bus` trait
(`bus.rs`):

```rust
pub trait Bus {
    fn read(&mut self, addr: u16) -> u8;
    fn write(&mut self, addr: u16, val: u8);
    fn input(&mut self, port: u16) -> u8;
    fn output(&mut self, port: u16, val: u8);
    // plus default little-endian read16/write16 helpers
}
```

A few design points:

- **The CPU is generic over the bus** (`step(&mut self, bus: &mut impl Bus)`),
  not handed a `dyn Bus`. There are only ever a couple of implementations (the
  real machine and the test harness), so monomorphization keeps the hot loop fast
  with no dynamic-dispatch cost.
- **Ports are `u16`, not `u8`.** This is a classic emulator bug to avoid: the Z80
  puts the `B` register on the high byte of the address bus during `IN r,(C)` /
  `OUT (C),r`, so a port number that is truncated to 8 bits silently loses
  information. We carry the full 16 bits everywhere.
- **`PlainBus`** is a trivial implementation backing memory and I/O with flat
  64 KB arrays. Tests (and, later, the CPU exerciser harness) load a program into
  a `PlainBus` and watch the CPU run.

This trait is the boundary between the CPU crate and the rest of the emulator:
the real machine will implement `Bus` such that reads/writes hit emulated RAM and
the BIOS-trap mechanism, but the CPU neither knows nor cares.

## Executing one instruction: `Cpu::step`

`Cpu::step` executes a single instruction and returns a `StepResult`:

```rust
pub struct StepResult {
    pub cycles: u32,   // T-states this instruction took
    pub stop: Stop,    // how it ended
}

pub enum Stop {
    Normal,            // ordinary instruction
    Halted,            // executed HALT; CPU now idles until an interrupt
    Illegal(u8),       // opcode not implemented yet (the byte is reported)
}
```

Returning *why* a step ended — not just how many cycles it took — is a
deliberate interface choice. The thing that drives the CPU (a run loop, the
exerciser harness, the future monitor) must be able to observe control flow:

- It needs to see `Halted`, or it would spin forever re-executing a `HALT`.
- It needs `Illegal(opcode)` to be **observable, never a panic**. During
  development the instruction set is incomplete; when the harness hits an
  unimplemented opcode it must be able to report *which* one and where, which is
  exactly when a panic would destroy the information you need.

### The fetch/decode/execute flow

1. **Halt check.** If the CPU is halted, it burns 4 T-states (an internal NOP),
   ticks the refresh register, and returns `Halted`. An interrupt (later) clears
   the halt.
2. **Fetch.** `fetch()` reads the opcode byte at `PC`, advances `PC`, and
   increments the `R` register. This is an "M1" (opcode-fetch) cycle.
   - Subtlety: **only opcode fetches tick `R`.** Operand bytes are read with
     `next_byte`/`next_word`, which advance `PC` but do *not* touch `R`. Getting
     this wrong is invisible until a program reads `R` and gets the wrong value.
   - Subtlety: `R` is a 7-bit counter — incrementing preserves bit 7. `inc_r`
     handles this.
3. **Decode.** The opcode byte is split into bit-fields and dispatched (below).
4. **Execute.** The matched handler does the work, then calls `done(cycles, stop)`
   which accumulates the cycle count and builds the `StepResult`.

## The structured decode

A naïve CPU emulator is a `match` with ~250 arms, one per opcode. That is
error-prone to write and to read. Instead cpemu uses the **standard Z80 opcode
structure** (popularized by Cristian Dinu's "Decoding Z80 Opcodes"). Every opcode
byte decomposes into fields:

```
  bit:  7 6 5 4 3 2 1 0
        └x┘ └─y─┘ └─z─┘
        x = bits 7-6   (0..3)   the top-level group
        y = bits 5-3   (0..7)   often a register or condition index
        z = bits 2-0   (0..7)   often a register index
        p = y >> 1     (0..3)   a register-pair index
        q = y & 1      (0..1)   a sub-selector
```

The whole unprefixed instruction set falls out of these fields plus a few small
lookup tables:

- `r[z]` selects an 8-bit register: `B C D E H L (HL) A`. Index 6 is the special
  "memory at `HL`" pseudo-register, handled by `reg_get`/`reg_set`.
- `rp[p]` selects a 16-bit pair `BC DE HL SP` (for most 16-bit ops); `rp2[p]`
  selects `BC DE HL AF` (for `PUSH`/`POP`).
- `alu[y]` selects an arithmetic/logic operation: `ADD ADC SUB SBC AND XOR OR CP`.
- `cc[y]` selects a branch condition: `NZ Z NC C PO PE P M`.

`step` switches on `x` and delegates to `exec_x0`, `exec_x1_load`, the inline
`x == 2` ALU case, and `exec_x3`. For example:

- `x == 1` is the entire `LD r[y], r[z]` register-copy block — *except* the one
  encoding where `y == z == 6`, which would be "load (HL) from (HL)" and is
  instead **`HALT`**. That single exception is why `LD` and `HALT` share a region.
- `x == 2` is `alu[y] A, r[z]` — every accumulator arithmetic/logic op against a
  register, in eight lines.

This structure also makes the prefix instructions tidy: the bytes `CB`, `DD`,
`ED`, and `FD` fall into specific slots in the `x == 3` group, where they
currently return `Stop::Illegal`. Implementing them (task `cpemu-yd5.3`) means
filling in those slots with sub-decoders, not disturbing the main table.

## The ALU and flags (`alu.rs`)

Flag computation is where CPU emulators go wrong, so it is isolated in `alu.rs`
as small methods that are unit-tested directly, independent of any instruction
decode. Each operation sets *all* the flags it affects, including `Y`/`X`.

Some specifics worth knowing because they are easy to get subtly wrong:

- **`add8`/`sub8`** compute the result in a wider integer so carry is just "did
  it exceed 8 bits", compute half-carry from the low nibble, and compute signed
  overflow from the sign bits of the operands and result.
- **`INC`/`DEC` do not affect the carry flag.** This is a common trip-up; `inc8`
  and `dec8` leave `C` untouched and set overflow only at the `0x7F→0x80` /
  `0x80→0x7F` boundaries.
- **`CP` takes its `Y`/`X` flags from the *operand*, not the result.** This is a
  genuine Z80 quirk: `CP` is a subtract that discards its result, but unlike
  `SUB` it copies bits 5 and 3 from the value being compared, not from the
  (discarded) difference. `cp8` does this explicitly.
- **`ADD HL,rr`** affects only `H`, `N`, and `C` (and the undocumented bits from
  the high byte); it leaves `S`, `Z`, and `P/V` alone.
- **`DAA`** is the decimal-adjust instruction and is notoriously fiddly; its
  implementation follows the standard table-driven correction and is covered by a
  test.

When you add instructions, add their flag checks here and test them in isolation
before wiring them into the decode. The exercisers will catch flag bugs
eventually, but a focused unit test catches them *now*, with a readable failure.

## Timing (T-states)

Each instruction reports how many T-states (clock cycles) it consumed. The values
encoded in the handlers are the standard documented Z80 timings. cpemu's goal is
*configurable-throttle* timing (run at roughly 4 MHz for authentic feel, or
unthrottled), not cycle-perfect emulation, so these base counts are sufficient;
we are not (yet) modeling the exact bus timing within an instruction.

## How to extend the instruction set

A practical recipe, following the existing structure:

1. Find the opcode's `x/y/z` slot (a Z80 opcode table or the decode rules above
   will tell you which `exec_*` function owns it).
2. If it needs a new arithmetic/logic primitive, add it to `alu.rs` **with a unit
   test for its flags**.
3. Implement the decode case, using the `reg_get`/`reg_set`/`rp_*`/`cond` helpers
   so `(HL)` and register selection are handled uniformly.
4. Assign the documented T-state count.
5. Add a small whole-program test in `cpu.rs` (load bytes into a `PlainBus`, run
   to `HALT`, assert the resulting state) so the instruction is exercised
   end-to-end, not just in theory.
6. Run `make check`.

## Current state

Implemented: the full **unprefixed** opcode map (the "8080 base" set) with Z80
flag semantics. Not yet implemented (returns `Stop::Illegal`): the `CB`, `DD`,
`ED`, `FD` prefixed instructions, and interrupt handling. See
[roadmap.md](roadmap.md) for the task breakdown.
