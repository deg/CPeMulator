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
//! [`TerminalMode::Televideo`] emulates the **TeleVideo TVI-950** — one of the
//! best-selling CP/M-era terminals, and the one our fetched software actually
//! targets (some of it labels that terminal "Microbee VDU", a TeleVideo-950-
//! compatible screen, which is why it speaks these exact codes). It also accepts
//! the **Lear-Siegler ADM-3A**'s `^Z` clear and `^^` home — the ADM-3A and
//! TeleVideo share the `ESC = row col` cursor-addressing lineage, so handling
//! both costs nothing and covers more software. The TVI-950 codes translated:
//!
//! | Code | TVI-950 function | ANSI |
//! |------|------------------|------|
//! | `ESC = r c` | position cursor (row, col; `+0x20` bias) | `ESC [ r ; c H` |
//! | `ESC ;` `ESC :` `ESC *` | clear (unprotected) page | `ESC [ 2J` + home |
//! | `ESC T` | clear to end of line | `ESC [ K` |
//! | `ESC Y` | clear to end of page | `ESC [ J` |
//! | `ESC E` | insert line at cursor | `ESC [ L` |
//! | `ESC R` | delete line at cursor | `ESC [ M` |
//! | `ESC )` / `ESC (` | half-intensity on / off | `ESC [ 2m` / `ESC [ 22m` |
//!
//! Anything it does not recognize — including modern `ESC [` (CSI) sequences
//! from a VT100/ANSI program — is passed through untouched, so software
//! configured for ANSI/VT100 (the cleanest target on a modern host) renders
//! with no translation at all. [`TerminalMode::Raw`] passes everything through.

/// Which terminal the guest software is driving.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TerminalMode {
    /// Pass every byte through unchanged.
    Raw,
    /// Translate TeleVideo TVI-950 (and ADM-3A `^Z`/`^^`) control codes to ANSI.
    #[default]
    Televideo,
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
            0x1A => push_str(out, "\x1b[2J\x1b[H"), // ^Z — ADM-3A clear screen + home
            0x1E => push_str(out, "\x1b[H"),        // ^^ (RS) — ADM-3A cursor home
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
            b'Y' => push_str(out, "\x1b[J"), // clear to end of page
            // Clear-screen variants. The ADM-3A clears with `^Z` (in
            // `feed_normal`); the TVI-950 has several ESC forms — `;`/`:` clear
            // the *unprotected* page, `*` the whole page — which on our
            // unprotected, attribute-light screen all mean the same thing.
            // PACMAN95 redraws each frame with `ESC ;` then `ESC =` (home).
            b'*' | b':' | b';' => push_str(out, "\x1b[2J\x1b[H"),
            b'E' => push_str(out, "\x1b[L"), // insert line at cursor
            b'R' => push_str(out, "\x1b[M"), // delete line at cursor
            // TVI-950 half-intensity attribute (Turbo Pascal dims text with it;
            // `)` turns it ON, `(` OFF — it is not a highlight pair). These MUST
            // be translated, not passed through: on a modern terminal `ESC (` /
            // `ESC )` are charset-designation lead-ins that swallow the next byte
            // — which is what once ate the `B` in Turbo's "BORLAND".
            b')' => push_str(out, "\x1b[2m"), // half-intensity on -> faint
            b'(' => push_str(out, "\x1b[22m"), // half-intensity off -> normal
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
        assert_eq!(run(TerminalMode::Televideo, b"Ok\r\n"), b"Ok\r\n");
    }

    #[test]
    fn cursor_addressing_becomes_ansi() {
        // ESC = (row 5 -> 0x25) (col 10 -> 0x2A)  ->  ESC[6;11H  (1-based)
        let out = run(TerminalMode::Televideo, &[0x1B, b'=', 0x25, 0x2A]);
        assert_eq!(out, b"\x1b[6;11H");
        // Top-left: row 0, col 0 -> ESC[1;1H
        let out = run(TerminalMode::Televideo, &[0x1B, b'=', 0x20, 0x20]);
        assert_eq!(out, b"\x1b[1;1H");
    }

    #[test]
    fn clears_translate() {
        assert_eq!(run(TerminalMode::Televideo, &[0x1A]), b"\x1b[2J\x1b[H"); // ^Z clear
        assert_eq!(run(TerminalMode::Televideo, &[0x1E]), b"\x1b[H"); // ^^ home
        assert_eq!(run(TerminalMode::Televideo, &[0x1B, b'T']), b"\x1b[K"); // clear EOL
        assert_eq!(run(TerminalMode::Televideo, &[0x1B, b'Y']), b"\x1b[J"); // clear EOP
        // The TVI-950 clear-page forms `;` `:` `*` all map to clear + home. The
        // game PACMAN95 (hardcoded, can't be reconfigured) redraws with `ESC ;`;
        // without translating it, output overdrew the old screen.
        for c in *b";:*" {
            assert_eq!(run(TerminalMode::Televideo, &[0x1B, c]), b"\x1b[2J\x1b[H");
        }
    }

    #[test]
    fn line_insert_delete_translate() {
        // TVI-950 ESC E / ESC R are line insert/delete (the Turbo Pascal editor
        // uses them); they map to the ANSI IL/DL sequences.
        assert_eq!(run(TerminalMode::Televideo, b"\x1bE"), b"\x1b[L");
        assert_eq!(run(TerminalMode::Televideo, b"\x1bR"), b"\x1b[M");
    }

    #[test]
    fn half_intensity_does_not_eat_the_next_char() {
        // `ESC )` / `ESC (` are the TVI-950 half-intensity on/off pair. Passed
        // through verbatim, a modern terminal reads them as charset-designation
        // lead-ins and swallows the following byte — Turbo's "BORLAND" rendered
        // as "ORLAND". They must translate to ANSI faint/normal, leaving the text
        // (the `B` and `C` here) intact. Note `)` is ON, `(` is OFF.
        let out = run(TerminalMode::Televideo, b"\x1b)B\x1b(C");
        assert_eq!(out, b"\x1b[2mB\x1b[22mC");
    }

    #[test]
    fn modern_csi_passes_through() {
        // A VT100/ANSI program's ESC[31m must survive intact in Televideo mode.
        assert_eq!(
            run(TerminalMode::Televideo, b"\x1b[31mhi\x1b[0m"),
            b"\x1b[31mhi\x1b[0m"
        );
    }
}
