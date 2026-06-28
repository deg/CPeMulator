// SPDX-License-Identifier: MIT
// Copyright (c) 2026 David Goldfarb <deg@degel.com>

//! The `CB`-prefixed instructions: rotates, shifts, and single-bit
//! `BIT`/`RES`/`SET` operations on the 8-bit registers and `(HL)`.
//!
//! Unlike the accumulator rotates in the unprefixed set (`RLCA` etc., which
//! leave S/Z/P-V alone), the `CB` rotates and shifts set the full set of flags:
//! S and Z from the result, parity in P/V, H and N cleared, carry from the
//! shifted-out bit.

use crate::bus::Bus;
use crate::cpu::{Cpu, StepResult, Stop};
use crate::registers::flag;

/// Index of the `(HL)` pseudo-register in the `r[]` table.
const HL_MEM: u8 = 6;

impl Cpu {
    /// One of the eight rotate/shift operations selected by `op` (0..7:
    /// RLC, RRC, RL, RR, SLA, SRA, SLL, SRL), returning the result and setting
    /// flags. `SLL` (op 6) is the undocumented "shift left, set bit 0".
    pub(crate) fn cb_rotate(&mut self, op: u8, v: u8) -> u8 {
        let old_c = self.regs.flag(flag::C);
        let (res, carry) = match op {
            0 => (v.rotate_left(1), v & 0x80 != 0),           // RLC
            1 => (v.rotate_right(1), v & 0x01 != 0),          // RRC
            2 => ((v << 1) | u8::from(old_c), v & 0x80 != 0), // RL
            3 => ((v >> 1) | (u8::from(old_c) << 7), v & 0x01 != 0), // RR
            4 => (v << 1, v & 0x80 != 0),                     // SLA
            5 => ((v >> 1) | (v & 0x80), v & 0x01 != 0),      // SRA (keeps sign)
            6 => ((v << 1) | 0x01, v & 0x80 != 0),            // SLL (undocumented)
            _ => (v >> 1, v & 0x01 != 0),                     // SRL
        };
        self.set_shift_flags(res, carry);
        res
    }

    fn set_shift_flags(&mut self, res: u8, carry: bool) {
        self.regs.set_flag(flag::S, res & 0x80 != 0);
        self.regs.set_flag(flag::Z, res == 0);
        self.regs.set_flag(flag::H, false);
        self.regs.set_flag(flag::PV, crate::alu::parity(res));
        self.regs.set_flag(flag::N, false);
        self.regs.set_flag(flag::C, carry);
        self.regs.set_flag(flag::Y, res & flag::Y != 0);
        self.regs.set_flag(flag::X, res & flag::X != 0);
    }

    /// `BIT n, v` — test bit `n` of `v` and set flags (the value is unchanged).
    /// The undocumented Y/X come from the tested value itself.
    fn cb_bit(&mut self, n: u8, v: u8) {
        self.cb_bit_with_yx(n, v, v);
    }

    /// `BIT n, v` with the undocumented Y/X flags sourced separately. For
    /// `BIT n,(HL)` and the `DD CB`/`FD CB` `(IX+d)`/`(IY+d)` forms, Y/X come not
    /// from the tested byte but from the high byte of the internal address
    /// (`yx_src`), which is why this is split out.
    ///
    /// Z (and the parity bit, which mirrors Z here) reflect whether the bit is
    /// clear; H is set, N cleared, carry untouched. S is set only when testing
    /// bit 7 and it is set.
    pub(crate) fn cb_bit_with_yx(&mut self, n: u8, v: u8, yx_src: u8) {
        let bit_set = v & (1 << n) != 0;
        self.regs.set_flag(flag::Z, !bit_set);
        self.regs.set_flag(flag::PV, !bit_set);
        self.regs.set_flag(flag::H, true);
        self.regs.set_flag(flag::N, false);
        self.regs.set_flag(flag::S, n == 7 && bit_set);
        self.regs.set_flag(flag::Y, yx_src & flag::Y != 0);
        self.regs.set_flag(flag::X, yx_src & flag::X != 0);
    }

