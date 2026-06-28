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

use std::collections::HashSet;

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

/// Allocation block size (1 KB = 8 sectors), per the 8" DPB (`BSH=3`).
pub const BLOCK_SIZE: usize = 1024;
/// Sectors per allocation block.
pub const SECTORS_PER_BLOCK: usize = BLOCK_SIZE / SECTOR_SIZE;
/// Number of 32-byte directory entries (`DRM+1 = 64`).
pub const DIR_ENTRIES: usize = 64;
/// Size of a directory entry.
pub const DIR_ENTRY_SIZE: usize = 32;
/// Marks a free/deleted directory entry (and a blank sector).
const EMPTY: u8 = 0xE5;
/// The first two allocation blocks (0, 1) hold the directory; files start at 2.
const FIRST_DATA_BLOCK: usize = 2;
/// One past the last allocation block (`DSM+1 = 243`).
const BLOCK_COUNT: usize = 243;
/// Blocks addressable by one directory extent (16 single-byte block numbers,
/// since `DSM < 256`).
const BLOCKS_PER_EXTENT: usize = 16;
/// 128-byte records covered by one extent (16 blocks x 8 sectors).
const RECORDS_PER_EXTENT: usize = BLOCKS_PER_EXTENT * SECTORS_PER_BLOCK;

/// Why placing a file on a disk failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiskError {
    /// The name is not a valid CP/M `8.3` name.
    BadName(String),
    /// No free directory entries remain.
    DirectoryFull,
    /// No free data blocks remain.
    DiskFull,
}

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

    // --- directory & allocation -----------------------------------------
    //
    // The data area begins at track `SYSTEM_TRACKS`. Allocation block `b`
    // occupies data sectors `b*8 .. b*8+8`; the directory is blocks 0-1. BDOS
    // computes file sector locations exactly this way, so a file written here is
    // read back correctly by the genuine BDOS.

    /// Map a data-area sector index (0-based from the first data track) to a
    /// physical (track, sector).
    fn data_location(data_sector: usize) -> (usize, usize) {
        (
            SYSTEM_TRACKS + data_sector / SECTORS_PER_TRACK,
            data_sector % SECTORS_PER_TRACK,
        )
    }

    /// Read directory entry `idx`.
    fn dir_entry(&self, idx: usize) -> [u8; DIR_ENTRY_SIZE] {
        let byte = idx * DIR_ENTRY_SIZE;
        let (t, s) = Self::data_location(byte / SECTOR_SIZE);
        let within = byte % SECTOR_SIZE;
        let mut entry = [0u8; DIR_ENTRY_SIZE];
        entry.copy_from_slice(&self.read_sector(t, s)[within..within + DIR_ENTRY_SIZE]);
        entry
    }

    /// Write directory entry `idx`.
    fn set_dir_entry(&mut self, idx: usize, entry: &[u8; DIR_ENTRY_SIZE]) {
        let byte = idx * DIR_ENTRY_SIZE;
        let (t, s) = Self::data_location(byte / SECTOR_SIZE);
        let o = Self::offset(t, s) + byte % SECTOR_SIZE;
        self.data[o..o + DIR_ENTRY_SIZE].copy_from_slice(entry);
    }

    /// Write up to one block of file data into allocation block `block`,
    /// zero-padding the rest of the block.
    fn write_block(&mut self, block: usize, data: &[u8]) {
        for i in 0..SECTORS_PER_BLOCK {
            let (t, s) = Self::data_location(block * SECTORS_PER_BLOCK + i);
            let o = Self::offset(t, s);
            let mut buf = [0u8; SECTOR_SIZE];
            let start = i * SECTOR_SIZE;
            if start < data.len() {
                let n = (data.len() - start).min(SECTOR_SIZE);
                buf[..n].copy_from_slice(&data[start..start + n]);
            }
            self.data[o..o + SECTOR_SIZE].copy_from_slice(&buf);
        }
    }

    /// The set of allocation blocks currently in use by directory entries.
    fn used_blocks(&self) -> HashSet<usize> {
        let mut used = HashSet::new();
        for idx in 0..DIR_ENTRIES {
            let e = self.dir_entry(idx);
            if e[0] == EMPTY {
                continue;
            }
            for &b in &e[16..32] {
                if b != 0 {
                    used.insert(b as usize);
                }
            }
        }
        used
    }

    /// Index of the first free directory entry, if any.
    fn free_dir_entry(&self) -> Option<usize> {
        (0..DIR_ENTRIES).find(|&idx| self.dir_entry(idx)[0] == EMPTY)
    }

    /// Place a file on the disk in user area 0, given an `8.3` name (e.g.
    /// `"HELLO.COM"`) and its raw contents. This is how the host puts programs
    /// and data onto a fresh disk before booting; from then on CP/M's own tools
    /// (`PIP`, `ED`) can manage files itself.
    pub fn add_file(&mut self, filename: &str, data: &[u8]) -> Result<(), DiskError> {
        let (name, ext) = parse_filename(filename)?;

        // Allocate the data blocks.
        let needed = data.len().div_ceil(BLOCK_SIZE).max(1);
        let mut used = self.used_blocks();
        let mut blocks = Vec::with_capacity(needed);
        let mut b = FIRST_DATA_BLOCK;
        while blocks.len() < needed {
            if b >= BLOCK_COUNT {
                return Err(DiskError::DiskFull);
            }
            if used.insert(b) {
                blocks.push(b);
            }
            b += 1;
        }
        for (i, &blk) in blocks.iter().enumerate() {
            let start = i * BLOCK_SIZE;
            self.write_block(blk, &data[start..data.len().min(start + BLOCK_SIZE)]);
        }

        // Write one directory entry (extent) per 16 blocks.
        let total_records = data.len().div_ceil(SECTOR_SIZE).max(1);
        let extents = blocks.len().div_ceil(BLOCKS_PER_EXTENT).max(1);
        for ex in 0..extents {
            let idx = self.free_dir_entry().ok_or(DiskError::DirectoryFull)?;
            let mut entry = [0u8; DIR_ENTRY_SIZE];
            entry[0] = 0; // user 0
            entry[1..9].copy_from_slice(&name);
            entry[9..12].copy_from_slice(&ext);
            entry[12] = ex as u8; // EX (extent number)
            entry[15] = (total_records - ex * RECORDS_PER_EXTENT).min(RECORDS_PER_EXTENT) as u8;
            let base = ex * BLOCKS_PER_EXTENT;
            for (j, slot) in entry[16..32].iter_mut().enumerate() {
                if let Some(&blk) = blocks.get(base + j) {
                    *slot = blk as u8;
                }
            }
            self.set_dir_entry(idx, &entry);
        }
        Ok(())
    }

    /// List the names of the files on the disk (first extent of each), e.g.
    /// `["HELLO.COM"]`. For inspection and tests.
    pub fn list_files(&self) -> Vec<String> {
        let mut files = Vec::new();
        for idx in 0..DIR_ENTRIES {
            let e = self.dir_entry(idx);
            if e[0] == EMPTY || e[12] != 0 {
                continue; // empty, or a continuation extent
            }
            // High bits of the name/ext bytes are file attributes — mask them.
            let name: String = e[1..9].iter().map(|b| (b & 0x7F) as char).collect();
            let ext: String = e[9..12].iter().map(|b| (b & 0x7F) as char).collect();
            let name = name.trim_end();
            let ext = ext.trim_end();
            files.push(if ext.is_empty() {
                name.to_string()
            } else {
                format!("{name}.{ext}")
            });
        }
        files
    }
}

