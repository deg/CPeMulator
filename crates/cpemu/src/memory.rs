// SPDX-License-Identifier: MIT
// Copyright (c) 2026 David Goldfarb <deg@degel.com>

//! The system's 64 KB of RAM.
//!
//! CP/M 2.2 runs in a flat 64 KB address space with no memory management, so
//! this is deliberately simple: a byte array with convenience accessors. The
//! [`crate::machine::Machine`] wraps it and presents it to the CPU through the
//! `Bus` trait.

/// 64 KB of system memory.
pub struct Memory {
    ram: Box<[u8; Self::SIZE]>,
}

impl Memory {
    /// The size of the address space (64 KB).
    pub const SIZE: usize = 0x1_0000;

    /// Create zeroed memory.
    pub fn new() -> Self {
        Self {
            ram: Box::new([0u8; Self::SIZE]),
        }
    }

    /// Read one byte.
    #[inline]
    pub fn read(&self, addr: u16) -> u8 {
        self.ram[addr as usize]
    }

    /// Write one byte.
    #[inline]
    pub fn write(&mut self, addr: u16, val: u8) {
        self.ram[addr as usize] = val;
    }

    /// Read a little-endian word (wraps at the top of memory).
    #[inline]
    pub fn read16(&self, addr: u16) -> u16 {
        u16::from(self.read(addr)) | (u16::from(self.read(addr.wrapping_add(1))) << 8)
    }

    /// Write a little-endian word (wraps at the top of memory).
    #[inline]
    pub fn write16(&mut self, addr: u16, val: u16) {
        self.write(addr, val as u8);
        self.write(addr.wrapping_add(1), (val >> 8) as u8);
    }

    /// Copy `bytes` into memory starting at `addr`, wrapping at 64 KB.
    pub fn load(&mut self, addr: u16, bytes: &[u8]) {
        for (i, &b) in bytes.iter().enumerate() {
            self.ram[addr.wrapping_add(i as u16) as usize] = b;
        }
    }

    /// Borrow a slice of memory `[addr, addr + len)` without wrapping. Panics if
    /// the range runs past the top of memory; callers that might wrap should use
    /// [`Memory::read`] in a loop instead.
    pub fn slice(&self, addr: u16, len: usize) -> &[u8] {
        let start = addr as usize;
        &self.ram[start..start + len]
    }
}

impl Default for Memory {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_write_roundtrip() {
        let mut m = Memory::new();
        m.write(0x1234, 0xAB);
        assert_eq!(m.read(0x1234), 0xAB);
        assert_eq!(m.read(0x0000), 0x00);
    }

    #[test]
    fn word_access_is_little_endian() {
        let mut m = Memory::new();
        m.write16(0x2000, 0xBEEF);
        assert_eq!(m.read(0x2000), 0xEF);
        assert_eq!(m.read(0x2001), 0xBE);
        assert_eq!(m.read16(0x2000), 0xBEEF);
    }

    #[test]
    fn load_and_slice() {
        let mut m = Memory::new();
        m.load(0x0100, &[1, 2, 3, 4]);
        assert_eq!(m.read(0x0100), 1);
        assert_eq!(m.read(0x0103), 4);
        assert_eq!(m.slice(0x0100, 4), &[1, 2, 3, 4]);
    }

    #[test]
    fn load_wraps_at_top() {
        let mut m = Memory::new();
        m.load(0xFFFF, &[0xAA, 0xBB]); // second byte wraps to 0x0000
        assert_eq!(m.read(0xFFFF), 0xAA);
        assert_eq!(m.read(0x0000), 0xBB);
    }
}
