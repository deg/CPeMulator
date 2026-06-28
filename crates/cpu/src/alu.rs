// SPDX-License-Identifier: MIT
// Copyright (c) 2026 David Goldfarb <deg@degel.com>

//! Arithmetic/logic operations and their flag effects.
//!
//! Flag computation is the part of a CPU core that exercisers are built to
//! catch, so it lives here in small, individually testable methods rather than
//! inline in the decode. Every operation sets the documented flags (S Z H P/V N
//! C) and also the undocumented Y/X bits (copies of result bits 5 and 3), which
//! the zexall stretch goal needs and which are cheap to maintain.

use crate::cpu::Cpu;
use crate::registers::flag;

/// Even parity (true when the number of set bits is even).
#[inline]
pub(crate) fn parity(v: u8) -> bool {
    v.count_ones().is_multiple_of(2)
}

impl Cpu {
    /// Copy the undocumented Y (bit 5) and X (bit 3) flags from `v`.
    #[inline]
    fn set_yx_from(&mut self, v: u8) {
        self.regs.set_flag(flag::Y, v & flag::Y != 0);
        self.regs.set_flag(flag::X, v & flag::X != 0);
    }

    /// 8-bit add (`ADD`/`ADC`): returns `a + b + carry_in`, setting flags.
    pub(crate) fn add8(&mut self, a: u8, b: u8, carry_in: bool) -> u8 {
        let c = u16::from(carry_in);
        let wide = u16::from(a) + u16::from(b) + c;
        let res = wide as u8;
        let half = (u16::from(a & 0x0f) + u16::from(b & 0x0f) + c) & 0x10 != 0;
        let overflow = ((a ^ b ^ 0x80) & (a ^ res) & 0x80) != 0;
        self.regs.set_flag(flag::S, res & 0x80 != 0);
        self.regs.set_flag(flag::Z, res == 0);
        self.regs.set_flag(flag::H, half);
        self.regs.set_flag(flag::PV, overflow);
        self.regs.set_flag(flag::N, false);
        self.regs.set_flag(flag::C, wide > 0xff);
        self.set_yx_from(res);
        res
    }

    /// 8-bit subtract (`SUB`/`SBC`): returns `a - b - carry_in`, setting flags.
    pub(crate) fn sub8(&mut self, a: u8, b: u8, carry_in: bool) -> u8 {
        let c = i16::from(carry_in);
        let wide = i16::from(a) - i16::from(b) - c;
        let res = wide as u8;
        let half = (i16::from(a & 0x0f) - i16::from(b & 0x0f) - c) & 0x10 != 0;
        let overflow = ((a ^ b) & (a ^ res) & 0x80) != 0;
        self.regs.set_flag(flag::S, res & 0x80 != 0);
        self.regs.set_flag(flag::Z, res == 0);
        self.regs.set_flag(flag::H, half);
        self.regs.set_flag(flag::PV, overflow);
        self.regs.set_flag(flag::N, true);
        self.regs.set_flag(flag::C, wide < 0);
        self.set_yx_from(res);
        res
    }

    /// Logical AND into A. H set, C cleared, parity flag.
    pub(crate) fn and8(&mut self, b: u8) {
        let res = self.regs.a & b;
        self.regs.a = res;
        self.set_logic_flags(res, true);
    }

    /// Logical OR into A. H and C cleared, parity flag.
    pub(crate) fn or8(&mut self, b: u8) {
        let res = self.regs.a | b;
        self.regs.a = res;
        self.set_logic_flags(res, false);
    }

    /// Logical XOR into A. H and C cleared, parity flag.
    pub(crate) fn xor8(&mut self, b: u8) {
        let res = self.regs.a ^ b;
        self.regs.a = res;
        self.set_logic_flags(res, false);
    }

    fn set_logic_flags(&mut self, res: u8, half: bool) {
        self.regs.set_flag(flag::S, res & 0x80 != 0);
        self.regs.set_flag(flag::Z, res == 0);
        self.regs.set_flag(flag::H, half);
        self.regs.set_flag(flag::PV, parity(res));
        self.regs.set_flag(flag::N, false);
        self.regs.set_flag(flag::C, false);
        self.set_yx_from(res);
    }

