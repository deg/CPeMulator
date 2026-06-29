// SPDX-License-Identifier: MIT
// Copyright (c) 2026 David Goldfarb <deg@degel.com>

//! The interactive debug monitor's command set: register and memory
//! inspect/edit, disassembly, single-step tracing, and breakpoint commands.
//!
//! Each command is a pure function from [`Monitor`] state (plus a typed command
//! line) to text, so the whole surface is unit-testable against the genuine CPU
//! without a terminal. The [`Monitor`] core (cpemu-6o2.1) supplies execution
//! control; this is the operator-facing layer on top. A front-end REPL reads a
//! line, calls [`Debugger::command`], prints the text, and acts on the returned
//! [`Action`].
//!
//! Numbers are hexadecimal (this is a Z80 monitor); counts after `d`/`m`/`s` are
//! decimal. The command set, DDT-flavoured:
//!
//! ```text
//!   r                 show registers
//!   d [addr] [n]      disassemble n instructions (default 8)
//!   m [addr] [n]      dump n bytes of memory (default 64)
//!   e addr b b ...    write bytes to memory
//!   s [n]             step n instructions, tracing each (default 1)
//!   b [addr]          set a breakpoint, or list them
//!   bc addr | bc *    clear one breakpoint, or all
//!   g                 go — resume the CP/M session
//!   q                 quit the emulator
//!   ?                 this help
//! ```

use crate::executor::BiosTrap;
use crate::monitor::Monitor;
use cpemu_cpu::disassemble;

/// What the REPL should do after a command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Stay in the debugger and read another command.
    Stay,
    /// Resume the CP/M session.
    Resume,
    /// Quit the emulator.
    Quit,
}

/// The debug monitor's command processor. Holds only the cursor addresses that
/// let `d` and `m` continue where they left off when given no address.
pub struct Debugger {
    dis_addr: u16,
    mem_addr: u16,
}

impl Default for Debugger {
    fn default() -> Self {
        Self::new()
    }
}

impl Debugger {
    /// A fresh debugger with its `d`/`m` cursors at 0x0100 (the TPA).
    pub fn new() -> Self {
        Self {
            dis_addr: 0x0100,
            mem_addr: 0x0100,
        }
    }

    /// The one-line banner shown when the monitor is entered.
    pub fn banner(pc: u16) -> String {
        format!("-- cpemu monitor (? for help, g to resume) -- PC={pc:04X}")
    }

    /// Run one command `line` against `mon`, returning the text to print and what
    /// the REPL should do next. An empty line is a no-op.
    pub fn command<T: BiosTrap>(&mut self, mon: &mut Monitor<T>, line: &str) -> (String, Action) {
        let mut toks = line.split_whitespace();
        let Some(cmd) = toks.next() else {
            return (String::new(), Action::Stay);
        };
        let args: Vec<&str> = toks.collect();
        match cmd {
            "r" => (self.registers(mon), Action::Stay),
            "d" => (self.disasm(mon, &args), Action::Stay),
            "m" => (self.memory(mon, &args), Action::Stay),
            "e" => (self.edit(mon, &args), Action::Stay),
            "s" => (self.trace(mon, &args), Action::Stay),
            "b" => (self.breakpoint(mon, &args), Action::Stay),
            "bc" => (self.breakpoint_clear(mon, &args), Action::Stay),
            "g" => ("resuming\r\n".into(), Action::Resume),
            "q" => ("quitting\r\n".into(), Action::Quit),
            "?" | "h" | "help" => (Self::help(), Action::Stay),
            other => (
                format!("?  unknown command '{other}' (try ?)\r\n"),
                Action::Stay,
            ),
        }
    }

    // --- inspection --------------------------------------------------------

    fn registers<T: BiosTrap>(&self, mon: &Monitor<T>) -> String {
        let r = &mon.exec.cpu.regs;
        let f = r.f;
        let bit = |b: u8, ch: char| if f & (1 << b) != 0 { ch } else { '.' };
        // S Z H P/V N C — the documented flag bits (7,6,4,2,1,0).
        let flags: String = [
            bit(7, 'S'),
            bit(6, 'Z'),
            bit(4, 'H'),
            bit(2, 'P'),
            bit(1, 'N'),
            bit(0, 'C'),
        ]
        .into_iter()
        .collect();
        format!(
            "PC={:04X} SP={:04X}  A={:02X} F={:02X}[{}]  BC={:04X} DE={:04X} HL={:04X}  IX={:04X} IY={:04X}\r\n",
            r.pc,
            r.sp,
            r.a,
            f,
            flags,
            r.bc(),
            r.de(),
            r.hl(),
            r.ix,
            r.iy
        )
    }

