// SPDX-License-Identifier: MIT
// Copyright (c) 2026 David Goldfarb <deg@degel.com>

//! The virtual machine: memory and I/O presented to the CPU as a `Bus`.
//!
//! `Machine` is the bridge between the CPU core and the system. The CPU calls
//! its `Bus` methods to read and write memory and to do port I/O. It also knows
//! the address range of the **BIOS trap region** — the small block of memory
//! whose addresses, when the CPU jumps to them, hand control to a Rust BIOS
//! handler (see [`crate::executor`]).
//!
//! Note what `Machine` deliberately does **not** hold: the BIOS trap *handler*.
//! The handler needs access to the CPU registers, but the CPU borrows the
//! `Machine` (as its `Bus`) for the duration of an instruction — so a handler
//! living inside `Machine` could never be given the CPU. The handler therefore
//! lives in the [`crate::executor::Executor`] alongside the CPU, and `Machine`
//! only describes *where* the traps are.

use crate::memory::Memory;
use cpemu_cpu::Bus;

/// The block of addresses that trap into the Rust BIOS.
///
/// A CP/M BIOS is a jump table of fixed-size entries (3-byte `JP` instructions),
/// so the region is described by a base address, the stride between entries, and
/// how many there are. When the CPU's PC lands exactly on an entry, that entry's
/// index is the BIOS function to perform.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrapRegion {
    /// Address of the first vector.
    pub base: u16,
    /// Bytes between consecutive vectors (3 for a standard CP/M `JP` table).
    pub stride: u16,
    /// Number of vectors.
    pub count: u8,
}

impl TrapRegion {
    /// If `pc` lands exactly on a vector in this region, return its index.
    ///
    /// A PC that falls *between* vectors (not a multiple of `stride`) is not a
    /// trap — it is ordinary code that happens to be in range — so it returns
    /// `None`. This guards against misreading a mid-vector address as a call.
    pub fn vector(&self, pc: u16) -> Option<u8> {
        let span = u32::from(self.stride) * u32::from(self.count);
        let offset = u32::from(pc.wrapping_sub(self.base));
        if offset >= span || offset % u32::from(self.stride) != 0 {
            return None;
        }
        Some((offset / u32::from(self.stride)) as u8)
    }
}

/// The virtual machine seen by the CPU: 64 KB of RAM plus an (open) I/O bus.
pub struct Machine {
    /// System memory.
    pub mem: Memory,
    /// Where the BIOS traps live.
    pub traps: TrapRegion,
}

impl Machine {
    /// Create a machine with zeroed memory and the given trap region.
    pub fn new(traps: TrapRegion) -> Self {
        Self {
            mem: Memory::new(),
            traps,
        }
    }

    /// The BIOS vector index if `pc` is on a trap, else `None`.
    pub fn trap_vector(&self, pc: u16) -> Option<u8> {
        self.traps.vector(pc)
    }
}

impl Bus for Machine {
    fn read(&mut self, addr: u16) -> u8 {
        self.mem.read(addr)
    }
    fn write(&mut self, addr: u16, val: u8) {
        self.mem.write(addr, val);
    }
    // No I/O devices yet: reads see an open bus (all ones) and writes are
    // discarded. A device map can be added behind these methods later without
    // touching the CPU.
    fn input(&mut self, _port: u16) -> u8 {
        0xFF
    }
    fn output(&mut self, _port: u16, _val: u8) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vector_indexes_each_entry() {
        let t = TrapRegion {
            base: 0x0200,
            stride: 3,
            count: 4,
        };
        assert_eq!(t.vector(0x0200), Some(0));
        assert_eq!(t.vector(0x0203), Some(1));
        assert_eq!(t.vector(0x0206), Some(2));
        assert_eq!(t.vector(0x0209), Some(3));
    }

    #[test]
    fn mid_vector_and_out_of_range_are_not_traps() {
        let t = TrapRegion {
            base: 0x0200,
            stride: 3,
            count: 4,
        };
        assert_eq!(t.vector(0x0201), None); // between vectors 0 and 1
        assert_eq!(t.vector(0x0202), None);
        assert_eq!(t.vector(0x020C), None); // just past the last vector
        assert_eq!(t.vector(0x01FF), None); // before the base
        assert_eq!(t.vector(0x0000), None);
    }

    #[test]
    fn io_is_open_bus() {
        let mut m = Machine::new(TrapRegion {
            base: 0xFA00,
            stride: 3,
            count: 17,
        });
        assert_eq!(m.input(0x00), 0xFF);
        m.output(0x00, 0x42); // no panic, no effect
    }
}
