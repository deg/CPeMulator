// SPDX-License-Identifier: MIT
// Copyright (c) 2026 David Goldfarb <deg@degel.com>

//! The Z80 programmer-visible register file.
//!
//! Individual 8-bit registers with 16-bit pair accessors, the shadow register
//! set (`AF'`, `BC'`, `DE'`, `HL'`) reached via `EX AF,AF'` and `EXX`, the
//! 16-bit `IX`/`IY`/`SP`/`PC`, and the `I`/`R` interrupt/refresh registers.

/// Status flag bit masks within the `F` register.
///
/// `Y` (bit 5) and `X` (bit 3) are the undocumented flags: they copy bits 5 and
/// 3 of an instruction's result. They are required for the zexall stretch goal
/// but cheap to maintain, so we carry them from the start.
pub mod flag {
    /// Sign (bit 7): set when the result's high bit is set.
    pub const S: u8 = 0b1000_0000;
    /// Zero (bit 6): set when the result is zero.
    pub const Z: u8 = 0b0100_0000;
    /// Undocumented (bit 5): copy of bit 5 of the result.
    pub const Y: u8 = 0b0010_0000;
    /// Half-carry (bit 4): carry out of bit 3.
    pub const H: u8 = 0b0001_0000;
    /// Undocumented (bit 3): copy of bit 3 of the result.
    pub const X: u8 = 0b0000_1000;
    /// Parity/overflow (bit 2).
    pub const PV: u8 = 0b0000_0100;
    /// Add/subtract (bit 1): set after a subtraction, for DAA.
    pub const N: u8 = 0b0000_0010;
    /// Carry (bit 0).
    pub const C: u8 = 0b0000_0001;
}

/// The Z80 register file.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Registers {
    pub a: u8,
    pub f: u8,
    pub b: u8,
    pub c: u8,
    pub d: u8,
    pub e: u8,
    pub h: u8,
    pub l: u8,

    // Shadow (alternate) register set.
    pub a_alt: u8,
    pub f_alt: u8,
    pub b_alt: u8,
    pub c_alt: u8,
    pub d_alt: u8,
    pub e_alt: u8,
    pub h_alt: u8,
    pub l_alt: u8,

    pub ix: u16,
    pub iy: u16,
    pub sp: u16,
    pub pc: u16,

    /// Interrupt vector register.
    pub i: u8,
    /// Memory refresh register.
    pub r: u8,
}

/// Combine two bytes (high, low) into a word.
#[inline]
fn pair(hi: u8, lo: u8) -> u16 {
    (u16::from(hi) << 8) | u16::from(lo)
}

impl Registers {
    /// `AF` register pair.
    #[inline]
    pub fn af(&self) -> u16 {
        pair(self.a, self.f)
    }
    /// `BC` register pair.
    #[inline]
    pub fn bc(&self) -> u16 {
        pair(self.b, self.c)
    }
    /// `DE` register pair.
    #[inline]
    pub fn de(&self) -> u16 {
        pair(self.d, self.e)
    }
    /// `HL` register pair.
    #[inline]
    pub fn hl(&self) -> u16 {
        pair(self.h, self.l)
    }

    /// Set the `AF` register pair.
    #[inline]
    pub fn set_af(&mut self, v: u16) {
        self.a = (v >> 8) as u8;
        self.f = v as u8;
    }
    /// Set the `BC` register pair.
    #[inline]
    pub fn set_bc(&mut self, v: u16) {
        self.b = (v >> 8) as u8;
        self.c = v as u8;
    }
    /// Set the `DE` register pair.
    #[inline]
    pub fn set_de(&mut self, v: u16) {
        self.d = (v >> 8) as u8;
        self.e = v as u8;
    }
    /// Set the `HL` register pair.
    #[inline]
    pub fn set_hl(&mut self, v: u16) {
        self.h = (v >> 8) as u8;
        self.l = v as u8;
    }

    /// Test whether the given flag bit(s) are set in `F`.
    #[inline]
    pub fn flag(&self, mask: u8) -> bool {
        self.f & mask != 0
    }

    /// Set or clear the given flag bit(s) in `F`.
    #[inline]
    pub fn set_flag(&mut self, mask: u8, value: bool) {
        if value {
            self.f |= mask;
        } else {
            self.f &= !mask;
        }
    }

    /// `EX AF,AF'` — exchange `AF` with the shadow `AF'`.
    #[inline]
    pub fn ex_af(&mut self) {
        core::mem::swap(&mut self.a, &mut self.a_alt);
        core::mem::swap(&mut self.f, &mut self.f_alt);
    }

    /// `EXX` — exchange `BC`/`DE`/`HL` with their shadows.
    #[inline]
    pub fn exx(&mut self) {
        core::mem::swap(&mut self.b, &mut self.b_alt);
        core::mem::swap(&mut self.c, &mut self.c_alt);
        core::mem::swap(&mut self.d, &mut self.d_alt);
        core::mem::swap(&mut self.e, &mut self.e_alt);
        core::mem::swap(&mut self.h, &mut self.h_alt);
        core::mem::swap(&mut self.l, &mut self.l_alt);
    }

    /// Increment the refresh register, preserving bit 7 (as real hardware does:
    /// only the low 7 bits count up on each instruction fetch).
    #[inline]
    pub fn inc_r(&mut self) {
        self.r = (self.r & 0x80) | (self.r.wrapping_add(1) & 0x7f);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pairs_round_trip() {
        let mut r = Registers::default();
        r.set_bc(0x1234);
        assert_eq!(r.b, 0x12);
        assert_eq!(r.c, 0x34);
        assert_eq!(r.bc(), 0x1234);
        r.set_af(0xABCD);
        assert_eq!(r.af(), 0xABCD);
    }

    #[test]
    fn flags_set_and_test() {
        let mut r = Registers::default();
        r.set_flag(flag::Z | flag::C, true);
        assert!(r.flag(flag::Z));
        assert!(r.flag(flag::C));
        assert!(!r.flag(flag::S));
        r.set_flag(flag::Z, false);
        assert!(!r.flag(flag::Z));
        assert!(r.flag(flag::C));
    }

    #[test]
    fn ex_af_and_exx_swap_with_shadows() {
        let mut r = Registers::default();
        r.set_af(0x1111);
        r.set_bc(0x2222);
        r.set_de(0x3333);
        r.set_hl(0x4444);
        r.ex_af();
        r.exx();
        assert_eq!(r.af(), 0x0000);
        assert_eq!(r.bc(), 0x0000);
        r.ex_af();
        r.exx();
        assert_eq!(r.af(), 0x1111);
        assert_eq!(r.hl(), 0x4444);
    }

    #[test]
    fn inc_r_preserves_high_bit() {
        let mut r = Registers {
            r: 0xFF,
            ..Default::default()
        };
        r.inc_r();
        assert_eq!(r.r, 0x80); // wrapped low 7 bits to 0, bit 7 kept
        r.r = 0x7F;
        r.inc_r();
        assert_eq!(r.r, 0x00);
    }
}
