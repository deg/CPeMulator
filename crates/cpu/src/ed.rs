// SPDX-License-Identifier: MIT
// Copyright (c) 2026 David Goldfarb <deg@degel.com>

//! The `ED`-prefixed instructions: 16-bit add/subtract with carry, direct
//! 16-bit memory loads, `NEG`, `RETN`/`RETI`, interrupt-mode selection, the
//! `I`/`R` register transfers, `RRD`/`RLD`, and the block move / compare /
//! I/O instructions (`LDIR`, `CPIR`, `INIR`, `OTIR`, and their variants).
//!
//! Repeating block instructions (`LDIR` etc.) are emulated one element per
//! `step`: each call processes a single element, and if the instruction should
//! repeat it rewinds `PC` by two so the same `ED xx` opcode runs again next
//! step. This keeps `step` "one instruction" while remaining interruptible,
//! exactly as the real CPU behaves between iterations.

use crate::alu::parity;
use crate::bus::Bus;
use crate::cpu::{Cpu, StepResult, Stop};
use crate::registers::flag;

/// `IM` mode selected by the `y` field of an `IM` opcode.
const IM_TABLE: [u8; 8] = [0, 0, 1, 2, 0, 0, 1, 2];

impl Cpu {
    /// Execute an `ED`-prefixed instruction. The `ED` byte has been consumed.
    pub(crate) fn exec_ed(&mut self, bus: &mut impl Bus) -> StepResult {
        let op = self.fetch(bus); // second M1 fetch — ticks R again
        let x = op >> 6;
        let y = (op >> 3) & 7;
        let z = op & 7;
        let p = y >> 1;
        let q = y & 1;

        match x {
            1 => self.exec_ed_x1(bus, y, z, p, q),
            2 => self.exec_ed_block(bus, y, z),
            // x == 0 and x == 3 are undefined; real hardware treats them as a
            // harmless two-byte no-op rather than trapping.
            _ => self.done(8, Stop::Normal),
        }
    }

    fn exec_ed_x1(&mut self, bus: &mut impl Bus, y: u8, z: u8, p: u8, q: u8) -> StepResult {
        match z {
            0 => {
                // IN r[y],(C) — read port BC; y==6 sets flags only.
                let port = self.regs.bc();
                let v = bus.input(port);
                self.set_in_flags(v);
                if y != 6 {
                    self.reg_set(y, v, bus);
                }
                self.done(12, Stop::Normal)
            }
            1 => {
                // OUT (C),r[y] — write port BC; y==6 writes 0.
                let port = self.regs.bc();
                let v = if y == 6 { 0 } else { self.reg_get(y, bus) };
                bus.output(port, v);
                self.done(12, Stop::Normal)
            }
            2 => {
                // SBC/ADC HL, rp[p]
                let hl = self.regs.hl();
                let rp = self.rp_get(p);
                let res = if q == 0 {
                    self.sbc16(hl, rp)
                } else {
                    self.adc16(hl, rp)
                };
                self.regs.set_hl(res);
                self.done(15, Stop::Normal)
            }
            3 => {
                // LD (nn),rp[p] / LD rp[p],(nn)
                let nn = self.next_word(bus);
                if q == 0 {
                    bus.write16(nn, self.rp_get(p));
                } else {
                    let v = bus.read16(nn);
                    self.rp_set(p, v);
                }
                self.done(20, Stop::Normal)
            }
            4 => {
                // NEG  (A = 0 - A); all y alias the documented NEG.
                let a = self.regs.a;
                self.regs.a = self.sub8(0, a, false);
                self.done(8, Stop::Normal)
            }
            5 => {
                // RETN (y==1 is RETI; functionally identical here).
                let addr = self.pop16(bus);
                self.regs.pc = addr;
                self.iff1 = self.iff2;
                self.done(14, Stop::Normal)
            }
            6 => {
                // IM
                self.im = IM_TABLE[y as usize];
                self.done(8, Stop::Normal)
            }
            _ => self.exec_ed_x1_z7(bus, y),
        }
    }

    fn exec_ed_x1_z7(&mut self, bus: &mut impl Bus, y: u8) -> StepResult {
        match y {
            0 => {
                self.regs.i = self.regs.a; // LD I,A
                self.done(9, Stop::Normal)
            }
            1 => {
                self.regs.r = self.regs.a; // LD R,A
                self.done(9, Stop::Normal)
            }
            2 => {
                self.load_a_from(self.regs.i); // LD A,I
                self.done(9, Stop::Normal)
            }
            3 => {
                self.load_a_from(self.regs.r); // LD A,R
                self.done(9, Stop::Normal)
            }
            4 => {
                self.rrd(bus); // RRD
                self.done(18, Stop::Normal)
            }
            5 => {
                self.rld(bus); // RLD
                self.done(18, Stop::Normal)
            }
            // y == 6, 7: documented NOP.
            _ => self.done(8, Stop::Normal),
        }
    }

