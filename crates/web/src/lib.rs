// SPDX-License-Identifier: MIT
// Copyright (c) 2026 David Goldfarb <deg@degel.com>

//! CPeMulator in a web browser: the WebAssembly wrapper.
//!
//! A thin layer over [`cpemu::session::Session`], compiled to
//! `wasm32-unknown-unknown` and exposed to JavaScript by `wasm-bindgen`. The
//! page creates an [`Emulator`], which cold-boots genuine CP/M 2.2, and then on
//! every animation frame:
//!
//! 1. passes along any keys the user typed ([`Emulator::type_bytes`]),
//! 2. runs a frame's worth of CPU cycles ([`Emulator::run`]), and
//! 3. writes the screen output ([`Emulator::take_output`]) to its terminal
//!    widget, xterm.js.
//!
//! All the emulation, including the run loop that never blocks, lives in the
//! `cpemu` library and is tested natively. This crate only converts types at the
//! JavaScript boundary. It is a separate crate because the glue
//! `#[wasm_bindgen]` generates uses `unsafe`, which `cpemu` forbids.
//!
//! `web/build.sh` builds it, `web/smoke.mjs` tests the built module in Node, and
//! `web/www/index.html` is the page. See `dev-docs/browser.md`.

mod bundle;
mod unzip;

use cpemu::disk::{Disk, DiskError, DiskFormat};
use cpemu::session::{ConsoleOptions, RunState, Session, SessionError};
use wasm_bindgen::prelude::*;

/// A running CP/M machine with one drive, A:.
#[wasm_bindgen]
pub struct Emulator {
    session: Session,
}

#[wasm_bindgen]
impl Emulator {
    /// Cold-boot CP/M with a fresh 8 MB disk as drive A:, holding the bundled
    /// DRI utilities (PIP, STAT, ED, ASM, ...). Nothing runs until the first
    /// [`run`](Emulator::run).
    #[wasm_bindgen(constructor)]
    pub fn new() -> Result<Emulator, JsError> {
        console_error_panic_hook::set_once();
        let disk = bundle::default_disk();
        let session = Session::new(disk, vec![], ConsoleOptions::default()).map_err(describe)?;
        Ok(Emulator { session })
    }

    /// Cold-boot CP/M from a saved drive-A: image (what
    /// [`diskImage`](Emulator::disk_image) returned, or a `.dsk` file). Its size
    /// picks the format: an 8" floppy or an 8 MB hard disk.
    #[wasm_bindgen(js_name = fromImage)]
    pub fn from_image(image: Vec<u8>) -> Result<Emulator, JsError> {
        console_error_panic_hook::set_once();
        let format = DiskFormat::for_image_size(image.len()).ok_or_else(|| {
            JsError::new(&format!(
                "{} bytes is not a CPeMulator disk image ({} = 8\" floppy, {} = 8 MB hard disk)",
                image.len(),
                DiskFormat::floppy_8inch().image_len(),
                DiskFormat::hard_8mb().image_len(),
            ))
        })?;
        let disk = Disk::from_image(format, image).map_err(|e| describe(SessionError::Disk(e)))?;
        let session = Session::new(disk, vec![], ConsoleOptions::default()).map_err(describe)?;
        Ok(Emulator { session })
    }

    /// Put a file on drive A: (replacing one of the same name) and warm-boot,
    /// which ends any running program, so CP/M sees it. `name` must be a CP/M
    /// `8.3` name.
    #[wasm_bindgen(js_name = putFile)]
    pub fn put_file(&mut self, name: &str, bytes: Vec<u8>) -> Result<(), JsError> {
        self.session.put_file(0, name, bytes).map_err(describe)
    }

    /// Put the files in a `.zip` archive on drive A: (folders flattened), then
    /// warm-boot as [`putFile`](Emulator::put_file) does. `only` and `exclude`
    /// are an archive's rules from the software library (see
    /// `unzip::Filter`); pass empty arrays to keep everything. Files that can't
    /// go on the disk are reported, not fatal; only an archive that can't be
    /// read at all is an error.
    #[wasm_bindgen(js_name = putZip)]
    pub fn put_zip(
        &mut self,
        zip: &[u8],
        only: Vec<String>,
        exclude: Vec<String>,
    ) -> Result<ZipReport, JsError> {
        let filter = unzip::Filter { only, exclude };
        let members = unzip::members(zip).map_err(|e| JsError::new(&e))?;
        let mut report = ZipReport::default();
        for m in members {
            if !filter.keeps(&m.path, &m.name) {
                report.left_out += 1;
                continue;
            }
            let result = m
                .data
                .and_then(|data| self.session.put_file(0, &m.name, data).map_err(message));
            match result {
                Ok(()) => report.put.push(m.name.to_ascii_uppercase()),
                Err(why) => report.skipped.push(format!("{}: {why}", m.name)),
            }
        }
        Ok(report)
    }

    /// The paths of the files in a `.zip` (folders included in each path), so
    /// the page can tell which library program an archive holds before
    /// unpacking it.
    #[wasm_bindgen(js_name = zipPaths)]
    pub fn zip_paths(zip: &[u8]) -> Result<Vec<String>, JsError> {
        unzip::paths(zip).map_err(|e| JsError::new(&e))
    }

