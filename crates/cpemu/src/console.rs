// SPDX-License-Identifier: MIT
// Copyright (c) 2026 David Goldfarb <deg@degel.com>

//! An interactive console: the host terminal as the CP/M console.
//!
//! CP/M expects a character-at-a-time serial terminal — no line buffering, and
//! the OS does its own echoing — so the host terminal is put into **raw mode**.
//! A background thread reads raw bytes from stdin into a channel; this lets
//! `CONIN` block for a key while `CONST` (which asks "is a key waiting?") can
//! answer without blocking by peeking at the channel.
//!
//! Raw mode is restored automatically when the console is dropped. Press
//! **Ctrl-\\** to break into the debug monitor (from which `q` quits); closing
//! the input stream (Ctrl-D / pipe end) is what the BIOS sees as end of input.
//!
//! This module is native-only (it needs crossterm and a reader thread). The key
//! translation it applies to typed bytes lives in [`crate::keys`], shared with
//! the browser console.

use crate::bios::Console;
use crate::keys::{KeyXlate, map_key};
use crate::terminal::{Terminal, TerminalMode};
use crossterm::terminal;
use std::io::{self, Read, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;

/// The byte that breaks into the debug monitor (Ctrl-\, the file separator —
/// effectively unused by CP/M software).
const BREAK: u8 = 0x1C;

/// Restores cooked terminal mode when dropped.
struct RawGuard;

impl RawGuard {
    fn enable() -> io::Result<Self> {
        terminal::enable_raw_mode()?;
        Ok(Self)
    }
}

impl Drop for RawGuard {
    fn drop(&mut self) {
        let _ = terminal::disable_raw_mode();
    }
}

/// The host terminal, presented to CP/M as its console.
pub struct TerminalConsole {
    rx: Receiver<u8>,
    pending: Option<u8>,
    term: Terminal,
    /// Set when the input stream ends (Ctrl-D / pipe close): the run loop treats
    /// this as quit, and CONIN treats it as end of input.
    closed: Arc<AtomicBool>,
    /// Set when the break key (Ctrl-\) is pressed: the run loop drops into the
    /// debug monitor. Cleared when polled.
    brk: Arc<AtomicBool>,
    /// Fold typed lowercase letters to uppercase (for old programs that expect
    /// capitals).
    upper: bool,
    /// Map the host Delete key (`0x7F`) to Backspace (`^H`, `0x08`) so it erases
    /// cleanly instead of triggering CP/M's RUBOUT (which re-echoes the char).
    map_delete: bool,
    /// Optional diagnostic tap: if `CPEMU_TRACE_OUT` names a writable path, every
    /// *raw* guest output byte (before terminal translation) is appended to it.
    /// This is how we capture exactly what a program emits when a display bug is
    /// only reproducible interactively (e.g. a game that needs real-time input).
    trace: Option<std::fs::File>,
    _raw: RawGuard,
}

impl TerminalConsole {
    /// Put the terminal into raw mode and start reading keystrokes, translating
    /// guest output according to `mode`. `upper` folds typed lowercase to
    /// uppercase; `map_delete` turns the Delete key into a clean Backspace;
    /// `ws_keys` maps the arrow keys to WordStar's cursor diamond.
    /// Fails if there is no interactive terminal to control.
    pub fn new(
        mode: TerminalMode,
        upper: bool,
        map_delete: bool,
        ws_keys: bool,
    ) -> io::Result<Self> {
        let raw = RawGuard::enable()?;
        let (tx, rx) = mpsc::channel();
        let closed = Arc::new(AtomicBool::new(false));
        let brk = Arc::new(AtomicBool::new(false));
        let reader_closed = Arc::clone(&closed);
        let reader_brk = Arc::clone(&brk);
        let mut keys = KeyXlate::new(ws_keys);
        thread::spawn(move || {
            let stdin = io::stdin();
            let mut out = Vec::new();
            'reader: for byte in stdin.lock().bytes() {
                let b = match byte {
                    Ok(b) => b,
                    Err(_) => break, // stream error
                };
                // Decode arrow-key sequences into WordStar keys (no-op when off).
                out.clear();
                keys.feed(b, &mut out);
                for &e in &out {
                    // The break key raises the monitor flag but does NOT end the
                    // stream — input keeps flowing after you resume.
                    if e == BREAK {
                        reader_brk.store(true, Ordering::SeqCst);
                    } else if tx.send(e).is_err() {
                        break 'reader; // the console was dropped
                    }
                }
            }
            // The stream ended (EOF/error or the console was dropped): raise the
            // closed flag (polled by the run loop as quit, read by CONIN as EOF)
            // and drop `tx` so a blocked read unblocks.
            reader_closed.store(true, Ordering::SeqCst);
        });
        Ok(Self {
            rx,
            pending: None,
            term: Terminal::new(mode),
            closed,
            brk,
            upper,
            map_delete,
            trace: std::env::var_os("CPEMU_TRACE_OUT").and_then(|p| std::fs::File::create(p).ok()),
            _raw: raw,
        })
    }
}

impl Console for TerminalConsole {
    fn read(&mut self) -> Option<u8> {
        let b = match self.pending.take() {
            Some(b) => b,
            None => self.rx.recv().ok()?, // blocks until a key, None when closed
        };
        Some(map_key(b, self.upper, self.map_delete))
    }

    fn status(&mut self) -> bool {
        if self.pending.is_some() {
            return true;
        }
        match self.rx.try_recv() {
            Ok(b) => {
                self.pending = Some(b);
                true
            }
            Err(TryRecvError::Empty | TryRecvError::Disconnected) => false,
        }
    }

    fn write(&mut self, byte: u8) {
        if let Some(f) = &mut self.trace {
            let _ = f.write_all(&[byte]);
        }
        let mut buf = Vec::new();
        self.term.feed(byte, &mut buf);
        if !buf.is_empty() {
            let mut out = io::stdout();
            let _ = out.write_all(&buf);
            let _ = out.flush();
        }
    }

    fn wants_quit(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    fn at_eof(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    fn wants_break(&mut self) -> bool {
        self.brk.swap(false, Ordering::SeqCst)
    }
}
