//! An interactive console: the host terminal as the CP/M console.
//!
//! CP/M expects a character-at-a-time serial terminal — no line buffering, and
//! the OS does its own echoing — so the host terminal is put into **raw mode**.
//! A background thread reads raw bytes from stdin into a channel; this lets
//! `CONIN` block for a key while `CONST` (which asks "is a key waiting?") can
//! answer without blocking by peeking at the channel.
//!
//! Raw mode is restored automatically when the console is dropped. Press
//! **Ctrl-\\** to leave the emulator (it closes the input stream, which the BIOS
//! sees as end of input).

use crate::bios::Console;
use crossterm::terminal;
use std::io::{self, Read, Write};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;

/// The byte that quits the emulator (Ctrl-\, the file separator — effectively
/// unused by CP/M software).
const QUIT: u8 = 0x1C;

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
    _raw: RawGuard,
}

impl TerminalConsole {
    /// Put the terminal into raw mode and start reading keystrokes. Fails if
    /// there is no interactive terminal to control.
    pub fn new() -> io::Result<Self> {
        let raw = RawGuard::enable()?;
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let stdin = io::stdin();
            for byte in stdin.lock().bytes() {
                match byte {
                    Ok(QUIT) | Err(_) => break, // quit key or stream closed
                    Ok(b) => {
                        if tx.send(b).is_err() {
                            break; // the console was dropped
                        }
                    }
                }
            }
            // Dropping `tx` closes the channel, which CONIN reports as EOF.
        });
        Ok(Self {
            rx,
            pending: None,
            _raw: raw,
        })
    }
}

impl Console for TerminalConsole {
    fn read(&mut self) -> Option<u8> {
        if let Some(b) = self.pending.take() {
            return Some(b);
        }
        self.rx.recv().ok() // blocks until a key, None when input closes
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
        let mut out = io::stdout();
        let _ = out.write_all(&[byte]);
        let _ = out.flush();
    }
}
