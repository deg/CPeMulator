// SPDX-License-Identifier: MIT
// Copyright (c) 2026 David Goldfarb <deg@degel.com>

//! An embeddable CP/M session that never blocks: the engine of the browser build.
//!
//! The CLI owns the host thread. It can block in a read, or sleep when CP/M sits
//! idle at its prompt. A web page cannot do either: JavaScript runs one task at a
//! time, so a call that blocks freezes the tab, and nothing would ever deliver the
//! keystroke it is waiting for. A [`Session`] therefore turns the run loop inside
//! out. The host calls [`Session::run`] with a budget of CPU cycles, about one
//! animation frame's worth, and gets control back when either:
//!
//! * the budget is spent, so the host draws a frame and calls `run` again;
//! * CP/M is waiting for a key that has not been typed ([`RunState::Idle`]), so
//!   the host can stop calling `run` until the user types something; or
//! * the machine stopped ([`RunState::Stopped`] and friends).
//!
//! This works because the BIOS `CONIN` never blocks either. With no key waiting,
//! it jumps back to its own entry point instead of returning (see
//! [`crate::bios`]), so a program waiting for input is just a CPU spinning on
//! one address, which this loop can notice and stop on.
//!
//! Keys go in with [`Session::type_bytes`] and screen output comes back from
//! [`Session::take_output`]. Both are bytes as a terminal sees them: input
//! passes through the same [`crate::keys`] translation as the CLI (Delete
//! becomes Backspace, optionally arrows become WordStar keys), and output
//! through the same [`crate::terminal`] TVI-950 to ANSI translation, which any
//! ANSI terminal widget (xterm.js in the browser) can display.
//!
//! The module is plain Rust with no browser dependency, so native `cargo test`
//! covers it; the `cpemu-web` crate is only a thin JavaScript wrapper around it.

use crate::bios::{Console, CpmBios, cold_boot_bios, work_area_bytes};
use crate::config::MachineConfig;
use crate::disk::Disk;
use crate::executor::{Executor, RunExit};
use crate::keys::{KeyXlate, map_key};
use crate::terminal::{Terminal, TerminalMode};
use std::collections::VecDeque;

/// Options for a [`Session`]'s console, the same as the CLI's flags of the same
/// names.
#[derive(Debug, Clone, Copy)]
pub struct ConsoleOptions {
    /// How guest output is translated (`--raw` selects [`TerminalMode::Raw`]).
    pub mode: TerminalMode,
    /// Fold typed lowercase letters to uppercase (`--upper`).
    pub upper: bool,
    /// Turn the Delete key (`0x7F`) into Backspace (on unless `--no-map-delete`).
    pub map_delete: bool,
    /// Map the arrow keys to WordStar's cursor diamond (`--arrows`).
    pub ws_keys: bool,
}

impl Default for ConsoleOptions {
    fn default() -> Self {
        Self {
            mode: TerminalMode::Televideo,
            upper: false,
            map_delete: true,
            ws_keys: false,
        }
    }
}

/// A console backed by two buffers: keys the host has delivered, and output the
/// host has not collected yet. It is never at end of input, because a person
/// may always type more.
pub struct BufferConsole {
    input: VecDeque<u8>,
    output: Vec<u8>,
    term: Terminal,
    keys: KeyXlate,
    upper: bool,
    map_delete: bool,
}

impl BufferConsole {
    /// An empty console with the given options.
    pub fn new(opts: ConsoleOptions) -> Self {
        Self {
            input: VecDeque::new(),
            output: Vec::new(),
            term: Terminal::new(opts.mode),
            keys: KeyXlate::new(opts.ws_keys),
            upper: opts.upper,
            map_delete: opts.map_delete,
        }
    }

    /// Queue bytes as a terminal sends them when keys are pressed.
    pub fn type_bytes(&mut self, bytes: &[u8]) {
        let mut out = Vec::new();
        for &b in bytes {
            out.clear();
            self.keys.feed(b, &mut out);
            self.input
                .extend(out.iter().map(|&k| map_key(k, self.upper, self.map_delete)));
        }
    }

