// SPDX-License-Identifier: MIT
// Copyright (c) 2026 David Goldfarb <deg@degel.com>

//! The software on the browser build's default drive A:.
//!
//! Only software we may redistribute: Digital Research's own CP/M 2.2 utilities,
//! under the same grant as the CCP and BDOS. Their provenance and hashes are in
//! `third-party/cpm22/utilities/NOTICE.md`. Anything else (games, compilers,
//! WordStar) the visitor uploads themselves.

use cpemu::cpm22::{BDOS, CCP};
use cpemu::disk::{Disk, terminate_text_file};

macro_rules! utility {
    ($name:literal) => {
        (
            $name,
            include_bytes!(concat!("../../../third-party/cpm22/utilities/", $name)) as &[u8],
        )
    };
}

/// Each bundled file's CP/M name and contents.
pub const FILES: &[(&str, &[u8])] = &[
    utility!("ASM.COM"),
    utility!("DDT.COM"),
    utility!("DUMP.ASM"),
    utility!("DUMP.COM"),
    utility!("ED.COM"),
    utility!("LOAD.COM"),
    utility!("PIP.COM"),
    utility!("STAT.COM"),
    utility!("SUBMIT.COM"),
    utility!("XSUB.COM"),
];

/// A fresh 8" system disk holding the bundled files.
pub fn default_disk() -> Disk {
    let mut disk = Disk::make_system_disk(CCP, BDOS);
    for (name, bytes) in FILES {
        disk.add_file(name, &terminate_text_file(name, bytes.to_vec()))
            .expect("the bundle fits on a floppy");
    }
    disk
}

#[cfg(test)]
mod tests {
    use super::*;
    use cpemu::session::{ConsoleOptions, RunState, Session};

    fn run_to_idle(s: &mut Session) -> String {
        let mut out = Vec::new();
        for _ in 0..10_000 {
            let state = s.run(66_667);
            out.extend(s.take_output());
            match state {
                RunState::Running => {}
                RunState::Idle => return String::from_utf8_lossy(&out).into_owned(),
                other => panic!("machine stopped: {other:?}"),
            }
        }
        panic!("never went idle: {}", String::from_utf8_lossy(&out));
    }

    fn command(s: &mut Session, line: &str) -> String {
        s.type_bytes(format!("{line}\r").as_bytes());
        run_to_idle(s)
    }

    #[test]
    fn the_utilities_work() {
        let mut s = Session::new(default_disk(), vec![], ConsoleOptions::default()).unwrap();
        run_to_idle(&mut s);
        let stat = command(&mut s, "stat");
        assert!(stat.contains("A: R/W, Space:"), "STAT said {stat:?}");
        // DRI's own tutorial path: assemble the sample, load it, run it.
        let asm = command(&mut s, "asm dump");
        assert!(asm.contains("END OF ASSEMBLY"), "ASM said {asm:?}");
        let load = command(&mut s, "load dump");
        assert!(load.contains("FIRST ADDRESS"), "LOAD said {load:?}");
        let dump = command(&mut s, "dump stat.com");
        assert!(dump.contains("0000 "), "DUMP said {dump:?}");
        let pip = command(&mut s, "pip b.com=stat.com");
        assert!(!pip.contains("ERROR"), "PIP said {pip:?}");
        let disk = s.disk(0).unwrap();
        assert_eq!(disk.read_file("B.COM"), disk.read_file("STAT.COM"));
    }
}
