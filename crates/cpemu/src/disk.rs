//! Virtual CP/M disks: a flat sector image plus the geometry CP/M needs.
//!
//! A [`DiskFormat`] captures everything BDOS needs to know about a drive — the
//! Disk Parameter Block it reads, plus the matching geometry the BIOS uses to
//! place sectors. Two presets are provided:
//!
//! * [`DiskFormat::floppy_8inch`] — the canonical CP/M reference drive: an 8"
//!   single-sided single-density floppy, ~241 KB usable.
//! * [`DiskFormat::hard_8mb`] — an 8 MB hard disk, the largest a single CP/M 2.2
//!   logical drive can be (the OS does 16-bit sector math, capping a drive at
//!   ~8 MB). Historically about one partition of a ~$10,000 1980 Corvus
//!   Winchester.
//!
//! Sectors are addressed without skew (the BIOS `SECTRAN` is identity), so the
//! image layout, the DPB, and `SECTRAN` are self-consistent. The crucial rule:
//! the disk and the BIOS must use the **same** `DiskFormat`, or BDOS computes
//! sector addresses the image doesn't agree with. The disk owns its format and
//! the BIOS reads the DPB from it.

use std::collections::HashSet;

/// Bytes per sector (the CP/M record size).
pub const SECTOR_SIZE: usize = 128;
/// Size of a directory entry.
pub const DIR_ENTRY_SIZE: usize = 32;
/// Marks a free/deleted directory entry (and a blank sector).
const EMPTY: u8 = 0xE5;
/// 128-byte records per 16 KB logical extent (one directory entry, since both
/// formats use extent-mask 0).
const RECORDS_PER_EXTENT: usize = 128;

/// DPB for the 8" reference floppy: 26 sectors/track, 1 KB blocks, 243 blocks,
/// 64 directory entries, 2 system tracks. (`SPT BSH BLM EXM DSM DRM AL0 AL1 CKS
/// OFF`, words little-endian.)
pub const DPB_8INCH: [u8; 15] = [
    0x1A, 0x00, // SPT = 26
    0x03, // BSH (1 KB blocks)
    0x07, // BLM
    0x00, // EXM
    0xF2, 0x00, // DSM = 242
    0x3F, 0x00, // DRM = 63
    0xC0, // AL0 (2 directory blocks)
    0x00, // AL1
    0x10, 0x00, // CKS = 16
    0x02, 0x00, // OFF = 2
];

/// DPB for the 8 MB hard disk: 64 sectors/track, 2 KB blocks, 4080 blocks (~8
/// MB), 1024 directory entries, 2 system tracks. Block size 2 KB keeps the
/// extent mask 0 even with 16-bit allocation (8 block pointers x 2 KB = one
/// 16 KB extent). `CKS = 0` marks it a fixed (non-removable) drive.
pub const DPB_8MB: [u8; 15] = [
    0x40, 0x00, // SPT = 64
    0x04, // BSH (2 KB blocks)
    0x0F, // BLM
    0x00, // EXM
    0xEF, 0x0F, // DSM = 4079
    0xFF, 0x03, // DRM = 1023
    0xFF, // AL0 (16 directory blocks)
    0xFF, // AL1
    0x00, 0x00, // CKS = 0 (fixed disk)
    0x02, 0x00, // OFF = 2
];

/// The geometry and parameters of a CP/M drive.
#[derive(Debug, Clone, Copy)]
pub struct DiskFormat {
    /// Logical sectors per track (`SPT`).
    pub sectors_per_track: usize,
    /// Reserved system tracks at the front (`OFF`).
    pub system_tracks: usize,
    /// Allocation block size in bytes (`BLS`).
    pub block_size: usize,
    /// Total allocation blocks (`DSM + 1`).
    pub block_count: usize,
    /// Directory entries (`DRM + 1`).
    pub dir_entries: usize,
    /// Whether allocation block numbers are 16-bit (true once `DSM >= 256`).
    pub wide_alloc: bool,
    /// The 15-byte Disk Parameter Block BDOS reads.
    pub dpb: [u8; 15],
}

impl DiskFormat {
    /// The 8" single-sided single-density reference floppy (~241 KB usable).
    pub fn floppy_8inch() -> Self {
        Self {
            sectors_per_track: 26,
            system_tracks: 2,
            block_size: 1024,
            block_count: 243,
            dir_entries: 64,
            wide_alloc: false,
            dpb: DPB_8INCH,
        }
    }