    /// `LD A,I` / `LD A,R`: load A and set flags, with P/V reflecting IFF2.
    fn load_a_from(&mut self, v: u8) {
        self.regs.a = v;
        self.regs.set_flag(flag::S, v & 0x80 != 0);
        self.regs.set_flag(flag::Z, v == 0);
        self.regs.set_flag(flag::H, false);
        self.regs.set_flag(flag::N, false);
        self.regs.set_flag(flag::PV, self.iff2);
        self.regs.set_flag(flag::Y, v & flag::Y != 0);
        self.regs.set_flag(flag::X, v & flag::X != 0);
    }

    /// Flags shared by `IN r,(C)`: S/Z/parity/Y/X from the value, H and N
    /// cleared, carry untouched.
    fn set_in_flags(&mut self, v: u8) {
        self.regs.set_flag(flag::S, v & 0x80 != 0);
        self.regs.set_flag(flag::Z, v == 0);
        self.regs.set_flag(flag::H, false);
        self.regs.set_flag(flag::PV, parity(v));
        self.regs.set_flag(flag::N, false);
        self.regs.set_flag(flag::Y, v & flag::Y != 0);
        self.regs.set_flag(flag::X, v & flag::X != 0);
    }

    /// `RRD` — rotate the low BCD digit of A through the two digits of `(HL)`.
    fn rrd(&mut self, bus: &mut impl Bus) {
        let addr = self.regs.hl();
        let m = bus.read(addr);
        let a = self.regs.a;
        bus.write(addr, (a << 4) | (m >> 4));
        self.regs.a = (a & 0xf0) | (m & 0x0f);
        self.set_rxd_flags();
    }

    /// `RLD` — the left-rotating counterpart of `RRD`.
    fn rld(&mut self, bus: &mut impl Bus) {
        let addr = self.regs.hl();
        let m = bus.read(addr);
        let a = self.regs.a;
        bus.write(addr, (m << 4) | (a & 0x0f));
        self.regs.a = (a & 0xf0) | (m >> 4);
        self.set_rxd_flags();
    }

    fn set_rxd_flags(&mut self) {
        let a = self.regs.a;
        self.regs.set_flag(flag::S, a & 0x80 != 0);
        self.regs.set_flag(flag::Z, a == 0);
        self.regs.set_flag(flag::H, false);
        self.regs.set_flag(flag::PV, parity(a));
        self.regs.set_flag(flag::N, false);
        self.regs.set_flag(flag::Y, a & flag::Y != 0);
        self.regs.set_flag(flag::X, a & flag::X != 0);
    }

    /// `x == 2`: the block instructions. Only `z < 4` with `y >= 4` are defined.
    fn exec_ed_block(&mut self, bus: &mut impl Bus, y: u8, z: u8) -> StepResult {
        if y < 4 {
            return self.done(8, Stop::Normal); // undefined → no-op
        }
        let inc = y & 1 == 0; // y even (4,6) increments; odd (5,7) decrements
        let repeat = y >= 6; // y 6,7 are the repeating forms
        match z {
            0 => self.block_ld(bus, inc, repeat),
            1 => self.block_cp(bus, inc, repeat),
            2 => self.block_in(bus, inc, repeat),
            3 => self.block_out(bus, inc, repeat),
            _ => self.done(8, Stop::Normal),
        }
    }

    /// LDI / LDD / LDIR / LDDR.
    fn block_ld(&mut self, bus: &mut impl Bus, inc: bool, repeat: bool) -> StepResult {
        let v = bus.read(self.regs.hl());
        bus.write(self.regs.de(), v);
        self.step_hl_de(inc);
        let bc = self.regs.bc().wrapping_sub(1);
        self.regs.set_bc(bc);

        // Undocumented flags: n = A + transferred byte; X from bit 3, Y from bit 1.
        let n = self.regs.a.wrapping_add(v);
        self.regs.set_flag(flag::H, false);
        self.regs.set_flag(flag::N, false);
        self.regs.set_flag(flag::PV, bc != 0);
        self.regs.set_flag(flag::Y, n & 0x02 != 0);
        self.regs.set_flag(flag::X, n & 0x08 != 0);

        self.block_repeat(bc != 0, repeat)
    }

