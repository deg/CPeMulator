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

use crate::bios::Console;
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

/// Transform a typed key on the way to the guest: map Delete to Backspace
/// (`map_delete`) and/or fold lowercase letters to uppercase (`upper`).
fn map_key(b: u8, upper: bool, map_delete: bool) -> u8 {
    let b = if map_delete && b == 0x7F { 0x08 } else { b };
    if upper { b.to_ascii_uppercase() } else { b }
}

/// Maps terminal cursor/navigation-key escape sequences to WordStar's cursor
/// "diamond" control keys, when `--arrows` is on. It is stateful because an
/// arrow key arrives as a multi-byte sequence — `ESC [ A`, or `ESC O A` in an
/// application-cursor-key mode. When off, every byte passes straight through.
///
/// There is no universal CP/M arrow-key convention (the BIOS delivered raw
/// bytes and most terminals lacked or differed on arrow keys), but WordStar's
/// `^E`/`^X`/`^S`/`^D` became the de-facto editor standard — Turbo Pascal's
/// editor and many others copied it — so it is the most-compatible single
/// target. It is opt-in because those codes mean other things outside an editor
/// (`^S` is XOFF; `^X` kills the input line).
struct KeyXlate {
    on: bool,
    state: KeyState,
    /// Parameter bytes of an in-progress `ESC [` sequence (digits, `;`).
    params: Vec<u8>,
}

#[derive(PartialEq)]
enum KeyState {
    Normal,
    Esc,
    /// Inside an `ESC [` / `ESC O` sequence, collecting params until the final.
    Seq,
}

impl KeyXlate {
    fn new(on: bool) -> Self {
        Self {
            on,
            state: KeyState::Normal,
            params: Vec::new(),
        }
    }

    /// Feed one input byte; append the byte(s) to deliver to the guest to `out`.
    fn feed(&mut self, b: u8, out: &mut Vec<u8>) {
        if !self.on {
            out.push(b);
            return;
        }
        match self.state {
            KeyState::Normal => {
                if b == 0x1B {
                    self.state = KeyState::Esc;
                } else {
                    out.push(b);
                }
            }
            KeyState::Esc => {
                if b == b'[' || b == b'O' {
                    self.state = KeyState::Seq;
                    self.params.clear();
                } else {
                    // A lone ESC, or ESC + a non-sequence byte: deliver the ESC,
                    // then handle this byte. WordStar shorthand (ESC then a
                    // letter) lands here and is delivered as the pair, intact.
                    out.push(0x1B);
                    if b == 0x1B {
                        self.state = KeyState::Esc; // ESC ESC: keep one buffered
                    } else {
                        out.push(b);
                        self.state = KeyState::Normal;
                    }
                }
            }
            KeyState::Seq => {
                if (0x40..=0x7E).contains(&b) {
                    if let Some(mapped) = map_key_seq(&self.params, b) {
                        out.push(mapped);
                    }
                    // An unrecognized nav sequence is dropped rather than echoed
                    // as garbage — you opted into key mapping.
                    self.state = KeyState::Normal;
                } else {
                    self.params.push(b);
                }
            }
        }
    }
}

/// Map the final byte of an `ESC [`/`ESC O` sequence (with any numeric params)
/// to a WordStar control key.
fn map_key_seq(params: &[u8], final_byte: u8) -> Option<u8> {
    match final_byte {
        b'A' => Some(0x05),                   // Up    -> ^E
        b'B' => Some(0x18),                   // Down  -> ^X
        b'C' => Some(0x04),                   // Right -> ^D
        b'D' => Some(0x13),                   // Left  -> ^S
        b'~' if params == b"3" => Some(0x7F), // Delete (then mapped per map_delete)
        _ => None, // Home/End/PgUp/PgDn/function keys: leave to the user's keys
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

#[cfg(test)]
mod tests {
    use super::{KeyXlate, map_key};

    fn xlate(on: bool, input: &[u8]) -> Vec<u8> {
        let mut k = KeyXlate::new(on);
        let mut out = Vec::new();
        for &b in input {
            k.feed(b, &mut out);
        }
        out
    }

    #[test]
    fn arrows_off_pass_through() {
        // Default: arrow sequences pass through verbatim (no remapping).
        let input = b"\x1b[A\x1b[Bx";
        assert_eq!(xlate(false, input), input);
    }

    #[test]
    fn arrows_map_to_wordstar_diamond() {
        // ESC[ arrows -> ^E/^X/^D/^S; the SS3 (ESC O) form and Delete map too.
        assert_eq!(xlate(true, b"\x1b[A"), vec![0x05]); // up    -> ^E
        assert_eq!(xlate(true, b"\x1b[B"), vec![0x18]); // down  -> ^X
        assert_eq!(xlate(true, b"\x1b[C"), vec![0x04]); // right -> ^D
        assert_eq!(xlate(true, b"\x1b[D"), vec![0x13]); // left  -> ^S
        assert_eq!(xlate(true, b"\x1bOA"), vec![0x05]); // up, application mode
        assert_eq!(xlate(true, b"\x1b[3~"), vec![0x7F]); // Delete
    }

    #[test]
    fn mapping_leaves_text_ctrl_and_break_alone() {
        // Plain text, control keys, and the monitor break byte (0x1C) are
        // untouched even with mapping on — only ESC sequences are intercepted.
        assert_eq!(xlate(true, b"hi\r\x0b\x1c"), b"hi\r\x0b\x1c");
    }

    #[test]
    fn esc_shorthand_survives_mapping() {
        // WordStar shorthand is ESC then a letter: both reach the guest intact,
        // not swallowed as a cursor sequence.
        assert_eq!(xlate(true, b"\x1bP"), b"\x1bP");
    }

    #[test]
    fn unknown_nav_sequence_is_dropped_not_garbled() {
        // A function key (e.g. ESC[15~) maps to nothing rather than echoing
        // "[15~" into the editor.
        assert_eq!(xlate(true, b"\x1b[15~"), Vec::<u8>::new());
    }

    #[test]
    fn lone_esc_is_buffered_until_the_next_key() {
        // The documented trade-off of in-stream arrow parsing: a lone ESC waits
        // for the next byte, then is delivered once a non-'[' byte follows.
        assert_eq!(xlate(true, b"\x1b"), Vec::<u8>::new()); // buffered
        assert_eq!(xlate(true, b"\x1bX"), b"\x1bX"); // flushed with X
    }

    #[test]
    fn delete_maps_to_backspace_by_default() {
        // Delete (0x7F) -> Backspace (0x08) when mapping is on; untouched off.
        assert_eq!(map_key(0x7F, false, true), 0x08);
        assert_eq!(map_key(0x7F, false, false), 0x7F);
        // A real Backspace is left alone either way.
        assert_eq!(map_key(0x08, false, true), 0x08);
    }

    #[test]
    fn upper_folds_only_letters() {
        assert_eq!(map_key(b'a', true, false), b'A');
        assert_eq!(map_key(b'Z', true, false), b'Z');
        assert_eq!(map_key(b'5', true, false), b'5');
        assert_eq!(map_key(b'a', false, false), b'a');
    }

    #[test]
    fn delete_then_upper_compose() {
        // 0x7F is not a letter, so order doesn't matter; both transforms apply.
        assert_eq!(map_key(0x7F, true, true), 0x08);
        assert_eq!(map_key(b'q', true, true), b'Q');
    }
}
