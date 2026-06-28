//! A virtual CP/M disk: a flat sector image plus the geometry CP/M needs.
//!
//! The default geometry is the canonical CP/M reference drive — an 8-inch
//! single-sided, single-density floppy: 77 tracks of 26 128-byte sectors, with
//! the first two tracks reserved for the system (the CCP+BDOS image the boot
//! loader reads). BDOS learns this geometry by reading the **Disk Parameter
//! Block** (`DPB`) that the BIOS hands it; the constants here and the bytes in
//! [`DPB_8INCH`] describe the same disk and must stay in agreement.
//!
//! Sectors are addressed without skew (the BIOS `SECTRAN` is identity), so the
//! image layout, the DPB, and `SECTRAN` are trivially self-consistent. Skew
//! fidelity only matters when loading real third-party images later.

/// Bytes per sector.
pub const SECTOR_SIZE: usize = 128;
/// Sectors per track.
pub const SECTORS_PER_TRACK: usize = 26;
/// Total tracks.
pub const TRACKS: usize = 77;
/// Reserved system tracks (hold the CCP+BDOS image).
pub const SYSTEM_TRACKS: usize = 2;
/// Total image size in bytes.
pub const DISK_SIZE: usize = TRACKS * SECTORS_PER_TRACK * SECTOR_SIZE;

/// The Disk Parameter Block for the 8-inch reference drive (15 bytes, the exact
/// layout BDOS reads). Block size 1024, 243 blocks, 64 directory entries, 2
/// reserved tracks.
///
/// Fields, in order: `SPT`(26) `BSH`(3) `BLM`(7) `EXM`(0) `DSM`(242) `DRM`(63)
/// `AL0`(0xC0) `AL1`(0) `CKS`(16) `OFF`(2). Words are little-endian.
pub const DPB_8INCH: [u8; 15] = [
    0x1A, 0x00, // SPT = 26
    0x03, // BSH
    0x07, // BLM
    0x00, // EXM
    0xF2, 0x00, // DSM = 242
    0x3F, 0x00, // DRM = 63
    0xC0, // AL0 (first two directory blocks)
    0x00, // AL1
    0x10, 0x00, // CKS = 16
    0x02, 0x00, // OFF = 2
];

/// A CP/M disk backed by a flat sector image.
pub struct Disk {
    data: Vec<u8>,
}

impl Disk {
    /// A blank, freshly-formatted disk: every byte `0xE5`, so the directory
    /// region reads as all-empty entries.
    pub fn blank() -> Self {
        Self {
            data: vec![0xE5; DISK_SIZE],
        }
    }

    /// Build a bootable system disk: a blank disk with the CCP+BDOS image
    /// written contiguously into the start of the system tracks, where the boot
    /// loader reads it from. The data area (directory and files) stays `0xE5`.
    pub fn make_system_disk(ccp: &[u8], bdos: &[u8]) -> Self {
        let mut disk = Self::blank();
        disk.data[..ccp.len()].copy_from_slice(ccp);
        disk.data[ccp.len()..ccp.len() + bdos.len()].copy_from_slice(bdos);
        disk
    }

    /// Byte offset of a (track, sector) pair. `sector` is 0-based.
    fn offset(track: usize, sector: usize) -> usize {
        (track * SECTORS_PER_TRACK + sector) * SECTOR_SIZE
    }

    /// Read one 128-byte sector (`sector` 0-based).
    pub fn read_sector(&self, track: usize, sector: usize) -> &[u8] {
        let o = Self::offset(track, sector);
        &self.data[o..o + SECTOR_SIZE]
    }

    /// Write one 128-byte sector (`sector` 0-based). `buf` must be 128 bytes.
    pub fn write_sector(&mut self, track: usize, sector: usize, buf: &[u8]) {
        let o = Self::offset(track, sector);
        self.data[o..o + SECTOR_SIZE].copy_from_slice(buf);
    }

    /// The raw bytes of the system area (the reserved tracks), which the boot
    /// loader copies into memory as the CCP+BDOS image.
    pub fn system_area(&self) -> &[u8] {
        &self.data[..SYSTEM_TRACKS * SECTORS_PER_TRACK * SECTOR_SIZE]
    }

    /// Write a directory entry's worth of bytes at a file's first directory
    /// slot — used by tests and tooling to place a file on the disk.
    pub fn data_mut(&mut self) -> &mut [u8] {
        &mut self.data
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blank_disk_is_all_empty_marker() {
        let d = Disk::blank();
        assert_eq!(d.data.len(), DISK_SIZE);
        assert!(d.data.iter().all(|&b| b == 0xE5));
        // The directory lives at the first data track (track 2, sector 0).
        assert!(d.read_sector(SYSTEM_TRACKS, 0).iter().all(|&b| b == 0xE5));
    }

    #[test]
    fn sector_addressing_is_linear() {
        assert_eq!(Disk::offset(0, 0), 0);
        assert_eq!(Disk::offset(0, 1), 128);
        assert_eq!(Disk::offset(1, 0), 26 * 128);
        assert_eq!(Disk::offset(2, 0), 2 * 26 * 128); // first data track = 0x1A00
    }

    #[test]
    fn system_disk_places_ccp_then_bdos() {
        let ccp = vec![0xAA; 0x800];
        let bdos = vec![0xBB; 0xE00];
        let d = Disk::make_system_disk(&ccp, &bdos);
        // CCP at track 0 sector 0.
        assert_eq!(d.read_sector(0, 0)[0], 0xAA);
        // BDOS immediately follows CCP (0x800 bytes in = track 0 sector 16).
        assert_eq!(d.read_sector(0, 16)[0], 0xBB);
        // The directory (track 2) is still empty.
        assert!(d.read_sector(2, 0).iter().all(|&b| b == 0xE5));
        // The whole CCP+BDOS image is contiguous at the start of the system area.
        assert_eq!(&d.system_area()[..0x800], &ccp[..]);
        assert_eq!(&d.system_area()[0x800..0x1600], &bdos[..]);
    }

    #[test]
    fn read_write_roundtrip() {
        let mut d = Disk::blank();
        let buf = [0x42u8; SECTOR_SIZE];
        d.write_sector(10, 5, &buf);
        assert_eq!(d.read_sector(10, 5), &buf);
    }

    #[test]
    fn dpb_describes_this_geometry() {
        // SPT in the DPB matches the sectors-per-track constant.
        let spt = u16::from(DPB_8INCH[0]) | (u16::from(DPB_8INCH[1]) << 8);
        assert_eq!(spt as usize, SECTORS_PER_TRACK);
        // OFF matches the reserved system tracks.
        let off = u16::from(DPB_8INCH[13]) | (u16::from(DPB_8INCH[14]) << 8);
        assert_eq!(off as usize, SYSTEM_TRACKS);
    }
}
