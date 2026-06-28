// SPDX-License-Identifier: MIT
// Copyright (c) 2026 David Goldfarb <deg@degel.com>

//! Terminal translation: turn a CP/M program's terminal control codes into the
//! ANSI sequences a modern terminal understands.
//!
//! CP/M software drives a specific serial terminal by emitting that terminal's
//! escape codes. Line-oriented programs (Zork, MBASIC) emit none and just print
//! text. Full-screen programs (Ladder, the Turbo Pascal editor, WordStar)
//! position the cursor and clear regions of the screen — and they target an
//! *old* terminal, not today's ANSI/xterm. Passed through raw, those codes are
//! garbage on a modern terminal.
//!
//! [`Terminal`] is a small state machine that rewrites those codes. The default
//! [`TerminalMode::Adm3a`] handles the Lear-Siegler **ADM-3A** (the de-facto
//! CP/M standard, and what the "Microbee" terminal our fetched software targets
//! is compatible with): `ESC = row col` cursor addressing, `^Z` clear-screen,
//! `^^` home, and the `ESC T`/`ESC Y` clear-to-end codes. Anything it does not
//! recognize — including modern `ESC [` (CSI) sequences from a VT100/ANSI
//! program — is passed through untouched, so this mode is a safe superset.
//! [`TerminalMode::Raw`] passes everything through unchanged.

/// Which terminal the guest software is driving.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TerminalMode {
    /// Pass every byte through unchanged.
    Raw,
    /// Translate Lear-Siegler ADM-3A control codes to ANSI.
    #[default]
    Adm3a,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Normal,
    Esc,
    /// Seen `ESC =`; waiting for the row byte.
    CursorRow,
    /// Seen `ESC = row`; waiting for the column byte (row stored).
    CursorCol(u8),
}

/// Translates a guest output byte stream into host terminal bytes.
pub struct Terminal {
    mode: TerminalMode,
    state: State,
}

impl Terminal {
    /// Create a translator for the given mode.
    pub fn new(mode: TerminalMode) -> Self {
        Self {
            mode,
            state: State::Normal,
        }
    }

    /// Feed one guest output byte, appending the host bytes to emit to `out`.
    pub fn feed(&mut self, byte: u8, out: &mut Vec<u8>) {
        if self.mode == TerminalMode::Raw {
            out.push(byte);
            return;
        }
        match self.state {
            State::Normal => self.feed_normal(byte, out),
            State::Esc => self.feed_esc(byte, out),
            State::CursorRow => self.state = State::CursorCol(byte.wrapping_sub(0x20)),
            State::CursorCol(row) => {
                let col = byte.wrapping_sub(0x20);
                // ANSI cursor position is 1-based.
                push_str(out, &format!("\x1b[{};{}H", row as u16 + 1, col as u16 + 1));
                self.state = State::Normal;
            }
        }
    }

    fn feed_normal(&mut self, byte: u8, out: &mut Vec<u8>) {
        match byte {
            0x1B => self.state = State::Esc,        // ESC — start a sequence
            0x1A => push_str(out, "\x1b[2J\x1b[H"), // ^Z — clear screen + home
            0x1E => push_str(out, "\x1b[H"),        // ^^ (RS) — cursor home
            _ => out.push(byte),
        }
    }

    fn feed_esc(&mut self, byte: u8, out: &mut Vec<u8>) {
        match byte {
            b'=' => {
                self.state = State::CursorRow;
                return;
            }
            b'T' => push_str(out, "\x1b[K"), // clear to end of line
            b'Y' => push_str(out, "\x1b[J"), // clear to end of screen
            b'*' | b':' => push_str(out, "\x1b[2J\x1b[H"), // clear screen
            other => {
                // Unknown — pass the ESC and this byte through verbatim. This is
                // what lets modern ESC[ (CSI) sequences survive untouched.
                out.push(0x1B);
                out.push(other);
            }
        }
        self.state = State::Normal;
    }
}

fn push_str(out: &mut Vec<u8>, s: &str) {
    out.extend_from_slice(s.as_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(mode: TerminalMode, input: &[u8]) -> Vec<u8> {
        let mut t = Terminal::new(mode);
        let mut out = Vec::new();
        for &b in input {
            t.feed(b, &mut out);
        }
        out
    }

    #[test]
    fn raw_passes_everything() {
        let input = b"\x1b=ABhello\x1a";
        assert_eq!(run(TerminalMode::Raw, input), input);
    }

    #[test]
    fn plain_text_is_unchanged() {
        assert_eq!(run(TerminalMode::Adm3a, b"Ok\r\n"), b"Ok\r\n");
    }

    #[test]
    fn cursor_addressing_becomes_ansi() {
        // ESC = (row 5 -> 0x25) (col 10 -> 0x2A)  ->  ESC[6;11H  (1-based)
        let out = run(TerminalMode::Adm3a, &[0x1B, b'=', 0x25, 0x2A]);
        assert_eq!(out, b"\x1b[6;11H");
        // Top-left: row 0, col 0 -> ESC[1;1H
        let out = run(TerminalMode::Adm3a, &[0x1B, b'=', 0x20, 0x20]);
        assert_eq!(out, b"\x1b[1;1H");
    }

    #[test]
    fn clears_translate() {
        assert_eq!(run(TerminalMode::Adm3a, &[0x1A]), b"\x1b[2J\x1b[H"); // clear screen
        assert_eq!(run(TerminalMode::Adm3a, &[0x1E]), b"\x1b[H"); // home
        assert_eq!(run(TerminalMode::Adm3a, &[0x1B, b'T']), b"\x1b[K"); // clear EOL
        assert_eq!(run(TerminalMode::Adm3a, &[0x1B, b'Y']), b"\x1b[J"); // clear EOS
    }

    #[test]
    fn modern_csi_passes_through() {
        // A VT100/ANSI program's ESC[31m must survive intact in Adm3a mode.
        assert_eq!(
            run(TerminalMode::Adm3a, b"\x1b[31mhi\x1b[0m"),
            b"\x1b[31mhi\x1b[0m"
        );
    }
}