    /// Drive A:'s whole sector image, to save in the browser or download.
    #[wasm_bindgen(js_name = diskImage)]
    pub fn disk_image(&self) -> Vec<u8> {
        self.session
            .disk(0)
            .map(|d| d.image().to_vec())
            .unwrap_or_default()
    }

    /// A count that grows whenever drive A: is written. Save the image again
    /// only when it differs from the count at the last save.
    #[wasm_bindgen(js_name = diskChanges)]
    pub fn disk_changes(&self) -> f64 {
        self.session.disk(0).map_or(0.0, |d| d.changes() as f64)
    }

    /// The contents of a file on drive A:, or `undefined` if there is none.
    /// CP/M stores sizes in 128-byte records, so a text file comes back padded
    /// with `^Z`.
    #[wasm_bindgen(js_name = readFile)]
    pub fn read_file(&self, name: &str) -> Option<Vec<u8>> {
        self.session.disk(0)?.read_file(name)
    }

    /// The names of the files on drive A:.
    pub fn files(&self) -> Vec<String> {
        self.session
            .disk(0)
            .map(Disk::list_files)
            .unwrap_or_default()
    }

    /// Whether a program is running, rather than CP/M's `A>` prompt. Ask
    /// before an upload or reset, which would end it and lose what it hasn't
    /// saved.
    #[wasm_bindgen(js_name = inProgram)]
    pub fn in_program(&self) -> bool {
        self.session.in_program()
    }

    /// Warm-boot CP/M (like `^C` at the prompt), abandoning any running program.
    #[wasm_bindgen(js_name = warmBoot)]
    pub fn warm_boot(&mut self) {
        self.session.warm_boot();
    }

    /// Queue keys, as the bytes a terminal sends (xterm.js `onData` text,
    /// encoded as UTF-8).
    #[wasm_bindgen(js_name = typeBytes)]
    pub fn type_bytes(&mut self, bytes: &[u8]) {
        self.session.type_bytes(bytes);
    }

    /// Run about `budget` T-states (4 MHz for one 60 Hz frame is about 66,667).
    /// Returns `"running"` (budget spent), `"idle"` (waiting for a key),
    /// `"halted"`, `"stopped"`, or `"illegal XX"` (an unimplemented opcode, in
    /// hex). After `"idle"` there is nothing to do until a key is typed.
    pub fn run(&mut self, budget: u32) -> String {
        match self.session.run(u64::from(budget)) {
            RunState::Running => "running".into(),
            RunState::Idle => "idle".into(),
            RunState::Halted => "halted".into(),
            RunState::Stopped => "stopped".into(),
            RunState::Illegal(op) => format!("illegal {op:02X}"),
        }
    }

    /// Screen output produced since the last call: bytes for an ANSI terminal.
    #[wasm_bindgen(js_name = takeOutput)]
    pub fn take_output(&mut self) -> Vec<u8> {
        self.session.take_output()
    }

    /// T-states executed since boot. A `Number` holds these exactly for about
    /// 70 years at 4 MHz.
    pub fn cycles(&self) -> f64 {
        self.session.cycles() as f64
    }
}

/// What [`Emulator::put_zip`] did with an archive's files.
#[wasm_bindgen]
#[derive(Default)]
pub struct ZipReport {
    put: Vec<String>,
    skipped: Vec<String>,
    left_out: u32,
}

#[wasm_bindgen]
impl ZipReport {
    /// The names of the files put on drive A:.
    #[wasm_bindgen(getter)]
    pub fn put(&self) -> Vec<String> {
        self.put.clone()
    }

    /// One line per file that couldn't go on the disk: its name and why.
    #[wasm_bindgen(getter)]
    pub fn skipped(&self) -> Vec<String> {
        self.skipped.clone()
    }

    /// How many files the archive's rules left off on purpose (manuals,
    /// source, other builds).
    #[wasm_bindgen(getter, js_name = leftOut)]
    pub fn left_out(&self) -> u32 {
        self.left_out
    }
}

fn describe(e: SessionError) -> JsError {
    JsError::new(&message(e))
}

fn message(e: SessionError) -> String {
    match e {
        SessionError::WorkAreaTooLarge { needed, capacity } => format!(
            "the drives need {needed} bytes of BIOS work area but only {capacity} are available"
        ),
        SessionError::NoDrive(d) => format!("no disk in drive {}:", char::from(b'A' + d)),
        SessionError::Disk(DiskError::BadName(n)) => {
            format!("\"{n}\" is not a CP/M file name (up to 8 letters, a dot, up to 3)")
        }
        SessionError::Disk(DiskError::DirectoryFull) => "the disk's directory is full".into(),
        SessionError::Disk(DiskError::DiskFull) => "the disk is full".into(),
        SessionError::Disk(DiskError::BadImageSize { got, want }) => {
            format!("a {got}-byte image does not match its format ({want} bytes)")
        }
    }
}
