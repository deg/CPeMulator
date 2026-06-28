// SPDX-License-Identifier: MIT
// Copyright (c) 2026 David Goldfarb <deg@degel.com>

//! A Z80 disassembler, shared with the monitor.
//!
//! [`disassemble`] decodes the single instruction at an address and returns its
//! assembly text plus its length in bytes, so a caller can step from one
//! instruction to the next. It mirrors the structure of the execution decoder
//! (the same `x/y/z` decomposition and the same prefix handling), which keeps
//! the two readable side by side.
//!
//! Output is lower-case with hexadecimal operands (`ld a,$12`, `jp $1234`,
//! `ld (ix+$05),b`). Relative jumps are shown as their absolute target address.

use crate::bus::Bus;

/// One disassembled instruction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Disassembly {
    /// The assembly text, e.g. `"ld a,$12"`.
    pub text: String,
    /// How many bytes the instruction occupies.
    pub len: u8,
}

const R: [&str; 8] = ["b", "c", "d", "e", "h", "l", "(hl)", "a"];
const RP: [&str; 4] = ["bc", "de", "hl", "sp"];
const RP2: [&str; 4] = ["bc", "de", "hl", "af"];
const CC: [&str; 8] = ["nz", "z", "nc", "c", "po", "pe", "p", "m"];
const ALU: [&str; 8] = [
    "add a,", "adc a,", "sub ", "sbc a,", "and ", "xor ", "or ", "cp ",
];
const ROT: [&str; 8] = ["rlc", "rrc", "rl", "rr", "sla", "sra", "sll", "srl"];

/// A little byte cursor over the bus that tracks how far it has read.
struct Reader<'a, B: Bus> {
    bus: &'a mut B,
    addr: u16,
    start: u16,
}

impl<'a, B: Bus> Reader<'a, B> {
    fn new(bus: &'a mut B, addr: u16) -> Self {
        Self {
            bus,
            addr,
            start: addr,
        }
    }
    fn byte(&mut self) -> u8 {
        let v = self.bus.read(self.addr);
        self.addr = self.addr.wrapping_add(1);
        v
    }
    fn word(&mut self) -> u16 {
        let lo = self.byte();
        let hi = self.byte();
        u16::from(lo) | (u16::from(hi) << 8)
    }
    fn len(&self) -> u8 {
        self.addr.wrapping_sub(self.start) as u8
    }
}

/// Format a signed index displacement as `+$0d` / `-$01`.
fn disp(d: u8) -> String {
    let d = d as i8;
    if d < 0 {
        format!("-${:02x}", -(d as i16))
    } else {
        format!("+${:02x}", d)
    }
}

/// Disassemble the instruction at `addr`, reading bytes through `bus`.
pub fn disassemble(bus: &mut impl Bus, addr: u16) -> Disassembly {
    let mut r = Reader::new(bus, addr);
    let op = r.byte();
    let text = match op {
        0xCB => decode_cb(&mut r, None),
        0xED => decode_ed(&mut r),
        0xDD => decode_index(&mut r, "ix"),
        0xFD => decode_index(&mut r, "iy"),
        _ => decode_main(&mut r, op),
    };
    Disassembly { text, len: r.len() }
}

/// The unprefixed opcode map.
fn decode_main(r: &mut Reader<impl Bus>, op: u8) -> String {
    let x = op >> 6;
    let y = (op >> 3) & 7;
    let z = op & 7;
    let p = y >> 1;
    let q = y & 1;

    match x {
        0 => decode_main_x0(r, y, z, p, q),
        1 => {
            if y == 6 && z == 6 {
                "halt".into()
            } else {
                format!("ld {},{}", R[y as usize], R[z as usize])
            }
        }
        2 => format!("{}{}", ALU[y as usize], R[z as usize]),
        _ => decode_main_x3(r, y, z, p, q),
    }
}

