// SPDX-License-Identifier: MIT
// Copyright (c) 2026 David Goldfarb <deg@degel.com>

//! Keyboard input translation, shared by every console front end.
//!
//! The native terminal console and the browser console both receive keys as
//! the bytes a terminal sends: plain characters, control codes, and escape
//! sequences such as `ESC [ A` for the Up arrow (xterm.js in the browser sends
//! the same bytes as a real terminal). This module turns those bytes into what
//! CP/M software expects. It is pure logic with no I/O, so it builds for every
//! target, WebAssembly included.

/// Transform a typed key on the way to the guest: map Delete to Backspace
/// (`map_delete`) and/or fold lowercase letters to uppercase (`upper`).
pub fn map_key(b: u8, upper: bool, map_delete: bool) -> u8 {
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
pub struct KeyXlate {
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
    /// A translator; `on` enables the arrow-key mapping (`--arrows`).
    pub fn new(on: bool) -> Self {
        Self {
            on,
            state: KeyState::Normal,
            params: Vec::new(),
        }
    }

    /// Feed one input byte; append the byte(s) to deliver to the guest to `out`.
    pub fn feed(&mut self, b: u8, out: &mut Vec<u8>) {
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
                if b == 0x1B {
                    // A fresh ESC aborts and restarts the parse (standard
                    // terminal behavior), so the new sequence isn't lost.
                    self.state = KeyState::Esc;
                } else if (0x40..=0x7E).contains(&b) {
                    if let Some(mapped) = map_key_seq(&self.params, b) {
                        out.push(mapped);
                    }
                    // An unrecognized nav sequence is dropped rather than echoed
                    // as garbage — you opted into key mapping.
                    self.state = KeyState::Normal;
                } else if self.params.len() < 16 {
                    self.params.push(b);
                } else {
                    // Runaway sequence (no real key is this long): resync.
                    self.state = KeyState::Normal;
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
    fn esc_restarts_an_in_progress_sequence() {
        // A fresh ESC[ mid-sequence restarts the parse, so the real arrow that
        // follows is recognized rather than mis-parsed and swallowed.
        assert_eq!(xlate(true, b"\x1b[\x1b[A"), vec![0x05]);
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