    /// `CP b` — compare A with `b` (subtract, discard result). The undocumented
    /// Y/X flags come from the **operand** `b`, not the result (a Z80 quirk).
    pub(crate) fn cp8(&mut self, b: u8) {
        let a = self.regs.a;
        self.sub8(a, b, false);
        self.set_yx_from(b);
    }

    /// `INC r` — increment, leaving the carry flag untouched.
    pub(crate) fn inc8(&mut self, v: u8) -> u8 {
        let res = v.wrapping_add(1);
        self.regs.set_flag(flag::S, res & 0x80 != 0);
        self.regs.set_flag(flag::Z, res == 0);
        self.regs.set_flag(flag::H, v & 0x0f == 0x0f);
        self.regs.set_flag(flag::PV, v == 0x7f);
        self.regs.set_flag(flag::N, false);
        self.set_yx_from(res);
        res
    }

    /// `DEC r` — decrement, leaving the carry flag untouched.
    pub(crate) fn dec8(&mut self, v: u8) -> u8 {
        let res = v.wrapping_sub(1);
        self.regs.set_flag(flag::S, res & 0x80 != 0);
        self.regs.set_flag(flag::Z, res == 0);
        self.regs.set_flag(flag::H, v & 0x0f == 0x00);
        self.regs.set_flag(flag::PV, v == 0x80);
        self.regs.set_flag(flag::N, true);
        self.set_yx_from(res);
        res
    }

    /// `ADD HL,rp` — 16-bit add. Sets H (carry out of bit 11) and C (carry out
    /// of bit 15), clears N; S/Z/P-V are unaffected. Y/X come from the high byte.
    pub(crate) fn add16(&mut self, a: u16, b: u16) -> u16 {
        let wide = u32::from(a) + u32::from(b);
        let res = wide as u16;
        let half = (u32::from(a & 0x0fff) + u32::from(b & 0x0fff)) & 0x1000 != 0;
        self.regs.set_flag(flag::H, half);
        self.regs.set_flag(flag::N, false);
        self.regs.set_flag(flag::C, wide > 0xffff);
        self.set_yx_from((res >> 8) as u8);
        res
    }

    /// `ADC HL,rr` — 16-bit add with carry. Unlike `ADD HL,rr`, this sets the
    /// full flag set: S/Z from the 16-bit result, H from carry out of bit 11,
    /// P/V from signed 16-bit overflow, N cleared, C from carry out of bit 15.
    pub(crate) fn adc16(&mut self, a: u16, b: u16) -> u16 {
        let c = u32::from(self.regs.flag(flag::C));
        let wide = u32::from(a) + u32::from(b) + c;
        let res = wide as u16;
        let half = (u32::from(a & 0x0fff) + u32::from(b & 0x0fff) + c) & 0x1000 != 0;
        let overflow = ((a ^ b ^ 0x8000) & (a ^ res) & 0x8000) != 0;
        self.regs.set_flag(flag::S, res & 0x8000 != 0);
        self.regs.set_flag(flag::Z, res == 0);
        self.regs.set_flag(flag::H, half);
        self.regs.set_flag(flag::PV, overflow);
        self.regs.set_flag(flag::N, false);
        self.regs.set_flag(flag::C, wide > 0xffff);
        self.set_yx_from((res >> 8) as u8);
        res
    }

    /// `SBC HL,rr` — 16-bit subtract with carry, setting the full flag set.
    pub(crate) fn sbc16(&mut self, a: u16, b: u16) -> u16 {
        let c = i32::from(self.regs.flag(flag::C));
        let wide = i32::from(a) - i32::from(b) - c;
        let res = wide as u16;
        let half = (i32::from(a & 0x0fff) - i32::from(b & 0x0fff) - c) & 0x1000 != 0;
        let overflow = ((a ^ b) & (a ^ res) & 0x8000) != 0;
        self.regs.set_flag(flag::S, res & 0x8000 != 0);
        self.regs.set_flag(flag::Z, res == 0);
        self.regs.set_flag(flag::H, half);
        self.regs.set_flag(flag::PV, overflow);
        self.regs.set_flag(flag::N, true);
        self.regs.set_flag(flag::C, wide < 0);
        self.set_yx_from((res >> 8) as u8);
        res
    }

