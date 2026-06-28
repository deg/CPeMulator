// SPDX-License-Identifier: MIT
// Copyright (c) 2026 David Goldfarb <deg@degel.com>

//! The `DD` and `FD` prefixes: `IX`/`IY`-indexed instructions.
//!
//! A `DD` (or `FD`) prefix modifies the instruction that follows it: `HL`
//! becomes `IX` (`IY`), the register halves `H`/`L` become `IXH`/`IXL`, and the
//! memory operand `(HL)` becomes `(IX+d)` with a signed displacement byte `d`.
//! Opcodes that do not reference `HL`/`H`/`L`/`(HL)` are unaffected — for those
//! the prefix is effectively a no-op, so we **delegate** them straight back to
//! the normal decode ([`Cpu::execute`]) and only special-case the ones that
//! actually change. This keeps the index handler small and avoids duplicating
//! the entire opcode table.
//!
//! One important subtlety: when an instruction uses the `(IX+d)` memory operand,
//! its *other* register operand is the real `H`/`L`, **not** `IXH`/`IXL`. The
//! half-register substitution only applies when there is no displacement
//! operand. The `halves` flag below encodes exactly this.

use crate::bus::Bus;
use crate::cpu::{Cpu, StepResult, Stop};

/// Which index register a `DD`/`FD` prefix selects.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Index {
    /// `IX` (the `DD` prefix).
    Ix,
    /// `IY` (the `FD` prefix).
    Iy,
}

/// True for the `r[]` fields affected by an index prefix: H (4), L (5), (HL) (6).
#[inline]
fn affected(field: u8) -> bool {
    (4..=6).contains(&field)
}

impl Cpu {
    fn ireg_get(&self, index: Index) -> u16 {
        match index {
            Index::Ix => self.regs.ix,
            Index::Iy => self.regs.iy,
        }
    }

    fn ireg_set(&mut self, index: Index, v: u16) {
        match index {
            Index::Ix => self.regs.ix = v,
            Index::Iy => self.regs.iy = v,
        }
    }

    fn ireg_high(&self, index: Index) -> u8 {
        (self.ireg_get(index) >> 8) as u8
    }
    fn ireg_low(&self, index: Index) -> u8 {
        self.ireg_get(index) as u8
    }
    fn set_ireg_high(&mut self, index: Index, v: u8) {
        let cur = self.ireg_get(index);
        self.ireg_set(index, (cur & 0x00ff) | (u16::from(v) << 8));
    }
    fn set_ireg_low(&mut self, index: Index, v: u8) {
        let cur = self.ireg_get(index);
        self.ireg_set(index, (cur & 0xff00) | u16::from(v));
    }

    /// `rp[p]` for index mode: `p == 2` is the index register instead of `HL`.
    fn idx_rp_get(&self, p: u8, index: Index) -> u16 {
        if p == 2 {
            self.ireg_get(index)
        } else {
            self.rp_get(p)
        }
    }

    /// Read the displacement byte and return the effective address `index + d`.
    fn fetch_disp(&mut self, bus: &mut impl Bus, index: Index) -> u16 {
        let d = self.next_byte(bus) as i8;
        self.ireg_get(index).wrapping_add(d as u16)
    }

    /// Read operand `field` in index mode. `ea` is the precomputed `(index+d)`
    /// address (only consulted when `field == 6`); `halves` selects whether
    /// fields 4/5 mean `IXH`/`IXL` (true) or the real `H`/`L` (false).
    fn idx_get(
        &mut self,
        field: u8,
        index: Index,
        ea: Option<u16>,
        halves: bool,
        bus: &mut impl Bus,
    ) -> u8 {
        match field {
            4 if halves => self.ireg_high(index),
            5 if halves => self.ireg_low(index),
            6 => bus.read(ea.expect("memory operand without an effective address")),
            other => self.reg_get(other, bus),
        }
    }

    /// Write operand `field` in index mode (see [`Cpu::idx_get`]).
    fn idx_set(
        &mut self,
        field: u8,
        index: Index,
        ea: Option<u16>,
        halves: bool,
        val: u8,
        bus: &mut impl Bus,
    ) {
        match field {
            4 if halves => self.set_ireg_high(index, val),
            5 if halves => self.set_ireg_low(index, val),
            6 => bus.write(
                ea.expect("memory operand without an effective address"),
                val,
            ),
            other => self.reg_set(other, val, bus),
        }
    }