    fn disasm<T: BiosTrap>(&mut self, mon: &mut Monitor<T>, args: &[&str]) -> String {
        let mut addr = args.first().and_then(|s| hex16(s)).unwrap_or(self.dis_addr);
        let count = args
            .get(1)
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(8);
        let pc = mon.exec.cpu.regs.pc;
        let mut out = String::new();
        for _ in 0..count {
            let d = disassemble(&mut mon.exec.machine, addr);
            let mut bytes = String::new();
            for i in 0..d.len {
                bytes += &format!(
                    "{:02X} ",
                    mon.exec.machine.mem.read(addr.wrapping_add(i.into()))
                );
            }
            let marker = if addr == pc { '>' } else { ' ' };
            out += &format!("{marker}{addr:04X}  {bytes:<13}{}\r\n", d.text);
            addr = addr.wrapping_add(d.len.max(1).into());
        }
        self.dis_addr = addr;
        out
    }

    fn memory<T: BiosTrap>(&mut self, mon: &mut Monitor<T>, args: &[&str]) -> String {
        let mut addr = args.first().and_then(|s| hex16(s)).unwrap_or(self.mem_addr);
        let count = args
            .get(1)
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(64);
        let mut out = String::new();
        let mut left = count;
        while left > 0 {
            let n = left.min(16);
            let mut hexs = String::new();
            let mut asc = String::new();
            for i in 0..16 {
                if i < n {
                    let b = mon.exec.machine.mem.read(addr.wrapping_add(i as u16));
                    hexs += &format!("{b:02X} ");
                    asc.push(if (0x20..=0x7e).contains(&b) {
                        b as char
                    } else {
                        '.'
                    });
                } else {
                    hexs += "   ";
                }
            }
            out += &format!("{addr:04X}  {hexs} {asc}\r\n");
            addr = addr.wrapping_add(n as u16);
            left -= n;
        }
        self.mem_addr = addr;
        out
    }

    // --- editing -----------------------------------------------------------

    fn edit<T: BiosTrap>(&mut self, mon: &mut Monitor<T>, args: &[&str]) -> String {
        let Some(addr) = args.first().and_then(|s| hex16(s)) else {
            return "e: usage: e ADDR BYTE [BYTE ...]\r\n".into();
        };
        let mut bytes = Vec::new();
        for s in &args[1..] {
            match hex8(s) {
                Some(b) => bytes.push(b),
                None => return format!("e: '{s}' is not a hex byte\r\n"),
            }
        }
        if bytes.is_empty() {
            return "e: no bytes given\r\n".into();
        }
        for (i, b) in bytes.iter().enumerate() {
            mon.exec.machine.mem.write(addr.wrapping_add(i as u16), *b);
        }
        format!("wrote {} byte(s) at {addr:04X}\r\n", bytes.len())
    }

    // --- execution / tracing ----------------------------------------------

    fn trace<T: BiosTrap>(&mut self, mon: &mut Monitor<T>, args: &[&str]) -> String {
        let n = args
            .first()
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(1);
        let mut out = String::new();
        for _ in 0..n {
            let pc = mon.exec.cpu.regs.pc;
            let d = disassemble(&mut mon.exec.machine, pc);
            out += &format!("{pc:04X}  {}\r\n", d.text);
            if let Some(reason) = mon.step() {
                out += &format!("--- {reason:?} ---\r\n");
                break;
            }
        }
        out += &self.registers(mon);
        out
    }

    // --- breakpoints -------------------------------------------------------

    fn breakpoint<T: BiosTrap>(&self, mon: &mut Monitor<T>, args: &[&str]) -> String {
        match args.first().and_then(|s| hex16(s)) {
            Some(addr) => {
                if mon.add_breakpoint(addr) {
                    format!("breakpoint set at {addr:04X}\r\n")
                } else {
                    format!("breakpoint already at {addr:04X}\r\n")
                }
            }
            None => {
                let list: Vec<String> = mon.breakpoints().map(|a| format!("{a:04X}")).collect();
                if list.is_empty() {
                    "no breakpoints\r\n".into()
                } else {
                    format!("breakpoints: {}\r\n", list.join(" "))
                }
            }
        }
    }

    fn breakpoint_clear<T: BiosTrap>(&self, mon: &mut Monitor<T>, args: &[&str]) -> String {
        match args.first() {
            Some(&"*") => {
                mon.clear_breakpoints();
                "all breakpoints cleared\r\n".into()
            }
            Some(s) => match hex16(s) {
                Some(addr) if mon.remove_breakpoint(addr) => {
                    format!("cleared breakpoint at {addr:04X}\r\n")
                }
                Some(addr) => format!("no breakpoint at {addr:04X}\r\n"),
                None => "bc: usage: bc ADDR | bc *\r\n".into(),
            },
            None => "bc: usage: bc ADDR | bc *\r\n".into(),
        }
    }

    fn help() -> String {
        "commands (numbers are hex):\r\n\
         \x20 r              registers\r\n\
         \x20 d [addr] [n]   disassemble n instructions (default 8)\r\n\
         \x20 m [addr] [n]   dump n bytes of memory (default 64)\r\n\
         \x20 e addr b...    write bytes to memory\r\n\
         \x20 s [n]          step n instructions, tracing each\r\n\
         \x20 b [addr]       set a breakpoint, or list them\r\n\
         \x20 bc addr | bc * clear one breakpoint, or all\r\n\
         \x20 g              go (resume CP/M)\r\n\
         \x20 q              quit the emulator\r\n"
            .into()
    }
}