    /// Take everything written since the last call, already translated for an
    /// ANSI terminal.
    pub fn take_output(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.output)
    }
}

impl Console for BufferConsole {
    fn read(&mut self) -> Option<u8> {
        // The BIOS calls this only after `status` said a key is ready, so the
        // queue is never empty here. If it were, there is no way to wait for a
        // key inside a single call, so report "no character" rather than block.
        self.input.pop_front()
    }

    fn status(&mut self) -> bool {
        !self.input.is_empty()
    }

    fn write(&mut self, byte: u8) {
        self.term.feed(byte, &mut self.output);
    }
}

/// Why [`Session::run`] returned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunState {
    /// The cycle budget was spent; call `run` again next frame.
    Running,
    /// CP/M is waiting for a key and none is queued. Nothing changes until the
    /// host calls [`Session::type_bytes`].
    Idle,
    /// The CPU executed `HALT` with no interrupt to wake it.
    Halted,
    /// The BIOS stopped the machine.
    Stopped,
    /// The CPU met an opcode the core does not implement.
    Illegal(u8),
}

/// Why a [`Session`] could not be built.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionError {
    /// The drives' BIOS work areas (allocation vectors and so on) do not fit in
    /// the memory above the BIOS jump table.
    WorkAreaTooLarge {
        /// Bytes the mounted drives need.
        needed: usize,
        /// Bytes available.
        capacity: usize,
    },
}

/// A booted CP/M machine with a buffered console, run in slices by its host.
pub struct Session {
    exec: Executor<CpmBios<BufferConsole>>,
    conin: u16,
}

impl Session {
    /// Cold-boot CP/M on the virtual machine with `disk_a` as drive A: and
    /// `extras` as further drives (`(drive number, disk)`, 1 = B:). The first
    /// call to [`Session::run`] loads the CCP and BDOS off drive A's system
    /// tracks, exactly as the CLI does.
    pub fn new(
        disk_a: Disk,
        extras: Vec<(u8, Disk)>,
        opts: ConsoleOptions,
    ) -> Result<Self, SessionError> {
        let config = MachineConfig::virtual_default();
        let needed = work_area_bytes(std::iter::once(&disk_a).chain(extras.iter().map(|(_, d)| d)));
        let capacity = config.memory_size - (config.bios_base as usize + 0x0100);
        if needed > capacity {
            return Err(SessionError::WorkAreaTooLarge { needed, capacity });
        }
        let mut bios = CpmBios::new(config, BufferConsole::new(opts), disk_a);
        for (drive, disk) in extras {
            bios.mount(drive, disk);
        }
        Ok(Self {
            exec: cold_boot_bios(config, bios),
            conin: config.bios_vector(3),
        })
    }

    /// Queue typed keys (bytes as a terminal sends them).
    pub fn type_bytes(&mut self, bytes: &[u8]) {
        self.exec.bios.console.type_bytes(bytes);
    }

    /// Take the screen output produced since the last call (ANSI bytes).
    pub fn take_output(&mut self) -> Vec<u8> {
        self.exec.bios.console.take_output()
    }

    /// Run until about `budget` T-states have elapsed, CP/M is idle at its
    /// prompt, or the machine stops. The budget may be overshot by part of one
    /// instruction.
    ///
    /// Pacing is the host's job: at a 4 MHz clock and 60 frames a second, a
    /// frame is worth about 66,667 T-states.
    pub fn run(&mut self, budget: u64) -> RunState {
        let end = self.exec.cpu.cycles.saturating_add(budget);
        while self.exec.cpu.cycles < end {
            if self.is_idle() {
                return RunState::Idle;
            }
            match self.exec.step() {
                None => {}
                Some(RunExit::Halted) => return RunState::Halted,
                Some(RunExit::Stopped) => return RunState::Stopped,
                Some(RunExit::Illegal(op)) => return RunState::Illegal(op),
            }
        }
        RunState::Running
    }