    /// Run an opcode the index prefix does not affect, charging the 4 T-states
    /// the ignored prefix cost.
    fn delegate(&mut self, op: u8, bus: &mut impl Bus) -> StepResult {
        let r = self.execute(op, bus);
        self.cycles += 4;
        StepResult {
            cycles: r.cycles + 4,
            stop: r.stop,
        }
    }

    /// Execute a `DD`/`FD`-prefixed instruction. The prefix byte is consumed.
    pub(crate) fn exec_index(&mut self, bus: &mut impl Bus, index: Index) -> StepResult {
        let op = self.fetch(bus); // second M1 fetch — ticks R again

        // Prefix chains and other prefixes: the last index prefix wins; ED
        // ignores the index entirely.
        match op {
            0xDD => return self.exec_index(bus, Index::Ix),
            0xFD => return self.exec_index(bus, Index::Iy),
            0xED => return self.exec_ed(bus),
            0xCB => return self.exec_index_cb(bus, index),
            _ => {}
        }

        let x = op >> 6;
        let y = (op >> 3) & 7;
        let z = op & 7;
        let p = y >> 1;
        let q = y & 1;

        match x {
            0 => self.index_x0(bus, index, op, y, z, p, q),
            1 => self.index_x1_load(bus, index, op, y, z),
            2 => {
                // alu A, r[z]
                if !affected(z) {
                    return self.delegate(op, bus);
                }
                let mem = z == 6;
                let ea = if mem {
                    Some(self.fetch_disp(bus, index))
                } else {
                    None
                };
                let v = self.idx_get(z, index, ea, !mem, bus);
                self.alu(y, v);
                self.done(if mem { 19 } else { 8 }, Stop::Normal)
            }
            _ => self.index_x3(bus, index, op, y, z, p, q),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn index_x0(
        &mut self,
        bus: &mut impl Bus,
        index: Index,
        op: u8,
        y: u8,
        z: u8,
        p: u8,
        q: u8,
    ) -> StepResult {
        match z {
            1 => {
                if q == 0 {
                    // LD rp[p],nn — only HL→index (p==2) is affected.
                    if p != 2 {
                        return self.delegate(op, bus);
                    }
                    let nn = self.next_word(bus);
                    self.ireg_set(index, nn);
                    self.done(14, Stop::Normal)
                } else {
                    // ADD index, rp[p]
                    let rp = self.idx_rp_get(p, index);
                    let res = self.add16(self.ireg_get(index), rp);
                    self.ireg_set(index, res);
                    self.done(15, Stop::Normal)
                }
            }
            2 => {
                // LD (nn),index / LD index,(nn) — only p==2 is affected.
                if p != 2 {
                    return self.delegate(op, bus);
                }
                let nn = self.next_word(bus);
                if q == 0 {
                    bus.write16(nn, self.ireg_get(index));
                } else {
                    let v = bus.read16(nn);
                    self.ireg_set(index, v);
                }
                self.done(20, Stop::Normal)
            }
            3 => {
                // INC/DEC rp[p] — only p==2 (the index register) is affected.
                if p != 2 {
                    return self.delegate(op, bus);
                }
                let v = self.ireg_get(index);
                let v = if q == 0 {
                    v.wrapping_add(1)
                } else {
                    v.wrapping_sub(1)
                };
                self.ireg_set(index, v);
                self.done(10, Stop::Normal)
            }
            4 | 5 => {
                // INC/DEC r[y]
                if !affected(y) {
                    return self.delegate(op, bus);
                }
                let mem = y == 6;
                let ea = if mem {
                    Some(self.fetch_disp(bus, index))
                } else {
                    None
                };
                let v = self.idx_get(y, index, ea, !mem, bus);
                let r = if z == 4 { self.inc8(v) } else { self.dec8(v) };
                self.idx_set(y, index, ea, !mem, r, bus);
                self.done(if mem { 23 } else { 8 }, Stop::Normal)
            }
            6 => {
                // LD r[y],n
                if !affected(y) {
                    return self.delegate(op, bus);
                }
                let mem = y == 6;
                let ea = if mem {
                    Some(self.fetch_disp(bus, index))
                } else {
                    None
                };
                let n = self.next_byte(bus);
                self.idx_set(y, index, ea, !mem, n, bus);
                self.done(if mem { 19 } else { 11 }, Stop::Normal)
            }
            // z == 0 (NOP/JR/EX AF/DJNZ) and z == 7 (rotates/DAA/…) are never
            // affected by an index prefix.
            _ => self.delegate(op, bus),
        }
    }

    fn index_x1_load(
        &mut self,
        bus: &mut impl Bus,
        index: Index,
        op: u8,
        y: u8,
        z: u8,
    ) -> StepResult {
        // HALT (y==z==6) is unaffected; and the instruction is only affected if
        // some operand is H/L/(HL).
        if (y == 6 && z == 6) || !(affected(y) || affected(z)) {
            return self.delegate(op, bus);
        }
        let mem = y == 6 || z == 6;
        let halves = !mem; // with a displacement operand, the other reg is real H/L
        let ea = if mem {
            Some(self.fetch_disp(bus, index))
        } else {
            None
        };
        let v = self.idx_get(z, index, ea, halves, bus);
        self.idx_set(y, index, ea, halves, v, bus);
        self.done(if mem { 19 } else { 8 }, Stop::Normal)
    }

    #[allow(clippy::too_many_arguments)]
    fn index_x3(
        &mut self,
        bus: &mut impl Bus,
        index: Index,
        op: u8,
        y: u8,
        z: u8,
        p: u8,
        q: u8,
    ) -> StepResult {
        match z {
            1 => {
                if q == 0 {
                    // POP rp2[p] — only p==2 (POP index) is affected.
                    if p != 2 {
                        return self.delegate(op, bus);
                    }
                    let v = self.pop16(bus);
                    self.ireg_set(index, v);
                    self.done(14, Stop::Normal)
                } else {
                    match p {
                        2 => {
                            self.regs.pc = self.ireg_get(index); // JP (index)
                            self.done(8, Stop::Normal)
                        }
                        3 => {
                            self.regs.sp = self.ireg_get(index); // LD SP,index
                            self.done(10, Stop::Normal)
                        }
                        _ => self.delegate(op, bus), // RET, EXX
                    }
                }
            }
            3 if y == 4 => {
                // EX (SP),index
                let sp = self.regs.sp;
                let mem = bus.read16(sp);
                bus.write16(sp, self.ireg_get(index));
                self.ireg_set(index, mem);
                self.done(23, Stop::Normal)
            }
            5 if q == 0 && p == 2 => {
                // PUSH index
                self.push16(bus, self.ireg_get(index));
                self.done(15, Stop::Normal)
            }
            _ => self.delegate(op, bus),
        }
    }

    /// `DD CB d op` / `FD CB d op`: bit/rotate/shift on `(index+d)`. The byte
    /// order is prefix, `CB`, displacement, then the operation byte — so the
    /// displacement is read *before* the operation. Rotates/shifts and RES/SET
    /// also (undocumented) copy their result into `r[z]` when `z != 6`.
    fn exec_index_cb(&mut self, bus: &mut impl Bus, index: Index) -> StepResult {
        let ea = self.fetch_disp(bus, index);
        let op = self.next_byte(bus);
        let x = op >> 6;
        let y = (op >> 3) & 7;
        let z = op & 7;
        let m = bus.read(ea);

        match x {
            0 => {
                let r = self.cb_rotate(y, m);
                bus.write(ea, r);
                if z != 6 {
                    self.reg_set(z, r, bus);
                }
                self.done(23, Stop::Normal)
            }
            1 => {
                // BIT y,(index+d): Y/X come from the high byte of the address.
                self.cb_bit_with_yx(y, m, (ea >> 8) as u8);
                self.done(20, Stop::Normal)
            }
            2 => {
                let r = m & !(1 << y);
                bus.write(ea, r);
                if z != 6 {
                    self.reg_set(z, r, bus);
                }
                self.done(23, Stop::Normal)
            }
            _ => {
                let r = m | (1 << y);
                bus.write(ea, r);
                if z != 6 {
                    self.reg_set(z, r, bus);
                }
                self.done(23, Stop::Normal)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::PlainBus;

    fn run(program: &[u8]) -> (Cpu, PlainBus) {
        let mut cpu = Cpu::new();
        let mut bus = PlainBus::new();
        bus.load(0x0000, program);
        let mut guard = 0;
        loop {
            let r = cpu.step(&mut bus);
            guard += 1;
            assert!(guard < 10_000, "runaway");
            match r.stop {
                Stop::Halted => break,
                Stop::Illegal(op) => panic!("illegal {op:#04x}"),
                Stop::Normal => {}
            }
        }
        (cpu, bus)
    }

    #[test]
    fn ld_ix_immediate_and_inc() {
        // LD IX,0x1234 ; INC IX ; HALT
        let (cpu, _) = run(&[0xDD, 0x21, 0x34, 0x12, 0xDD, 0x23, 0x76]);
        assert_eq!(cpu.regs.ix, 0x1235);
    }

    #[test]
    fn ld_iy_and_store_indexed() {
        // LD IY,0x40 ; LD A,0xAB ; LD (IY+3),A ; HALT  -> (0x43) == 0xAB
        let (_, mut bus) = run(&[
            0xFD, 0x21, 0x40, 0x00, // LD IY,0x40
            0x3E, 0xAB, // LD A,0xAB
            0xFD, 0x77, 0x03, // LD (IY+3),A
            0x76,
        ]);
        assert_eq!(bus.read(0x0043), 0xAB);
    }

    #[test]
    fn load_indexed_with_negative_displacement() {
        // LD IX,0x50 ; LD (IX-1),value via reg ; read back into B
        // LD IX,0x50 ; LD A,0x99 ; LD (IX-1),A ; LD B,(IX-1) ; HALT
        let (cpu, bus) = run(&[
            0xDD, 0x21, 0x50, 0x00, // LD IX,0x50
            0x3E, 0x99, // LD A,0x99
            0xDD, 0x77, 0xFF, // LD (IX-1),A   (d = -1)
            0xDD, 0x46, 0xFF, // LD B,(IX-1)
            0x76,
        ]);
        let mut bus = bus;
        assert_eq!(bus.read(0x004F), 0x99);
        assert_eq!(cpu.regs.b, 0x99);
    }

    #[test]
    fn add_a_indexed() {
        // LD IX,0x60 ; LD (IX+0),0x05 ; LD A,0x03 ; ADD A,(IX+0) ; HALT -> A=0x08
        let (cpu, _) = run(&[
            0xDD, 0x21, 0x60, 0x00, // LD IX,0x60
            0xDD, 0x36, 0x00, 0x05, // LD (IX+0),0x05
            0x3E, 0x03, // LD A,0x03
            0xDD, 0x86, 0x00, // ADD A,(IX+0)
            0x76,
        ]);
        assert_eq!(cpu.regs.a, 0x08);
    }

    #[test]
    fn add_ix_rp() {
        // LD IX,0x1000 ; LD BC,0x0234 ; ADD IX,BC ; HALT -> IX=0x1234
        let (cpu, _) = run(&[
            0xDD, 0x21, 0x00, 0x10, // LD IX,0x1000
            0x01, 0x34, 0x02, // LD BC,0x0234
            0xDD, 0x09, // ADD IX,BC
            0x76,
        ]);
        assert_eq!(cpu.regs.ix, 0x1234);
    }

    #[test]
    fn undocumented_ixh_half() {
        // LD IX,0x1234 ; INC IXH ; HALT -> IX high byte 0x12 -> 0x13
        let (cpu, _) = run(&[0xDD, 0x21, 0x34, 0x12, 0xDD, 0x24, 0x76]);
        assert_eq!(cpu.regs.ix, 0x1334);
    }

    #[test]
    fn ddcb_set_bit_on_indexed() {
        // LD IX,0x70 ; LD (IX+0),0 ; SET 7,(IX+0) ; HALT -> (0x70)==0x80
        let (_, mut bus) = run(&[
            0xDD, 0x21, 0x70, 0x00, // LD IX,0x70
            0xDD, 0x36, 0x00, 0x00, // LD (IX+0),0
            0xDD, 0xCB, 0x00, 0xFE, // SET 7,(IX+0)
            0x76,
        ]);
        assert_eq!(bus.read(0x0070), 0x80);
    }

    #[test]
    fn unaffected_opcode_is_delegated() {
        // DD before LD A,n (unaffected) just runs LD A,n.
        // DD ; LD A,0x42 ; HALT
        let (cpu, _) = run(&[0xDD, 0x3E, 0x42, 0x76]);
        assert_eq!(cpu.regs.a, 0x42);
    }

    #[test]
    fn jp_index_jumps() {
        // LD IX,0x06 ; JP (IX) ; (skipped) ; at 0x06: LD A,1 ; HALT
        let mut cpu = Cpu::new();
        let mut bus = PlainBus::new();
        bus.load(
            0x0000,
            &[
                0xDD, 0x21, 0x06, 0x00, // LD IX,0x06
                0xDD, 0xE9, // JP (IX)
                0x3E, 0x01, // 0x06: LD A,1
                0x76, // HALT
            ],
        );
        loop {
            if cpu.step(&mut bus).stop == Stop::Halted {
                break;
            }
        }
        assert_eq!(cpu.regs.a, 0x01);
    }
}
