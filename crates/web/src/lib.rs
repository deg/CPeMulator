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

use cpemu::session::{ConsoleOptions, RunState, Session, SessionError};
use wasm_bindgen::prelude::*;

/// A running CP/M machine with one drive, A:.
#[wasm_bindgen]
pub struct Emulator {
    session: Session,
}

#[wasm_bindgen]
impl Emulator {
    /// Cold-boot CP/M with a fresh 8" floppy as drive A:, holding the bundled
    /// DRI utilities (PIP, STAT, ED, ASM, ...). Nothing runs until the first
    /// [`run`](Emulator::run).
    #[wasm_bindgen(constructor)]
    pub fn new() -> Result<Emulator, JsError> {
        console_error_panic_hook::set_once();
        let disk = bundle::default_disk();
        let session = Session::new(disk, vec![], ConsoleOptions::default()).map_err(describe)?;
        Ok(Emulator { session })
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

fn describe(e: SessionError) -> JsError {
    match e {
        SessionError::WorkAreaTooLarge { needed, capacity } => JsError::new(&format!(
            "the drives need {needed} bytes of BIOS work area but only {capacity} are available"
        )),
    }
}