    /// Execute a `CB`-prefixed instruction. The `CB` byte has already been
    /// consumed; this fetches and runs the operation byte.
    pub(crate) fn exec_cb(&mut self, bus: &mut impl Bus) -> StepResult {
        let op = self.fetch(bus); // second M1 fetch — ticks R again
        let x = op >> 6;
        let y = (op >> 3) & 7;
        let z = op & 7;
        let is_mem = z == HL_MEM;

        match x {
            0 => {
                // rotate/shift rot[y] r[z]
                let v = self.reg_get(z, bus);
                let r = self.cb_rotate(y, v);
                self.reg_set(z, r, bus);
                self.done(if is_mem { 15 } else { 8 }, Stop::Normal)
            }
            1 => {
                // BIT y, r[z]
                let v = self.reg_get(z, bus);
                self.cb_bit(y, v);
                self.done(if is_mem { 12 } else { 8 }, Stop::Normal)
            }
            2 => {
                // RES y, r[z]
                let v = self.reg_get(z, bus);
                self.reg_set(z, v & !(1 << y), bus);
                self.done(if is_mem { 15 } else { 8 }, Stop::Normal)
            }
            _ => {
                // SET y, r[z]
                let v = self.reg_get(z, bus);
                self.reg_set(z, v | (1 << y), bus);
                self.done(if is_mem { 15 } else { 8 }, Stop::Normal)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::PlainBus;

    fn run(program: &[u8]) -> Cpu {
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
        cpu
    }

    #[test]
    fn rlc_sets_carry_and_flags() {
        // LD A,0x85 ; RLC A ; HALT   (0x85 -> 0x0B, carry from bit7)
        let cpu = run(&[0x3E, 0x85, 0xCB, 0x07, 0x76]);
        assert_eq!(cpu.regs.a, 0x0B);
        assert!(cpu.regs.flag(flag::C));
    }

    #[test]
    fn sra_preserves_sign() {
        // LD A,0x80 ; SRA A ; HALT   (0x80 -> 0xC0)
        let cpu = run(&[0x3E, 0x80, 0xCB, 0x2F, 0x76]);
        assert_eq!(cpu.regs.a, 0xC0);
        assert!(!cpu.regs.flag(flag::C));
    }

    #[test]
    fn srl_shifts_in_zero() {
        // LD A,0x01 ; SRL A ; HALT   (0x01 -> 0x00, carry set, zero set)
        let cpu = run(&[0x3E, 0x01, 0xCB, 0x3F, 0x76]);
        assert_eq!(cpu.regs.a, 0x00);
        assert!(cpu.regs.flag(flag::C));
        assert!(cpu.regs.flag(flag::Z));
    }

    #[test]
    fn bit_set_clears_zero() {
        // LD A,0x08 ; BIT 3,A ; HALT  (bit 3 set -> Z clear, H set)
        let cpu = run(&[0x3E, 0x08, 0xCB, 0x5F, 0x76]);
        assert!(!cpu.regs.flag(flag::Z));
        assert!(cpu.regs.flag(flag::H));
        assert_eq!(cpu.regs.a, 0x08); // BIT does not modify the register
    }

    #[test]
    fn bit_clear_sets_zero() {
        // LD A,0x00 ; BIT 3,A ; HALT
        let cpu = run(&[0x3E, 0x00, 0xCB, 0x5F, 0x76]);
        assert!(cpu.regs.flag(flag::Z));
    }

    #[test]
    fn res_and_set() {
        // LD A,0xFF ; RES 0,A ; SET 1 already set... ; HALT
        // RES 0,A: 0xFF -> 0xFE
        let cpu = run(&[0x3E, 0xFF, 0xCB, 0x87, 0x76]);
        assert_eq!(cpu.regs.a, 0xFE);
    }

    #[test]
    fn set_on_memory() {
        // LD HL,0x50 ; LD (HL),0 ; SET 7,(HL) ; HALT  -> (0x50) == 0x80
        let mut cpu = Cpu::new();
        let mut bus = PlainBus::new();
        bus.load(
            0x0000,
            &[
                0x21, 0x50, 0x00, // LD HL,0x0050
                0x36, 0x00, // LD (HL),0
                0xCB, 0xFE, // SET 7,(HL)
                0x76, // HALT
            ],
        );
        loop {
            if cpu.step(&mut bus).stop == Stop::Halted {
                break;
            }
        }
        assert_eq!(bus.read(0x0050), 0x80);
    }
}