/// Parse a 16-bit hex address, tolerating a `$` or `0x` prefix.
fn hex16(s: &str) -> Option<u16> {
    let s = s
        .strip_prefix("0x")
        .or_else(|| s.strip_prefix('$'))
        .unwrap_or(s);
    u16::from_str_radix(s, 16).ok()
}

/// Parse an 8-bit hex byte, tolerating a `$` or `0x` prefix.
fn hex8(s: &str) -> Option<u8> {
    let s = s
        .strip_prefix("0x")
        .or_else(|| s.strip_prefix('$'))
        .unwrap_or(s);
    u8::from_str_radix(s, 16).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::executor::{Executor, TrapAction};
    use crate::machine::{Machine, TrapRegion};
    use crate::memory::Memory;
    use cpemu_cpu::Cpu;

    struct NoBios;
    impl BiosTrap for NoBios {
        fn call(&mut self, _v: u8, _c: &mut Cpu, _m: &mut Memory) -> TrapAction {
            TrapAction::Return
        }
    }

    // LD A,$12 ; LD B,$34 ; HALT  at 0x0100.
    const PROG: &[u8] = &[0x3E, 0x12, 0x06, 0x34, 0x76];

    fn mon() -> Monitor<NoBios> {
        let traps = TrapRegion {
            base: 0xFFFF,
            stride: 1,
            count: 0,
        };
        let mut machine = Machine::new(traps);
        machine.mem.load(0x0100, PROG);
        let mut exec = Executor::new(machine, NoBios);
        exec.cpu.regs.pc = 0x0100;
        Monitor::new(exec)
    }

    fn run(d: &mut Debugger, m: &mut Monitor<NoBios>, line: &str) -> (String, Action) {
        d.command(m, line)
    }

    #[test]
    fn registers_show_pc_and_flags() {
        let (mut d, mut m) = (Debugger::new(), mon());
        let (out, act) = run(&mut d, &mut m, "r");
        assert_eq!(act, Action::Stay);
        assert!(out.contains("PC=0100"), "{out}");
        assert!(out.contains("SP="), "{out}");
    }

    #[test]
    fn disassemble_decodes_known_opcodes() {
        let (mut d, mut m) = (Debugger::new(), mon());
        let (out, _) = run(&mut d, &mut m, "d 100 2");
        assert!(out.contains("ld a,$12"), "first instr:\n{out}");
        assert!(out.contains("ld b,$34"), "second instr:\n{out}");
        assert!(out.contains(">0100"), "PC should be marked:\n{out}");
        // A second `d` with no address continues from where this left off.
        let (out2, _) = run(&mut d, &mut m, "d");
        assert!(
            out2.contains("0104"),
            "should continue past the listed instrs:\n{out2}"
        );
    }

    #[test]
    fn memory_dump_shows_hex_and_ascii() {
        let (mut d, mut m) = (Debugger::new(), mon());
        m.exec.machine.mem.load(0x0200, b"HI!");
        let (out, _) = run(&mut d, &mut m, "m 200 3");
        assert!(out.contains("48 49 21"), "hex:\n{out}");
        assert!(out.contains("HI!"), "ascii:\n{out}");
    }

    #[test]
    fn edit_writes_bytes() {
        let (mut d, mut m) = (Debugger::new(), mon());
        let (out, _) = run(&mut d, &mut m, "e 300 DE AD BE EF");
        assert!(out.contains("wrote 4"), "{out}");
        assert_eq!(m.exec.machine.mem.read(0x0300), 0xDE);
        assert_eq!(m.exec.machine.mem.read(0x0303), 0xEF);
    }

    #[test]
    fn trace_steps_and_shows_each_instruction() {
        let (mut d, mut m) = (Debugger::new(), mon());
        let (out, _) = run(&mut d, &mut m, "s 2");
        assert!(out.contains("ld a,$12"), "{out}");
        assert!(out.contains("ld b,$34"), "{out}");
        assert_eq!(m.exec.cpu.regs.a, 0x12);
        assert_eq!(m.exec.cpu.regs.b, 0x34);
    }

    #[test]
    fn breakpoint_set_list_clear() {
        let (mut d, mut m) = (Debugger::new(), mon());
        assert!(run(&mut d, &mut m, "b 104").0.contains("set at 0104"));
        assert!(run(&mut d, &mut m, "b 102").0.contains("set at 0102"));
        assert!(run(&mut d, &mut m, "b").0.contains("0102 0104")); // listed, sorted
        assert!(run(&mut d, &mut m, "bc 104").0.contains("cleared"));
        assert!(run(&mut d, &mut m, "bc *").0.contains("all"));
        assert!(run(&mut d, &mut m, "b").0.contains("no breakpoints"));
    }

    #[test]
    fn go_and_quit_actions() {
        let (mut d, mut m) = (Debugger::new(), mon());
        assert_eq!(run(&mut d, &mut m, "g").1, Action::Resume);
        assert_eq!(run(&mut d, &mut m, "q").1, Action::Quit);
        assert_eq!(run(&mut d, &mut m, "").1, Action::Stay); // empty line
        assert!(run(&mut d, &mut m, "zztop").0.contains("unknown"));
    }
}