    /// `RLCA` — rotate A left circular.
    pub(crate) fn rlca(&mut self) {
        let a = self.regs.a;
        let res = a.rotate_left(1);
        self.regs.a = res;
        self.rotate_a_flags(res, a & 0x80 != 0);
    }

    /// `RRCA` — rotate A right circular.
    pub(crate) fn rrca(&mut self) {
        let a = self.regs.a;
        let res = a.rotate_right(1);
        self.regs.a = res;
        self.rotate_a_flags(res, a & 0x01 != 0);
    }

    /// `RLA` — rotate A left through carry.
    pub(crate) fn rla(&mut self) {
        let a = self.regs.a;
        let res = (a << 1) | u8::from(self.regs.flag(flag::C));
        self.regs.a = res;
        self.rotate_a_flags(res, a & 0x80 != 0);
    }

    /// `RRA` — rotate A right through carry.
    pub(crate) fn rra(&mut self) {
        let a = self.regs.a;
        let res = (a >> 1) | (u8::from(self.regs.flag(flag::C)) << 7);
        self.regs.a = res;
        self.rotate_a_flags(res, a & 0x01 != 0);
    }

    /// Flag effects shared by the A-register rotates: H and N cleared, C from
    /// the rotated-out bit, S/Z/P-V unaffected, Y/X from the result.
    fn rotate_a_flags(&mut self, res: u8, carry: bool) {
        self.regs.set_flag(flag::H, false);
        self.regs.set_flag(flag::N, false);
        self.regs.set_flag(flag::C, carry);
        self.set_yx_from(res);
    }

    /// `CPL` — one's complement A. Sets H and N.
    pub(crate) fn cpl(&mut self) {
        self.regs.a = !self.regs.a;
        self.regs.set_flag(flag::H, true);
        self.regs.set_flag(flag::N, true);
        let a = self.regs.a;
        self.set_yx_from(a);
    }

    /// `SCF` — set carry flag (clears H and N).
    pub(crate) fn scf(&mut self) {
        self.regs.set_flag(flag::C, true);
        self.regs.set_flag(flag::H, false);
        self.regs.set_flag(flag::N, false);
        let a = self.regs.a;
        self.set_yx_from(a);
    }

    /// `CCF` — complement carry flag (old carry copied into H, N cleared).
    pub(crate) fn ccf(&mut self) {
        let old = self.regs.flag(flag::C);
        self.regs.set_flag(flag::H, old);
        self.regs.set_flag(flag::C, !old);
        self.regs.set_flag(flag::N, false);
        let a = self.regs.a;
        self.set_yx_from(a);
    }

