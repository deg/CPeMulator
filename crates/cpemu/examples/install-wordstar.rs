// SPDX-License-Identifier: MIT
// Copyright (c) 2026 David Goldfarb <deg@degel.com>

//! Install the fetched WordStar 4 for the ANSI/VT-100 terminal, in place.
//!
//! The WordStar binaries are NOT in this repo — `scripts/fetch-software.sh`
//! downloads the generic edition from the Internet Archive. That edition ships
//! *uninstalled* (no terminal configured), so its full-screen editor can't
//! position the cursor. Rather than commit a pre-installed (proprietary) binary,
//! this tool reproduces the install from the original files: it boots WordStar's
//! own `WINSTALL` inside the emulator and drives it to install `WS.COM` for
//! "ANSI Standard (DEC VT-100)" — whose plain `ESC[` codes pass straight through
//! cpemu's terminal translation — then extracts the patched `WS.COM` (via
//! [`Disk::read_file`]) and writes it back over the downloaded one.
//!
//! Reproducible from the original online files plus this code; nothing
//! proprietary is committed. Run via `make wordstar`, or directly:
//!   `cargo run --release -p cpemu --example install-wordstar -- [DIR]`
//! where DIR (default `software`) holds the fetched WordStar files.

use cpemu::bios::{Console, cold_boot};
use cpemu::config::MachineConfig;
use cpemu::cpm22::{BDOS, CCP};
use cpemu::disk::{Disk, DiskFormat};
use cpemu::executor::RunExit;
use std::collections::VecDeque;
use std::path::Path;
use std::process::ExitCode;

/// The WINSTALL keystrokes that install WS for ANSI: install file `WS`, save to
/// `WS`, then Console menu (A) -> Monitor selection (A) -> "ANSI Standard (DEC
/// VT-100)" (C) -> done (X) -> done (X) -> confirm save (Y).
const INSTALL_SEQ: &str = "WINSTALL\rWS\rWS\rAACXXY";

/// Files WINSTALL needs on the disk (loaded if present; WS.COM/WS.OVR/WINSTALL
/// are the essentials).
const WS_FILES: &[&str] = &[
    "WS.COM",
    "WS.OVR",
    "WSMSGS.OVR",
    "WSHELP.OVR",
    "WSPRINT.OVR",
    "WSSHORT.OVR",
    "WINSTALL.COM",
];

/// Once the guest has gone quiet (no output for `IDLE_THRESHOLD` steps) it has
/// drawn a prompt and run its type-ahead flush, so it is now blocking for a
/// genuine keystroke — only then do we release the next key. One key per idle
/// window defeats WINSTALL's flush (a batch of keys would be discarded).
const IDLE_THRESHOLD: u64 = 400_000;

/// Releases the install keystrokes one per idle window; never signals EOF (so a
/// blocking read just retries rather than ending the run).
struct GatedConsole {
    keys: VecDeque<u8>,
    idle: u64,
}

impl Console for GatedConsole {
    fn read(&mut self) -> Option<u8> {
        if self.idle <= IDLE_THRESHOLD {
            return None; // still settling: make a blocking read retry
        }
        let key = self.keys.pop_front()?;
        self.idle = 0;
        Some(key)
    }
    fn status(&mut self) -> bool {
        self.idle > IDLE_THRESHOLD && !self.keys.is_empty()
    }
    fn at_eof(&self) -> bool {
        false
    }
    fn write(&mut self, _byte: u8) {
        self.idle = 0; // any output means the guest is busy, not waiting
    }
}

fn main() -> ExitCode {
    let dir = std::env::args().nth(1).unwrap_or_else(|| "software".into());
    let dir = Path::new(&dir);
    let ws_com = dir.join("WS.COM");
    if !ws_com.exists() {
        eprintln!(
            "install-wordstar: {} not found — run scripts/fetch-software.sh first",
            ws_com.display()
        );
        return ExitCode::FAILURE;
    }

    // Boot a scratch disk holding WordStar + WINSTALL, then drive the install.
    let mut disk = Disk::make_system_disk_with(DiskFormat::hard_8mb(), CCP, BDOS);
    for f in WS_FILES {
        if let Ok(bytes) = std::fs::read(dir.join(f)) {
            disk.add_file(f, &bytes)
                .expect("WordStar file fits on the disk");
        }
    }
    let console = GatedConsole {
        keys: INSTALL_SEQ.bytes().collect(),
        idle: 0,
    };
    let mut exec = cold_boot(MachineConfig::virtual_default(), disk, console);

    // Step until every keystroke is consumed and the screen has settled — or a
    // generous cap, so a broken install can't spin forever.
    let mut steps = 0u64;
    while steps < 4_000_000_000 {
        if matches!(exec.step(), Some(RunExit::Halted | RunExit::Stopped)) {
            break;
        }
        exec.bios.console.idle += 1;
        if exec.bios.console.keys.is_empty() && exec.bios.console.idle > 5_000_000 {
            break;
        }
        steps += 1;
    }

    // Verify against the actual patched binary, not the screen: it must now
    // carry the ANSI terminal name and have shed the "uninstalled" marker.
    let Some(bytes) = exec.bios.disk().read_file("WS.COM") else {
        eprintln!("install-wordstar: WS.COM vanished from the disk after install");
        return ExitCode::FAILURE;
    };
    let installed = bytes.windows(13).any(|w| w == b"ANSI Standard");
    let still_uninstalled = bytes.windows(20).any(|w| w == b"Uninstalled Terminal");
    if !installed || still_uninstalled {
        eprintln!(
            "install-wordstar: WINSTALL did not produce an ANSI-installed WS.COM \
             (its menus may have changed); leaving {} untouched",
            ws_com.display()
        );
        return ExitCode::FAILURE;
    }

    if let Err(e) = std::fs::write(&ws_com, &bytes) {
        eprintln!(
            "install-wordstar: could not write {}: {e}",
            ws_com.display()
        );
        return ExitCode::FAILURE;
    }
    println!(
        "install-wordstar: installed WordStar for ANSI/VT-100 -> {} ({} bytes)",
        ws_com.display(),
        bytes.len()
    );
    ExitCode::SUCCESS
}
