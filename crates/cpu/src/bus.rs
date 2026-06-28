// SPDX-License-Identifier: MIT
// Copyright (c) 2026 David Goldfarb <deg@degel.com>

//! The CPU's only contact with the outside world.
//!
//! The CPU is generic over a [`Bus`] implementation (monomorphized, not `dyn`)
//! so the hot execution loop stays fast and the crate stays dependency-free.
//! The real emulator supplies a bus backed by the virtual machine; tests and
//! the exerciser harness supply [`PlainBus`].

/// Memory and I/O port access for the CPU.
///
/// Ports are addressed with a full `u16`: the Z80 places the `B` register on
/// the high byte of the address bus for `IN r,(C)` / `OUT (C),r`, so port
/// numbers must not be truncated to `u8`.
pub trait Bus {
    /// Read one byte of memory.
    fn read(&mut self, addr: u16) -> u8;
    /// Write one byte of memory.
    fn write(&mut self, addr: u16, val: u8);
    /// Read a byte from an I/O port.
    fn input(&mut self, port: u16) -> u8;
    /// Write a byte to an I/O port.
    fn output(&mut self, port: u16, val: u8);

    /// Read a little-endian word from memory.
    fn read16(&mut self, addr: u16) -> u16 {
        u16::from(self.read(addr)) | (u16::from(self.read(addr.wrapping_add(1))) << 8)
    }
    /// Write a little-endian word to memory.
    fn write16(&mut self, addr: u16, val: u16) {
        self.write(addr, val as u8);
        self.write(addr.wrapping_add(1), (val >> 8) as u8);
    }
}

/// A trivial [`Bus`]: a flat 64K RAM and a flat 64K I/O space.
///
/// Used by unit tests and (later) the CPU exerciser harness, which loads a test
/// ROM into RAM and watches control flow.
pub struct PlainBus {
    mem: [u8; 0x1_0000],
    io: [u8; 0x1_0000],
}

impl PlainBus {
    /// Create a zero-initialized bus.
    pub fn new() -> Self {
        Self {
            mem: [0; 0x1_0000],
            io: [0; 0x1_0000],
        }
    }

    /// Load `bytes` into memory starting at `addr` (wrapping at 64K).
    pub fn load(&mut self, addr: u16, bytes: &[u8]) {
        for (i, &b) in bytes.iter().enumerate() {
            self.mem[addr.wrapping_add(i as u16) as usize] = b;
        }
    }
}

impl Default for PlainBus {
    fn default() -> Self {
        Self::new()
    }
}

impl Bus for PlainBus {
    fn read(&mut self, addr: u16) -> u8 {
        self.mem[addr as usize]
    }
    fn write(&mut self, addr: u16, val: u8) {
        self.mem[addr as usize] = val;
    }
    fn input(&mut self, port: u16) -> u8 {
        self.io[port as usize]
    }
    fn output(&mut self, port: u16, val: u8) {
        self.io[port as usize] = val;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn word_access_is_little_endian() {
        let mut bus = PlainBus::new();
        bus.write16(0x1000, 0xBEEF);
        assert_eq!(bus.read(0x1000), 0xEF);
        assert_eq!(bus.read(0x1001), 0xBE);
        assert_eq!(bus.read16(0x1000), 0xBEEF);
    }

    #[test]
    fn load_copies_bytes() {
        let mut bus = PlainBus::new();
        bus.load(0x0100, &[1, 2, 3]);
        assert_eq!(bus.read(0x0100), 1);
        assert_eq!(bus.read(0x0102), 3);
    }
}