fn decode_main_x0(r: &mut Reader<impl Bus>, y: u8, z: u8, p: u8, q: u8) -> String {
    match z {
        0 => match y {
            0 => "nop".into(),
            1 => "ex af,af'".into(),
            2 => format!("djnz ${:04x}", jr_target(r)),
            3 => format!("jr ${:04x}", jr_target(r)),
            _ => format!("jr {},${:04x}", CC[(y - 4) as usize], jr_target(r)),
        },
        1 if q == 0 => format!("ld {},${:04x}", RP[p as usize], r.word()),
        1 => format!("add hl,{}", RP[p as usize]),
        2 => match (q, p) {
            (0, 0) => "ld (bc),a".into(),
            (0, 1) => "ld (de),a".into(),
            (0, 2) => format!("ld (${:04x}),hl", r.word()),
            (0, _) => format!("ld (${:04x}),a", r.word()),
            (_, 0) => "ld a,(bc)".into(),
            (_, 1) => "ld a,(de)".into(),
            (_, 2) => format!("ld hl,(${:04x})", r.word()),
            (_, _) => format!("ld a,(${:04x})", r.word()),
        },
        3 if q == 0 => format!("inc {}", RP[p as usize]),
        3 => format!("dec {}", RP[p as usize]),
        4 => format!("inc {}", R[y as usize]),
        5 => format!("dec {}", R[y as usize]),
        6 => format!("ld {},${:02x}", R[y as usize], r.byte()),
        _ => ["rlca", "rrca", "rla", "rra", "daa", "cpl", "scf", "ccf"][y as usize].into(),
    }
}

fn decode_main_x3(r: &mut Reader<impl Bus>, y: u8, z: u8, p: u8, q: u8) -> String {
    match z {
        0 => format!("ret {}", CC[y as usize]),
        1 if q == 0 => format!("pop {}", RP2[p as usize]),
        1 => match p {
            0 => "ret".into(),
            1 => "exx".into(),
            2 => "jp (hl)".into(),
            _ => "ld sp,hl".into(),
        },
        2 => format!("jp {},${:04x}", CC[y as usize], r.word()),
        3 => match y {
            0 => format!("jp ${:04x}", r.word()),
            // y==1 is the CB prefix, handled before decode_main is reached.
            2 => format!("out (${:02x}),a", r.byte()),
            3 => format!("in a,(${:02x})", r.byte()),
            4 => "ex (sp),hl".into(),
            5 => "ex de,hl".into(),
            6 => "di".into(),
            _ => "ei".into(),
        },
        4 => format!("call {},${:04x}", CC[y as usize], r.word()),
        5 if q == 0 => format!("push {}", RP2[p as usize]),
        5 => format!("call ${:04x}", r.word()),
        6 => format!("{}${:02x}", ALU[y as usize], r.byte()),
        _ => format!("rst ${:02x}", y * 8),
    }
}

/// Relative-jump target: the byte after this 2-byte instruction plus the signed
/// displacement.
fn jr_target(r: &mut Reader<impl Bus>) -> u16 {
    let d = r.byte() as i8;
    r.addr.wrapping_add(d as u16)
}

/// `CB`-prefixed instructions. When `idx` is `Some((name, d))` this is the
/// `DD CB`/`FD CB` form operating on `(name+d)`.
fn decode_cb(r: &mut Reader<impl Bus>, idx: Option<(&str, u8)>) -> String {
    let op = r.byte();
    let x = op >> 6;
    let y = (op >> 3) & 7;
    let z = op & 7;

    let operand = match idx {
        Some((name, d)) => format!("({}{})", name, disp(d)),
        None => R[z as usize].to_string(),
    };

    match x {
        0 => format!("{} {}", ROT[y as usize], operand),
        1 => format!("bit {},{}", y, operand),
        2 => format!("res {},{}", y, operand),
        _ => format!("set {},{}", y, operand),
    }
}