    /// An 8 MB hard disk — the CP/M 2.2 single-drive maximum.
    pub fn hard_8mb() -> Self {
        Self {
            sectors_per_track: 64,
            system_tracks: 2,
            block_size: 2048,
            block_count: 4080,
            dir_entries: 1024,
            wide_alloc: true,
            dpb: DPB_8MB,
        }
    }

    /// Sectors per allocation block.
    pub fn sectors_per_block(&self) -> usize {
        self.block_size / SECTOR_SIZE
    }
    /// Allocation block numbers stored per directory entry (8 wide / 16 narrow).
    pub fn blocks_per_entry(&self) -> usize {
        if self.wide_alloc { 8 } else { 16 }
    }
    /// Allocation blocks reserved for the directory.
    pub fn dir_blocks(&self) -> usize {
        (self.dir_entries * DIR_ENTRY_SIZE).div_ceil(self.block_size)
    }
    /// Total sectors in the image (system area + data area).
    pub fn total_sectors(&self) -> usize {
        self.system_tracks * self.sectors_per_track + self.block_count * self.sectors_per_block()
    }
    /// Length of the allocation vector BDOS keeps (`(DSM / 8) + 1` bytes).
    pub fn alloc_vector_len(&self) -> usize {
        (self.block_count - 1) / 8 + 1
    }
}

impl Default for DiskFormat {
    fn default() -> Self {
        Self::floppy_8inch()
    }
}

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

/// A CP/M disk: a flat sector image plus the format describing it.
pub struct Disk {
    data: Vec<u8>,
    format: DiskFormat,
}

impl Disk {
    /// A blank 8" floppy (every byte `0xE5`).
    pub fn blank() -> Self {
        Self::blank_with(DiskFormat::floppy_8inch())
    }

    /// A blank disk of the given format.
    pub fn blank_with(format: DiskFormat) -> Self {
        Self {
            data: vec![EMPTY; format.total_sectors() * SECTOR_SIZE],
            format,
        }
    }

    /// A bootable 8" floppy with the CCP+BDOS image in its system tracks.
    pub fn make_system_disk(ccp: &[u8], bdos: &[u8]) -> Self {
        Self::make_system_disk_with(DiskFormat::floppy_8inch(), ccp, bdos)
    }

    /// A bootable disk of the given format with the CCP+BDOS image written
    /// contiguously into the start of the system tracks (where the boot loader
    /// reads it). The data area stays `0xE5`.
    pub fn make_system_disk_with(format: DiskFormat, ccp: &[u8], bdos: &[u8]) -> Self {
        let mut disk = Self::blank_with(format);
        disk.data[..ccp.len()].copy_from_slice(ccp);
        disk.data[ccp.len()..ccp.len() + bdos.len()].copy_from_slice(bdos);
        disk
    }

    /// This disk's format.
    pub fn format(&self) -> &DiskFormat {
        &self.format
    }

    /// Byte offset of a physical (track, sector).
    fn offset(&self, track: usize, sector: usize) -> usize {
        (track * self.format.sectors_per_track + sector) * SECTOR_SIZE
    }

    /// Read one 128-byte sector (`sector` 0-based).
    pub fn read_sector(&self, track: usize, sector: usize) -> &[u8] {
        let o = self.offset(track, sector);
        &self.data[o..o + SECTOR_SIZE]
    }

    /// Write one 128-byte sector (`buf` must be 128 bytes).
    pub fn write_sector(&mut self, track: usize, sector: usize, buf: &[u8]) {
        let o = self.offset(track, sector);
        self.data[o..o + SECTOR_SIZE].copy_from_slice(buf);
    }

    /// The system area (reserved tracks) the boot loader copies into memory.
    pub fn system_area(&self) -> &[u8] {
        &self.data[..self.format.system_tracks * self.format.sectors_per_track * SECTOR_SIZE]
    }

    // --- directory & allocation -----------------------------------------

    /// Map a data-area sector index (0-based from the first data track) to a
    /// physical (track, sector). BDOS computes file locations the same way.
    fn data_location(&self, data_sector: usize) -> (usize, usize) {
        (
            self.format.system_tracks + data_sector / self.format.sectors_per_track,
            data_sector % self.format.sectors_per_track,
        )
    }

    /// Read directory entry `idx`.
    fn dir_entry(&self, idx: usize) -> [u8; DIR_ENTRY_SIZE] {
        let byte = idx * DIR_ENTRY_SIZE;
        let (t, s) = self.data_location(byte / SECTOR_SIZE);
        let within = byte % SECTOR_SIZE;
        let mut entry = [0u8; DIR_ENTRY_SIZE];
        entry.copy_from_slice(&self.read_sector(t, s)[within..within + DIR_ENTRY_SIZE]);
        entry
    }