    /// CPI / CPD / CPIR / CPDR.
    fn block_cp(&mut self, bus: &mut impl Bus, inc: bool, repeat: bool) -> StepResult {
        let v = bus.read(self.regs.hl());
        let a = self.regs.a;
        let res = a.wrapping_sub(v);
        let half = (a & 0x0f).wrapping_sub(v & 0x0f) & 0x10 != 0;

        if inc {
            self.regs.set_hl(self.regs.hl().wrapping_add(1));
        } else {
            self.regs.set_hl(self.regs.hl().wrapping_sub(1));
        }
        let bc = self.regs.bc().wrapping_sub(1);
        self.regs.set_bc(bc);

        self.regs.set_flag(flag::S, res & 0x80 != 0);
        self.regs.set_flag(flag::Z, res == 0);
        self.regs.set_flag(flag::H, half);
        self.regs.set_flag(flag::N, true);
        self.regs.set_flag(flag::PV, bc != 0);
        // Undocumented: n = res - H; X from bit 3, Y from bit 1.
        let n = res.wrapping_sub(u8::from(half));
        self.regs.set_flag(flag::Y, n & 0x02 != 0);
        self.regs.set_flag(flag::X, n & 0x08 != 0);

        // CPIR/CPDR stop when BC == 0 or a match is found (Z set).
        self.block_repeat(bc != 0 && res != 0, repeat)
    }

    /// INI / IND / INIR / INDR.
    fn block_in(&mut self, bus: &mut impl Bus, inc: bool, repeat: bool) -> StepResult {
        let v = bus.input(self.regs.bc());
        bus.write(self.regs.hl(), v);
        let b = self.regs.b.wrapping_sub(1);
        self.regs.b = b;
        if inc {
            self.regs.set_hl(self.regs.hl().wrapping_add(1));
        } else {
            self.regs.set_hl(self.regs.hl().wrapping_sub(1));
        }
        let c_adjust = if inc {
            self.regs.c.wrapping_add(1)
        } else {
            self.regs.c.wrapping_sub(1)
        };
        self.set_block_io_flags(b, v, c_adjust);
        self.block_repeat(b != 0, repeat)
    }

    /// OUTI / OUTD / OTIR / OTDR.
    fn block_out(&mut self, bus: &mut impl Bus, inc: bool, repeat: bool) -> StepResult {
        let v = bus.read(self.regs.hl());
        let b = self.regs.b.wrapping_sub(1);
        self.regs.b = b;
        bus.output(self.regs.bc(), v);
        if inc {
            self.regs.set_hl(self.regs.hl().wrapping_add(1));
        } else {
            self.regs.set_hl(self.regs.hl().wrapping_sub(1));
        }
        self.set_block_io_flags(b, v, self.regs.l);
        self.block_repeat(b != 0, repeat)
    }

    /// The (obscure) flag rules shared by the block-I/O instructions. These are
    /// not part of the zexdoc gate and are provided for completeness.
    fn set_block_io_flags(&mut self, b: u8, value: u8, k_addend: u8) {
        let k = u16::from(value) + u16::from(k_addend);
        self.regs.set_flag(flag::S, b & 0x80 != 0);
        self.regs.set_flag(flag::Z, b == 0);
        self.regs.set_flag(flag::N, value & 0x80 != 0);
        self.regs.set_flag(flag::C, k > 0xff);
        self.regs.set_flag(flag::H, k > 0xff);
        self.regs.set_flag(flag::PV, parity(((k & 7) as u8) ^ b));
        self.regs.set_flag(flag::Y, b & flag::Y != 0);
        self.regs.set_flag(flag::X, b & flag::X != 0);
    }

    /// Advance HL and DE together for the LDI/LDD family.
    fn step_hl_de(&mut self, inc: bool) {
        if inc {
            self.regs.set_hl(self.regs.hl().wrapping_add(1));
            self.regs.set_de(self.regs.de().wrapping_add(1));
        } else {
            self.regs.set_hl(self.regs.hl().wrapping_sub(1));
            self.regs.set_de(self.regs.de().wrapping_sub(1));
        }
    }

    /// Finish a block instruction: if it repeats and the condition still holds,
    /// rewind PC by two so the same `ED xx` runs again next step (21 T-states);
    /// otherwise it is done (16 T-states).
    fn block_repeat(&mut self, condition: bool, repeat: bool) -> StepResult {
        if repeat && condition {
            self.regs.pc = self.regs.pc.wrapping_sub(2);
            self.done(21, Stop::Normal)
        } else {
            self.done(16, Stop::Normal)
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
            assert!(guard < 100_000, "runaway");
            match r.stop {
                Stop::Halted => break,
                Stop::Illegal(op) => panic!("illegal {op:#04x}"),
                Stop::Normal => {}
            }
        }
        (cpu, bus)
    }

    #[test]
    fn sbc_hl_sets_flags() {
        // LD HL,0x0005 ; LD BC,0x0005 ; SBC HL,BC (carry clear) ; HALT -> 0, Z set
        let (cpu, _) = run(&[
            0x21, 0x05, 0x00, // LD HL,5
            0x01, 0x05, 0x00, // LD BC,5
            0xED, 0x42, // SBC HL,BC
            0x76,
        ]);
        assert_eq!(cpu.regs.hl(), 0x0000);
        assert!(cpu.regs.flag(flag::Z));
        assert!(cpu.regs.flag(flag::N));
    }