/// `ED`-prefixed instructions.
fn decode_ed(r: &mut Reader<impl Bus>) -> String {
    let op = r.byte();
    let x = op >> 6;
    let y = (op >> 3) & 7;
    let z = op & 7;
    let p = y >> 1;
    let q = y & 1;

    if x == 2 {
        // Block instructions occupy y>=4, z<4.
        const BLOCK: [[&str; 4]; 4] = [
            ["ldi", "cpi", "ini", "outi"],
            ["ldd", "cpd", "ind", "outd"],
            ["ldir", "cpir", "inir", "otir"],
            ["lddr", "cpdr", "indr", "otdr"],
        ];
        if y >= 4 && z < 4 {
            return BLOCK[(y - 4) as usize][z as usize].into();
        }
        return "nop*".into();
    }
    if x != 1 {
        return "nop*".into(); // undefined ED → effective no-op
    }

    match z {
        0 if y == 6 => "in (c)".into(),
        0 => format!("in {},(c)", R[y as usize]),
        1 if y == 6 => "out (c),0".into(),
        1 => format!("out (c),{}", R[y as usize]),
        2 if q == 0 => format!("sbc hl,{}", RP[p as usize]),
        2 => format!("adc hl,{}", RP[p as usize]),
        3 if q == 0 => format!("ld (${:04x}),{}", r.word(), RP[p as usize]),
        3 => format!("ld {},(${:04x})", RP[p as usize], r.word()),
        4 => "neg".into(),
        5 if y == 1 => "reti".into(),
        5 => "retn".into(),
        6 => format!("im {}", [0, 0, 1, 2, 0, 0, 1, 2][y as usize]),
        _ => [
            "ld i,a", "ld r,a", "ld a,i", "ld a,r", "rrd", "rld", "nop*", "nop*",
        ][y as usize]
            .into(),
    }
}

