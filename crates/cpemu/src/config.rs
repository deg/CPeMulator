//! The machine configuration — the seam between "our virtual machine" and a
//! possible future real machine.
//!
//! Everything about *where things live* in the address space is described here,
//! in one place, rather than scattered as constants through the boot loader and
//! BIOS. A different machine (a historical Altair, Kaypro, or RC2014) would be a
//! different [`MachineConfig`] with a different memory map and, eventually, a
//! different set of devices — without the layers above having to change shape.
//!
//! This is also where the **CP/M memory map is decided**. The genuine DRI
//! CCP/BDOS assemble at an origin that depends on this map, so the boot epic
//! assembles them against the addresses chosen here.

use crate::machine::{Machine, TrapRegion};

/// Size of the CP/M 2.2 CCP (Console Command Processor): 2 KB.
pub const CCP_SIZE: u16 = 0x0800;
/// Size of the CP/M 2.2 BDOS: 3.5 KB.
pub const BDOS_SIZE: u16 = 0x0E00;
/// Number of entries in the standard CP/M 2.2 BIOS jump table.
///
/// `BOOT, WBOOT, CONST, CONIN, CONOUT, LIST, PUNCH, READER, HOME, SELDSK,
/// SETTRK, SETSEC, SETDMA, READ, WRITE, LISTST, SECTRAN` — 17 vectors.
pub const BIOS_VECTORS: u8 = 17;
/// Stride between BIOS jump-table entries (a 3-byte `JP`).
pub const BIOS_STRIDE: u16 = 3;
/// Where a transient program (a `.COM`) is loaded and run.
pub const TPA_BASE: u16 = 0x0100;

/// Describes the machine's memory layout (and, later, its devices).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MachineConfig {
    /// A human-readable name for the machine.
    pub name: &'static str,
    /// Size of the address space in bytes (64 KB for CP/M 2.2).
    pub memory_size: usize,
    /// Load address of the CCP.
    pub ccp_base: u16,
    /// Load address of the BDOS.
    pub bdos_base: u16,
    /// Base of the BIOS jump table (and the start of the trap region).
    pub bios_base: u16,
}

impl MachineConfig {
    /// cpemu's own virtual machine: a full 64 KB CP/M 2.2 system.
    ///
    /// The high end of memory holds, in order, the CCP, the BDOS, and the BIOS
    /// jump table; everything from [`TPA_BASE`] up to the CCP is the transient
    /// program area (~57 KB). These addresses are what the genuine CCP/BDOS are
    /// assembled against in the boot epic.
    ///
    /// ```text
    ///   0x0000  page zero (warm-boot & BDOS entry vectors, default FCB, DMA)
    ///   0x0100  TPA — transient programs load and run here  ┐
    ///     ...                                               │ ~57 KB
    ///   0xE400  CCP   (2 KB)                                ┘
    ///   0xEC00  BDOS  (3.5 KB)
    ///   0xFA00  BIOS  jump table (trap region) and beyond
    /// ```
    pub fn virtual_default() -> Self {
        let ccp_base = 0xE400;
        Self {
            name: "cpemu virtual machine",
            memory_size: 0x1_0000,
            ccp_base,
            bdos_base: ccp_base + CCP_SIZE,             // 0xEC00
            bios_base: ccp_base + CCP_SIZE + BDOS_SIZE, // 0xFA00
        }
    }

    /// The BIOS trap region implied by this layout.
    pub fn trap_region(&self) -> TrapRegion {
        TrapRegion {
            base: self.bios_base,
            stride: BIOS_STRIDE,
            count: BIOS_VECTORS,
        }
    }

    /// The BDOS entry point that page zero's `CALL 5` vector should target. The
    /// BDOS image begins with a 6-byte serial number, so its entry is 6 bytes in.
    pub fn bdos_entry(&self) -> u16 {
        self.bdos_base + 6
    }

    /// Address of a given BIOS vector (`0 = BOOT`, `1 = WBOOT`, …).
    pub fn bios_vector(&self, index: u8) -> u16 {
        self.bios_base + u16::from(index) * BIOS_STRIDE
    }
}

impl Default for MachineConfig {
    fn default() -> Self {
        Self::virtual_default()
    }
}

impl Machine {
    /// Build a machine from a configuration (using its trap region).
    pub fn from_config(config: &MachineConfig) -> Self {
        Machine::new(config.trap_region())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_layout_is_ordered_and_sized() {
        let c = MachineConfig::virtual_default();
        assert_eq!(c.memory_size, 0x1_0000);
        // TPA below CCP below BDOS below BIOS, contiguous and correctly sized.
        assert!(TPA_BASE < c.ccp_base);
        assert_eq!(c.bdos_base, c.ccp_base + CCP_SIZE);
        assert_eq!(c.bios_base, c.bdos_base + BDOS_SIZE);
        // The whole system fits in 64 KB with room for the BIOS table.
        let bios_end = c.bios_base as usize + BIOS_VECTORS as usize * BIOS_STRIDE as usize;
        assert!(bios_end <= c.memory_size);
    }

    #[test]
    fn trap_region_matches_config() {
        let c = MachineConfig::virtual_default();
        let t = c.trap_region();
        assert_eq!(t.base, c.bios_base);
        assert_eq!(t.count, BIOS_VECTORS);
        // The WBOOT vector (index 1) is one stride past BOOT.
        assert_eq!(c.bios_vector(1), c.bios_base + BIOS_STRIDE);
        assert_eq!(t.vector(c.bios_vector(1)), Some(1));
    }

    #[test]
    fn from_config_builds_matching_machine() {
        let c = MachineConfig::virtual_default();
        let m = Machine::from_config(&c);
        assert_eq!(m.traps, c.trap_region());
        // BDOS entry skips the 6-byte serial number.
        assert_eq!(c.bdos_entry(), c.bdos_base + 6);
    }
}