    /// Write directory entry `idx`.
    fn set_dir_entry(&mut self, idx: usize, entry: &[u8; DIR_ENTRY_SIZE]) {
        let byte = idx * DIR_ENTRY_SIZE;
        let (t, s) = self.data_location(byte / SECTOR_SIZE);
        let o = self.offset(t, s) + byte % SECTOR_SIZE;
        self.data[o..o + DIR_ENTRY_SIZE].copy_from_slice(entry);
    }

    /// Write up to one block of file data into allocation block `block`.
    fn write_block(&mut self, block: usize, data: &[u8]) {
        let spb = self.format.sectors_per_block();
        for i in 0..spb {
            let (t, s) = self.data_location(block * spb + i);
            let o = self.offset(t, s);
            let mut buf = [0u8; SECTOR_SIZE];
            let start = i * SECTOR_SIZE;
            if start < data.len() {
                let n = (data.len() - start).min(SECTOR_SIZE);
                buf[..n].copy_from_slice(&data[start..start + n]);
            }
            self.data[o..o + SECTOR_SIZE].copy_from_slice(&buf);
        }
    }

    /// Decode the allocation block numbers from a directory entry, honoring the
    /// format's 8-bit / 16-bit width.
    fn entry_blocks(&self, entry: &[u8; DIR_ENTRY_SIZE]) -> Vec<usize> {
        let al = &entry[16..32];
        let mut blocks = Vec::new();
        if self.format.wide_alloc {
            for j in 0..8 {
                let b = u16::from(al[2 * j]) | (u16::from(al[2 * j + 1]) << 8);
                if b != 0 {
                    blocks.push(b as usize);
                }
            }
        } else {
            for &b in al {
                if b != 0 {
                    blocks.push(b as usize);
                }
            }
        }
        blocks
    }

    /// The allocation blocks currently in use by directory entries.
    fn used_blocks(&self) -> HashSet<usize> {
        let mut used = HashSet::new();
        for idx in 0..self.format.dir_entries {
            let e = self.dir_entry(idx);
            if e[0] != EMPTY {
                used.extend(self.entry_blocks(&e));
            }
        }
        used
    }

    /// Index of the first free directory entry, if any.
    fn free_dir_entry(&self) -> Option<usize> {
        (0..self.format.dir_entries).find(|&idx| self.dir_entry(idx)[0] == EMPTY)
    }

    /// Place a file on the disk in user area 0, given an `8.3` name and its raw
    /// contents. This is how the host puts programs and data onto a fresh disk
    /// before booting.
    pub fn add_file(&mut self, filename: &str, data: &[u8]) -> Result<(), DiskError> {
        let (name, ext) = parse_filename(filename)?;
        let fmt = self.format;

        // Allocate data blocks.
        let needed = data.len().div_ceil(fmt.block_size).max(1);
        let mut used = self.used_blocks();
        let mut blocks = Vec::with_capacity(needed);
        let mut b = fmt.dir_blocks();
        while blocks.len() < needed {
            if b >= fmt.block_count {
                return Err(DiskError::DiskFull);
            }
            if used.insert(b) {
                blocks.push(b);
            }
            b += 1;
        }
        for (i, &blk) in blocks.iter().enumerate() {
            let start = i * fmt.block_size;
            self.write_block(blk, &data[start..data.len().min(start + fmt.block_size)]);
        }

        // One directory entry (16 KB extent) per `blocks_per_entry` blocks.
        let bpe = fmt.blocks_per_entry();
        let total_records = data.len().div_ceil(SECTOR_SIZE).max(1);
        let extents = blocks.len().div_ceil(bpe).max(1);
        for ex in 0..extents {
            let idx = self.free_dir_entry().ok_or(DiskError::DirectoryFull)?;
            let mut entry = [0u8; DIR_ENTRY_SIZE];
            entry[1..9].copy_from_slice(&name);
            entry[9..12].copy_from_slice(&ext);
            entry[12] = ex as u8; // EX
            entry[15] = (total_records - ex * RECORDS_PER_EXTENT).min(RECORDS_PER_EXTENT) as u8;
            let base = ex * bpe;
            if fmt.wide_alloc {
                for j in 0..bpe {
                    if let Some(&blk) = blocks.get(base + j) {
                        entry[16 + 2 * j] = blk as u8;
                        entry[16 + 2 * j + 1] = (blk >> 8) as u8;
                    }
                }
            } else {
                for j in 0..bpe {
                    if let Some(&blk) = blocks.get(base + j) {
                        entry[16 + j] = blk as u8;
                    }
                }
            }
            self.set_dir_entry(idx, &entry);
        }
        Ok(())
    }