    /// Whether CP/M is parked in `CONIN` with no key queued.
    pub fn is_idle(&mut self) -> bool {
        self.exec.cpu.regs.pc == self.conin && !self.exec.bios.console.status()
    }

    /// Total T-states executed since boot.
    pub fn cycles(&self) -> u64 {
        self.exec.cpu.cycles
    }

    /// The disk mounted as `drive` (0 = A:), if any. Its image is what a host
    /// saves to keep the user's files.
    pub fn disk(&self, drive: u8) -> Option<&Disk> {
        self.exec.bios.disk_at(drive)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cpm22::{BDOS, CCP};

    fn bare_session() -> Session {
        Session::new(
            Disk::make_system_disk(CCP, BDOS),
            vec![],
            ConsoleOptions::default(),
        )
        .expect("one floppy fits")
    }

    /// Run in frame-sized slices until idle, with a generous safety limit.
    fn run_to_idle(s: &mut Session) -> String {
        let mut out = Vec::new();
        for _ in 0..1000 {
            let state = s.run(66_667);
            out.extend(s.take_output());
            match state {
                RunState::Running => {}
                RunState::Idle => return String::from_utf8_lossy(&out).into_owned(),
                other => panic!("machine stopped: {other:?}"),
            }
        }
        panic!(
            "never went idle; output so far: {}",
            String::from_utf8_lossy(&out)
        );
    }

    #[test]
    fn boots_to_the_prompt_and_goes_idle() {
        let mut s = bare_session();
        let out = run_to_idle(&mut s);
        assert!(out.contains("A>"), "no prompt in {out:?}");
        // Idle is stable: running again without input does nothing.
        let before = s.cycles();
        assert_eq!(s.run(66_667), RunState::Idle);
        assert_eq!(s.cycles(), before);
    }

    #[test]
    fn typed_command_runs() {
        let mut s = bare_session();
        run_to_idle(&mut s);
        s.type_bytes(b"dir\r");
        let out = run_to_idle(&mut s);
        assert!(out.contains("NO FILE"), "unexpected DIR output {out:?}");
        assert!(out.trim_end().ends_with("A>"), "no fresh prompt in {out:?}");
    }

    #[test]
    fn budget_bounds_a_slice() {
        // A slice stops near its budget, so a busy program cannot hog the host.
        let mut s = bare_session();
        assert_eq!(s.run(1_000), RunState::Running);
        assert!(
            s.cycles() >= 1_000 && s.cycles() < 1_100,
            "ran {}",
            s.cycles()
        );
    }

    #[test]
    fn delete_key_becomes_backspace() {
        // xterm.js sends 0x7F for Backspace; CP/M erases cleanly only on ^H.
        let mut c = BufferConsole::new(ConsoleOptions::default());
        c.type_bytes(b"a\x7f");
        assert_eq!(c.read(), Some(b'a'));
        assert_eq!(c.read(), Some(0x08));
        assert!(!c.status());
    }

    #[test]
    fn output_is_translated_for_ansi() {
        // TVI-950 clear-to-end-of-line (ESC T) reaches the host as ANSI ESC [ K.
        let mut c = BufferConsole::new(ConsoleOptions::default());
        for &b in b"x\x1bT" {
            c.write(b);
        }
        assert_eq!(c.take_output(), b"x\x1b[K");
        assert!(c.take_output().is_empty());
    }

    #[test]
    fn oversized_drives_are_refused() {
        use crate::disk::DiskFormat;
        let big = || Disk::blank_with(DiskFormat::hard_8mb());
        let extras = (1..16).map(|d| (d, big())).collect();
        let err = Session::new(big(), extras, ConsoleOptions::default());
        assert!(matches!(err, Err(SessionError::WorkAreaTooLarge { .. })));
    }
}