/// `DD`/`FD`-prefixed instructions, with `name` being `"ix"` or `"iy"`.
fn decode_index(r: &mut Reader<impl Bus>, name: &str) -> String {
    let op = r.byte();

    // Prefix chains and ED: mirror the executor's behavior closely enough for a
    // readable listing.
    match op {
        0xDD => return decode_index(r, "ix"),
        0xFD => return decode_index(r, "iy"),
        0xED => return decode_ed(r),
        0xCB => {
            // DD CB d op: displacement precedes the operation byte.
            let d = r.byte();
            return decode_cb(r, Some((name, d)));
        }
        _ => {}
    }

    let nh = format!("{name}h"); // ixh / iyh
    let nl = format!("{name}l"); // ixl / iyl
    let x = op >> 6;
    let y = (op >> 3) & 7;
    let z = op & 7;
    let p = y >> 1;
    let q = y & 1;

    // Operand name for the r[] field under an index prefix. When the
    // instruction has a memory operand (`mem`), fields 4/5 stay the real h/l.
    let regname = |field: u8, d: Option<u8>, mem: bool| -> String {
        match field {
            4 if !mem => nh.clone(),
            5 if !mem => nl.clone(),
            6 => format!("({}{})", name, disp(d.unwrap())),
            other => R[other as usize].to_string(),
        }
    };

    match x {
        0 => match z {
            1 if q == 0 && p == 2 => format!("ld {},${:04x}", name, r.word()),
            1 if q == 0 => decode_main(r, op), // unaffected
            1 => {
                let rp = if p == 2 {
                    name.to_string()
                } else {
                    RP[p as usize].to_string()
                };
                format!("add {},{}", name, rp)
            }
            2 if p == 2 && q == 0 => format!("ld (${:04x}),{}", r.word(), name),
            2 if p == 2 => format!("ld {},(${:04x})", name, r.word()),
            2 => decode_main(r, op),
            3 if p == 2 && q == 0 => format!("inc {name}"),
            3 if p == 2 => format!("dec {name}"),
            3 => decode_main(r, op),
            4 | 5 if (4..=6).contains(&y) => {
                let mem = y == 6;
                let d = if mem { Some(r.byte()) } else { None };
                let mnem = if z == 4 { "inc" } else { "dec" };
                format!("{} {}", mnem, regname(y, d, mem))
            }
            6 if (4..=6).contains(&y) => {
                let mem = y == 6;
                let d = if mem { Some(r.byte()) } else { None };
                let n = r.byte();
                format!("ld {},${:02x}", regname(y, d, mem), n)
            }
            _ => decode_main(r, op),
        },
        1 => {
            if (y == 6 && z == 6) || !((4..=6).contains(&y) || (4..=6).contains(&z)) {
                return decode_main(r, op);
            }
            let mem = y == 6 || z == 6;
            let d = if mem { Some(r.byte()) } else { None };
            format!("ld {},{}", regname(y, d, mem), regname(z, d, mem))
        }
        2 => {
            if !(4..=6).contains(&z) {
                return decode_main(r, op);
            }
            let mem = z == 6;
            let d = if mem { Some(r.byte()) } else { None };
            format!("{}{}", ALU[y as usize], regname(z, d, mem))
        }
        _ => match z {
            1 if q == 0 && p == 2 => format!("pop {name}"),
            1 if q == 1 && p == 2 => format!("jp ({name})"),
            1 if q == 1 && p == 3 => format!("ld sp,{name}"),
            3 if y == 4 => format!("ex (sp),{name}"),
            5 if q == 0 && p == 2 => format!("push {name}"),
            _ => decode_main(r, op),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::PlainBus;

    fn dis(bytes: &[u8]) -> Disassembly {
        let mut bus = PlainBus::new();
        bus.load(0x0100, bytes);
        disassemble(&mut bus, 0x0100)
    }

    fn text(bytes: &[u8]) -> String {
        dis(bytes).text
    }

    #[test]
    fn simple_ops() {
        assert_eq!(text(&[0x00]), "nop");
        assert_eq!(text(&[0x76]), "halt");
        assert_eq!(text(&[0x78]), "ld a,b");
        assert_eq!(text(&[0x3e, 0x12]), "ld a,$12");
        assert_eq!(text(&[0x21, 0x34, 0x12]), "ld hl,$1234");
        assert_eq!(text(&[0x80]), "add a,b");
        assert_eq!(text(&[0xc6, 0x05]), "add a,$05");
        assert_eq!(text(&[0xc9]), "ret");
        assert_eq!(text(&[0xc3, 0x00, 0x20]), "jp $2000");
        assert_eq!(text(&[0xff]), "rst $38");
    }

    #[test]
    fn lengths_are_correct() {
        assert_eq!(dis(&[0x00]).len, 1);
        assert_eq!(dis(&[0x3e, 0x12]).len, 2);
        assert_eq!(dis(&[0x21, 0x34, 0x12]).len, 3);
        assert_eq!(dis(&[0xdd, 0x36, 0x05, 0x42]).len, 4); // ld (ix+$05),$42
    }

    #[test]
    fn relative_jumps_show_target() {
        // At 0x0100: jr +2 -> target is 0x0100 + 2 + 2 = 0x0104.
        assert_eq!(text(&[0x18, 0x02]), "jr $0104");
        // jr with negative displacement: 0x0100 + 2 - 4 = 0x00fe.
        assert_eq!(text(&[0x18, 0xfc]), "jr $00fe");
        assert_eq!(text(&[0x20, 0x02]), "jr nz,$0104");
    }

    #[test]
    fn cb_prefixed() {
        assert_eq!(text(&[0xcb, 0x00]), "rlc b");
        assert_eq!(text(&[0xcb, 0x46]), "bit 0,(hl)");
        assert_eq!(text(&[0xcb, 0xff]), "set 7,a");
    }

    #[test]
    fn ed_prefixed() {
        assert_eq!(text(&[0xed, 0x44]), "neg");
        assert_eq!(text(&[0xed, 0x4a]), "adc hl,bc");
        assert_eq!(text(&[0xed, 0xb0]), "ldir");
        assert_eq!(text(&[0xed, 0x5e]), "im 2");
        assert_eq!(text(&[0xed, 0x43, 0x00, 0x40]), "ld ($4000),bc");
    }

    #[test]
    fn index_prefixed() {
        assert_eq!(text(&[0xdd, 0x21, 0x34, 0x12]), "ld ix,$1234");
        assert_eq!(text(&[0xdd, 0x23]), "inc ix");
        assert_eq!(text(&[0xdd, 0x86, 0x05]), "add a,(ix+$05)");
        assert_eq!(text(&[0xfd, 0x77, 0xfe]), "ld (iy-$02),a");
        assert_eq!(text(&[0xdd, 0x24]), "inc ixh");
        assert_eq!(text(&[0xdd, 0xe5]), "push ix");
        assert_eq!(text(&[0xdd, 0xcb, 0x05, 0xfe]), "set 7,(ix+$05)");
        // An opcode the prefix doesn't affect disassembles normally.
        assert_eq!(text(&[0xdd, 0x3e, 0x42]), "ld a,$42");
    }
}
