// SPDX-License-Identifier: MIT
// Copyright (c) 2026 David Goldfarb <deg@degel.com>

//! Reading the files out of a `.zip` archive, so a visitor can upload CP/M
//! software as it is usually distributed.
//!
//! The `zip` crate does the reading, built with only the compression methods
//! CP/M archives use: deflate, and the legacy PKZIP 1.x methods (Shrink,
//! Reduce, Implode). Many CP/M archives were zipped in the DOS era with those
//! (Zork's and MBASIC's are imploded), and a modern reader that handles only
//! deflate rejects them.
//!
//! A member that can't be read (encrypted, an unsupported method, damaged) is
//! reported, not fatal: the rest of the archive still goes on the disk.
//! Folders are flattened, since a CP/M disk has none: `GAME/ZORK1.COM` becomes
//! `ZORK1.COM`. Folder entries and macOS's `__MACOSX` metadata are dropped.

use std::io::{Cursor, Read};
use zip::ZipArchive;

/// One file taken out of an archive.
pub struct Member {
    /// Its path inside the archive, folders included.
    pub path: String,
    /// The file's name, without any folder path.
    pub name: String,
    /// Its contents, or why it couldn't be read.
    pub data: Result<Vec<u8>, String>,
}

/// The files in `zip`, in archive order. Fails only if `zip` can't be read as
/// a zip archive at all; a member that can't be read comes back with an error.
pub fn members(zip: &[u8]) -> Result<Vec<Member>, String> {
    let mut archive = ZipArchive::new(Cursor::new(zip)).map_err(|_| "not a zip archive")?;
    let mut out = Vec::new();
    for i in 0..archive.len() {
        // The name comes from the central directory, so it is known even when
        // the member's data can't be read. Old archives store names as bytes
        // in the DOS code page; CP/M names are plain ASCII, so a lenient
        // decode loses nothing.
        let path = match archive.name_for_index(i).and_then(Result::ok) {
            Some(name) => name.into_owned(),
            None => return Err("damaged zip directory".into()),
        };
        let data = match archive.by_index(i) {
            Ok(file) if file.is_dir() => continue,
            Ok(mut file) => {
                let mut data = Vec::new();
                file.read_to_end(&mut data)
                    .map(|_| data)
                    .map_err(|e| e.to_string())
            }
            Err(e) => Err(e.to_string()),
        };
        if path.ends_with('/') || path.starts_with("__MACOSX/") {
            continue;
        }
        let name = path.rsplit('/').next().unwrap_or(&path).to_string();
        out.push(Member { path, name, data });
    }
    Ok(out)
}

/// The paths of the files in `zip`, without reading their data: enough to tell
/// which program an archive holds. Folder entries are left out.
pub fn paths(zip: &[u8]) -> Result<Vec<String>, String> {
    let archive = ZipArchive::new(Cursor::new(zip)).map_err(|_| "not a zip archive")?;
    Ok((0..archive.len())
        .filter_map(|i| archive.name_for_index(i).and_then(Result::ok))
        .map(|name| name.into_owned())
        .filter(|path| !path.ends_with('/'))
        .collect())
}

/// Which of an archive's files belong on the disk: the rules the desktop's
/// `scripts/fetch-software.sh` applies to each archive in its library, which
/// the web page's software library repeats (`web/www/library.json`).
/// Matching ignores case, as old archives mix it.
#[derive(Default)]
pub struct Filter {
    /// If not empty, only files whose path contains one of these (folder names
    /// such as `bdsc160/`) are kept. BDS C's archive, for one, holds an 8080
    /// and a Z80 build with the same file names.
    pub only: Vec<String>,
    /// Files never kept: `*.EXT` patterns or exact names. Scanned manuals,
    /// source and docs, and install-time tools you'd never run at `A>`.
    pub exclude: Vec<String>,
}