    /// `DAA` — decimal adjust A after an add or subtract of BCD operands.
    pub(crate) fn daa(&mut self) {
        let a = self.regs.a;
        let n = self.regs.flag(flag::N);
        let h = self.regs.flag(flag::H);
        let c = self.regs.flag(flag::C);

        let mut diff: u8 = 0;
        let mut new_c = false;
        if h || (a & 0x0f) > 9 {
            diff |= 0x06;
        }
        if c || a > 0x99 {
            diff |= 0x60;
            new_c = true;
        }

        let res = if n {
            a.wrapping_sub(diff)
        } else {
            a.wrapping_add(diff)
        };
        // Half-carry after the adjust.
        let new_h = if n {
            h && (a & 0x0f) < 0x06
        } else {
            (a & 0x0f) > 9
        };

        self.regs.a = res;
        self.regs.set_flag(flag::S, res & 0x80 != 0);
        self.regs.set_flag(flag::Z, res == 0);
        self.regs.set_flag(flag::H, new_h);
        self.regs.set_flag(flag::PV, parity(res));
        // N is preserved by DAA.
        self.regs.set_flag(flag::C, c || new_c);
        self.set_yx_from(res);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cpu() -> Cpu {
        Cpu::new()
    }

    #[test]
    fn add8_half_and_carry() {
        let mut c = cpu();
        let r = c.add8(0x0f, 0x01, false);
        assert_eq!(r, 0x10);
        assert!(c.regs.flag(flag::H));
        assert!(!c.regs.flag(flag::C));
        let r = c.add8(0xff, 0x01, false);
        assert_eq!(r, 0x00);
        assert!(c.regs.flag(flag::Z));
        assert!(c.regs.flag(flag::C));
        assert!(c.regs.flag(flag::H));
    }

    #[test]
    fn add8_signed_overflow() {
        let mut c = cpu();
        // 0x7f + 0x01 = 0x80: signed overflow.
        c.add8(0x7f, 0x01, false);
        assert!(c.regs.flag(flag::PV));
        assert!(c.regs.flag(flag::S));
    }

    #[test]
    fn sub8_borrow_and_flags() {
        let mut c = cpu();
        let r = c.sub8(0x10, 0x01, false);
        assert_eq!(r, 0x0f);
        assert!(c.regs.flag(flag::H));
        assert!(c.regs.flag(flag::N));
        assert!(!c.regs.flag(flag::C));
        let r = c.sub8(0x00, 0x01, false);
        assert_eq!(r, 0xff);
        assert!(c.regs.flag(flag::C));
        assert!(c.regs.flag(flag::S));
    }

    #[test]
    fn logic_flags() {
        let mut c = cpu();
        c.regs.a = 0xf0;
        c.and8(0x0f);
        assert_eq!(c.regs.a, 0x00);
        assert!(c.regs.flag(flag::Z));
        assert!(c.regs.flag(flag::H));
        assert!(!c.regs.flag(flag::C));
        assert!(c.regs.flag(flag::PV)); // parity of 0 is even

        c.regs.a = 0xff;
        c.or8(0x00);
        assert_eq!(c.regs.a, 0xff);
        assert!(!c.regs.flag(flag::H));
        assert!(c.regs.flag(flag::S));
    }

    #[test]
    fn cp_uses_operand_for_yx() {
        let mut c = cpu();
        c.regs.a = 0x00;
        // operand has bit5 and bit3 set; result (0x00 - 0x28) bits differ.
        c.cp8(0x28);
        assert!(c.regs.flag(flag::Y));
        assert!(c.regs.flag(flag::X));
        assert!(c.regs.flag(flag::N));
    }

    #[test]
    fn inc_dec_preserve_carry() {
        let mut c = cpu();
        c.regs.set_flag(flag::C, true);
        let r = c.inc8(0x7f);
        assert_eq!(r, 0x80);
        assert!(c.regs.flag(flag::PV)); // overflow 0x7f->0x80
        assert!(c.regs.flag(flag::H));
        assert!(c.regs.flag(flag::C)); // carry untouched
        let r = c.dec8(0x00);
        assert_eq!(r, 0xff);
        assert!(c.regs.flag(flag::N));
        assert!(c.regs.flag(flag::C)); // still untouched
    }

    #[test]
    fn add16_flags() {
        let mut c = cpu();
        let r = c.add16(0x0fff, 0x0001);
        assert_eq!(r, 0x1000);
        assert!(c.regs.flag(flag::H));
        assert!(!c.regs.flag(flag::C));
        let r = c.add16(0xffff, 0x0001);
        assert_eq!(r, 0x0000);
        assert!(c.regs.flag(flag::C));
    }

    #[test]
    fn daa_after_add() {
        let mut c = cpu();
        // 0x09 + 0x08 = 0x11 binary; as BCD should be 0x17.
        c.regs.a = 0x09;
        let r = c.add8(0x09, 0x08, false);
        c.regs.a = r;
        c.daa();
        assert_eq!(c.regs.a, 0x17);
    }

    #[test]
    fn rotates_carry() {
        let mut c = cpu();
        c.regs.a = 0x80;
        c.rlca();
        assert_eq!(c.regs.a, 0x01);
        assert!(c.regs.flag(flag::C));
        c.regs.a = 0x01;
        c.regs.set_flag(flag::C, false);
        c.rra();
        assert_eq!(c.regs.a, 0x00);
        assert!(c.regs.flag(flag::C));
    }
}