/// Parse an `8.3` filename into space-padded name and extension fields.
fn parse_filename(filename: &str) -> Result<([u8; 8], [u8; 3]), DiskError> {
    let upper = filename.trim().to_ascii_uppercase();
    let (name, ext) = upper.split_once('.').unwrap_or((upper.as_str(), ""));
    if name.is_empty() || name.len() > 8 || ext.len() > 3 {
        return Err(DiskError::BadName(filename.to_string()));
    }
    let mut name_buf = [b' '; 8];
    let mut ext_buf = [b' '; 3];
    name_buf[..name.len()].copy_from_slice(name.as_bytes());
    ext_buf[..ext.len()].copy_from_slice(ext.as_bytes());
    Ok((name_buf, ext_buf))
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

    #[test]
    fn add_file_then_list() {
        let mut d = Disk::blank();
        d.add_file("hello.com", &[1, 2, 3]).unwrap();
        assert_eq!(d.list_files(), vec!["HELLO.COM"]);
    }

    #[test]
    fn added_file_lands_in_first_data_block() {
        let mut d = Disk::blank();
        d.add_file("X.BIN", &[0xAB, 0xCD]).unwrap();
        // First data block is 2 -> data sector 16 -> track 2, sector 16.
        assert_eq!(d.read_sector(2, 16)[..2], [0xAB, 0xCD]);
        // The directory entry records block 2 and one record.
        let e = d.dir_entry(0);
        assert_eq!(e[0], 0); // user 0
        assert_eq!(e[15], 1); // RC = 1 record
        assert_eq!(e[16], 2); // first allocation block
    }

    #[test]
    fn two_files_get_distinct_blocks() {
        let mut d = Disk::blank();
        d.add_file("A.COM", &[0u8; 2000]).unwrap(); // 2 blocks
        d.add_file("B.COM", &[0u8; 10]).unwrap(); // 1 block
        let a = d.dir_entry(0);
        let b = d.dir_entry(1);
        assert_eq!(&a[16..18], &[2, 3]); // A uses blocks 2,3
        assert_eq!(b[16], 4); // B gets the next free block
        let mut names = d.list_files();
        names.sort();
        assert_eq!(names, vec!["A.COM", "B.COM"]);
    }

    #[test]
    fn bad_names_are_rejected() {
        let mut d = Disk::blank();
        assert!(matches!(
            d.add_file("TOOLONGNAME.COM", &[0]),
            Err(DiskError::BadName(_))
        ));
        assert!(matches!(d.add_file("", &[0]), Err(DiskError::BadName(_))));
    }
}