impl Filter {
    /// Whether the member at `path` (named `name`) belongs on the disk.
    pub fn keeps(&self, path: &str, name: &str) -> bool {
        let path = path.to_ascii_lowercase();
        let name = name.to_ascii_lowercase();
        let wanted = self.only.is_empty()
            || self
                .only
                .iter()
                .any(|o| path.contains(&o.to_ascii_lowercase()));
        let excluded = self.exclude.iter().any(|pattern| {
            let pattern = pattern.to_ascii_lowercase();
            match pattern.strip_prefix('*') {
                Some(suffix) => name.ends_with(suffix),
                None => name == pattern,
            }
        });
        wanted && !excluded
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Made by Python's zipfile: a folder entry, GAME/HELLO.COM (deflated),
    /// readme.txt (stored), "bad name.txt", and a __MACOSX metadata file.
    const SAMPLE: &[u8] = include_bytes!("../tests/fixtures/sample.zip");

    #[test]
    fn reads_stored_and_deflated_members_and_flattens_folders() {
        let m = members(SAMPLE).unwrap();
        let names: Vec<&str> = m.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(names, ["HELLO.COM", "readme.txt", "bad name.txt"]);
        assert_eq!(m[0].path, "GAME/HELLO.COM");
        let hello = m[0].data.as_ref().unwrap();
        assert!(hello.ends_with(b"HELLO FROM A ZIP$"));
        let readme = m[1].data.as_ref().unwrap();
        assert_eq!(readme.len(), 43 * 40);
    }

    #[test]
    fn filter_keeps_only_listed_folders_and_drops_excluded_files() {
        let f = Filter {
            only: vec!["bdsc160/".into()],
            exclude: vec!["*.pdf".into(), "*.C".into(), "INSTALL.COM".into()],
        };
        assert!(f.keeps("bdsc160/CC.COM", "CC.COM"));
        assert!(f.keeps("BDSC160/cc2.com", "cc2.com")); // case ignored
        assert!(!f.keeps("bdsz20/CC.COM", "CC.COM")); // another folder
        assert!(!f.keeps("bdsc160/guide.PDF", "guide.PDF"));
        assert!(!f.keeps("bdsc160/stdio.c", "stdio.c"));
        assert!(!f.keeps("bdsc160/install.com", "install.com"));
        // An exact name doesn't match as a suffix.
        assert!(f.keeps("bdsc160/REINSTALL.COM", "REINSTALL.COM"));
        // No rules: everything is kept.
        assert!(Filter::default().keeps("any/thing.pdf", "thing.pdf"));
    }

    #[test]
    fn paths_lists_files_without_folders() {
        let p = paths(SAMPLE).unwrap();
        assert_eq!(
            p,
            [
                "GAME/HELLO.COM",
                "readme.txt",
                "bad name.txt",
                "__MACOSX/GAME/._HELLO.COM"
            ]
        );
        assert!(paths(b"not a zip").is_err());
    }

    #[test]
    fn rejects_what_is_not_a_zip() {
        assert!(members(b"MZ not a zip at all").is_err());
        assert!(members(&[]).is_err());
        // Cut off before its end-of-directory record.
        assert!(members(&SAMPLE[..SAMPLE.len() - 30]).is_err());
    }

    #[test]
    fn damaged_member_data_is_reported_not_fatal() {
        let mut z = SAMPLE.to_vec();
        // Corrupt the deflated HELLO.COM's data, just past its local header
        // (signature PK\3\4, fixed fields, then the name and extra field).
        let name = b"GAME/HELLO.COM";
        let lfh = (0..z.len() - 34)
            .find(|&i| z[i..].starts_with(b"PK\x03\x04") && z[i + 30..].starts_with(name))
            .unwrap();
        let extra = usize::from(u16::from_le_bytes([z[lfh + 28], z[lfh + 29]]));
        let start = lfh + 30 + name.len() + extra;
        for b in &mut z[start..start + 8] {
            *b = 0xFF;
        }
        let m = members(&z).unwrap();
        assert!(m[0].data.is_err(), "corruption went unnoticed");
        assert!(m[1].data.is_ok());
    }

    /// The legacy methods, against real archives: `make setup` downloads them
    /// into software/_zips (not committed: they aren't ours to redistribute).
    /// Every member must read cleanly, which includes the crate's CRC-32 check
    /// of the decompressed bytes, and the files whose fetched copy in software/
    /// came from that archive must match it byte for byte. (Others there, like
    /// ZORK3.DAT, were overwritten by another archive's version.) Run with
    /// `cargo test -p cpemu-web -- --ignored`.
    #[test]
    #[ignore = "needs the fetched software library (make setup)"]
    fn legacy_methods_read_real_archives() {
        let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../../software/");
        for (zip, same_as_fetched) in [
            ("zork123_80.zip", &["ZORK1.COM", "ZORK1.DAT"][..]), // implode, shrink
            ("mbasic.zip", &["MBASIC.COM"][..]),                 // implode
        ] {
            let bytes = std::fs::read(format!("{root}_zips/{zip}")).expect("run make setup");
            let m = members(&bytes).unwrap();
            for member in &m {
                assert!(
                    member.data.is_ok(),
                    "{zip}: {}: {:?}",
                    member.name,
                    member.data
                );
            }
            for f in same_as_fetched {
                let got = m
                    .iter()
                    .find(|m| m.name == *f)
                    .unwrap()
                    .data
                    .as_ref()
                    .unwrap();
                let want = std::fs::read(format!("{root}{f}")).unwrap();
                assert!(got == &want, "{zip}: {f} differs from the fetched copy");
            }
        }
    }
}