    #[test]
    fn adc_hl_carry() {
        // LD HL,0xFFFF ; LD BC,1 ; ADC HL,BC ; HALT -> 0x0000, carry set
        let (cpu, _) = run(&[
            0x21, 0xFF, 0xFF, // LD HL,0xFFFF
            0x01, 0x01, 0x00, // LD BC,1
            0xED, 0x4A, // ADC HL,BC
            0x76,
        ]);
        assert_eq!(cpu.regs.hl(), 0x0000);
        assert!(cpu.regs.flag(flag::C));
        assert!(cpu.regs.flag(flag::Z));
    }

    #[test]
    fn neg_basic() {
        // LD A,1 ; NEG ; HALT -> 0xFF, carry set (A != 0), N set
        let (cpu, _) = run(&[0x3E, 0x01, 0xED, 0x44, 0x76]);
        assert_eq!(cpu.regs.a, 0xFF);
        assert!(cpu.regs.flag(flag::C));
        assert!(cpu.regs.flag(flag::N));
    }

    #[test]
    fn ld_nn_rp_roundtrip() {
        // LD BC,0xCAFE ; LD (0x60),BC ; LD DE,(0x60) ; HALT
        let (cpu, mut bus) = run(&[
            0x01, 0xFE, 0xCA, // LD BC,0xCAFE
            0xED, 0x43, 0x60, 0x00, // LD (0x0060),BC
            0xED, 0x5B, 0x60, 0x00, // LD DE,(0x0060)
            0x76,
        ]);
        assert_eq!(cpu.regs.de(), 0xCAFE);
        assert_eq!(bus.read16(0x0060), 0xCAFE);
    }

    #[test]
    fn ldir_copies_block() {
        // Copy 4 bytes from 0x40 to 0x50.
        // LD HL,0x40 ; LD DE,0x50 ; LD BC,4 ; LDIR ; HALT
        let mut cpu = Cpu::new();
        let mut bus = PlainBus::new();
        bus.load(
            0x0000,
            &[
                0x21, 0x40, 0x00, // LD HL,0x40
                0x11, 0x50, 0x00, // LD DE,0x50
                0x01, 0x04, 0x00, // LD BC,4
                0xED, 0xB0, // LDIR
                0x76,
            ],
        );
        bus.load(0x0040, &[0xDE, 0xAD, 0xBE, 0xEF]);
        loop {
            if cpu.step(&mut bus).stop == Stop::Halted {
                break;
            }
        }
        assert_eq!(bus.read(0x0050), 0xDE);
        assert_eq!(bus.read(0x0053), 0xEF);
        assert_eq!(cpu.regs.bc(), 0);
    }

    #[test]
    fn cpir_finds_match() {
        // Search for 0xBE starting at 0x40, length 4.
        // LD A,0xBE ; LD HL,0x40 ; LD BC,4 ; CPIR ; HALT
        let mut cpu = Cpu::new();
        let mut bus = PlainBus::new();
        bus.load(
            0x0000,
            &[
                0x3E, 0xBE, // LD A,0xBE
                0x21, 0x40, 0x00, // LD HL,0x40
                0x01, 0x04, 0x00, // LD BC,4
                0xED, 0xB1, // CPIR
                0x76,
            ],
        );
        bus.load(0x0040, &[0xDE, 0xAD, 0xBE, 0xEF]);
        loop {
            if cpu.step(&mut bus).stop == Stop::Halted {
                break;
            }
        }
        // Match at 0x42; HL advanced past it to 0x43.
        assert_eq!(cpu.regs.hl(), 0x0043);
        assert!(cpu.regs.flag(flag::Z));
    }

    #[test]
    fn rrd_rotates_nibbles() {
        // A=0x12, (HL)=0x34 ; RRD -> A=0x14, (HL)=0x23
        let mut cpu = Cpu::new();
        let mut bus = PlainBus::new();
        bus.load(
            0x0000,
            &[
                0x3E, 0x12, // LD A,0x12
                0x21, 0x70, 0x00, // LD HL,0x70
                0x36, 0x34, // LD (HL),0x34
                0xED, 0x67, // RRD
                0x76,
            ],
        );
        loop {
            if cpu.step(&mut bus).stop == Stop::Halted {
                break;
            }
        }
        assert_eq!(cpu.regs.a, 0x14);
        assert_eq!(bus.read(0x0070), 0x23);
    }

    #[test]
    fn im_and_ld_i() {
        // IM 2 ; LD A,0x7F ; LD I,A ; HALT
        let (cpu, _) = run(&[0xED, 0x5E, 0x3E, 0x7F, 0xED, 0x47, 0x76]);
        assert_eq!(cpu.im, 2);
        assert_eq!(cpu.regs.i, 0x7F);
    }
}