    /// List the file names on the disk (first extent of each), e.g.
    /// `["HELLO.COM"]`.
    pub fn list_files(&self) -> Vec<String> {
        let mut files = Vec::new();
        for idx in 0..self.format.dir_entries {
            let e = self.dir_entry(idx);
            if e[0] == EMPTY || e[12] != 0 {
                continue; // empty, or a continuation extent
            }
            // High bits of the name/ext bytes are attributes — mask them off.
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

    /// Reconstruct a file from its directory entries the way BDOS does, using
    /// the disk's format (records-per-block and 8/16-bit allocation).
    fn reconstruct(disk: &Disk, name: &[u8; 8], ext: &[u8; 3]) -> Vec<u8> {
        let spb = disk.format.sectors_per_block();
        let mut entries: Vec<_> = (0..disk.format.dir_entries)
            .map(|i| disk.dir_entry(i))
            .filter(|e| e[0] != EMPTY && &e[1..9] == name && &e[9..12] == ext)
            .collect();
        entries.sort_by_key(|e| e[12]); // by extent number
        let mut out = Vec::new();
        for e in &entries {
            let rc = e[15] as usize;
            let alloc = disk.entry_blocks(e);
            for rec in 0..rc {
                let block = alloc[rec / spb];
                let (t, s) = disk.data_location(block * spb + rec % spb);
                out.extend_from_slice(disk.read_sector(t, s));
            }
        }
        out
    }

    #[test]
    fn blank_floppy_is_all_empty_marker() {
        let d = Disk::blank();
        assert_eq!(d.data.len(), d.format.total_sectors() * SECTOR_SIZE);
        assert!(d.data.iter().all(|&b| b == 0xE5));
    }

    #[test]
    fn floppy_and_hard_geometry() {
        let f = DiskFormat::floppy_8inch();
        assert_eq!(f.dir_blocks(), 2);
        assert!(!f.wide_alloc);
        let h = DiskFormat::hard_8mb();
        assert!(h.wide_alloc);
        assert_eq!(h.dir_blocks(), 16); // 1024 entries * 32 / 2048
        assert_eq!(h.blocks_per_entry(), 8);
        // ~8 MB capacity, under the 65,525-sector CP/M limit.
        assert_eq!(h.block_count * h.block_size, 8_355_840);
        assert!(h.total_sectors() < 65_525 + h.system_tracks * h.sectors_per_track);
        // DPB SPT and OFF agree with the geometry.
        assert_eq!(u16::from(h.dpb[0]) as usize, h.sectors_per_track);
        assert_eq!(u16::from(h.dpb[13]) as usize, h.system_tracks);
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
        let e = d.dir_entry(0);
        assert_eq!(e[15], 1); // RC = 1 record
        assert_eq!(e[16], 2); // first allocation block
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

    #[test]
    fn floppy_multi_extent_round_trips() {
        // 20000 bytes spans two directory extents on the 1 KB-block floppy.
        let data: Vec<u8> = (0..20000u32).map(|i| (i % 251) as u8).collect();
        let mut d = Disk::blank();
        d.add_file("BIG.DAT", &data).unwrap();
        let recovered = reconstruct(&d, b"BIG     ", b"DAT");
        assert!(recovered.len() >= data.len());
        assert_eq!(&recovered[..data.len()], &data[..]);
    }

    #[test]
    fn hard_disk_16bit_allocation_round_trips() {
        // A >512 KB file forces allocation blocks past 256, exercising the
        // 16-bit block-number *high byte* — the one thing the software catalog
        // (all in low blocks) never touches.
        let mut d = Disk::blank_with(DiskFormat::hard_8mb());
        let data: Vec<u8> = (0..600_000u32)
            .map(|i| (i.wrapping_mul(2654435761) >> 16) as u8)
            .collect();
        d.add_file("HUGE.DAT", &data).unwrap();

        // It really did use a block number >= 256 (high byte non-zero).
        let max_block = (0..d.format.dir_entries)
            .map(|i| d.dir_entry(i))
            .filter(|e| e[0] != EMPTY)
            .flat_map(|e| d.entry_blocks(&e))
            .max()
            .unwrap();
        assert!(
            max_block >= 256,
            "test did not cross block 256 (max {max_block})"
        );

        let recovered = reconstruct(&d, b"HUGE    ", b"DAT");
        assert!(recovered.len() >= data.len());
        assert_eq!(
            &recovered[..data.len()],
            &data[..],
            "16-bit allocation mismatch"
        );
    }
}
