//! The CPU itself: machine state and the fetch/decode/execute step.
//!
//! Decoding follows the standard Z80 opcode structure (see Cristian Dinu's
//! "Decoding Z80 Opcodes"): the opcode byte splits into
//! `x = bits 7-6`, `y = bits 5-3`, `z = bits 2-0`, with `p = y >> 1` and
//! `q = y & 1`. This compact, table-driven decode covers the whole unprefixed
//! map far more reliably than ~250 hand-written arms.
//!
//! This task (`cpemu-yd5.2`) implements the unprefixed ("8080 base") opcodes
//! with Z80 semantics. The `CB`/`DD`/`ED`/`FD` prefix bytes remain
//! [`Stop::Illegal`] until the Z80-extensions task (`cpemu-yd5.3`).

use crate::bus::Bus;
use crate::index::Index;
use crate::registers::{Registers, flag};

/// Index of the `(HL)` pseudo-register in the `r[]` table.
const HL_MEM: u8 = 6;

/// Why a [`Cpu::step`] returned — so the runner can observe control flow rather
/// than only counting cycles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stop {
    /// An instruction executed normally.
    Normal,
    /// The CPU is halted (executed `HALT`); it idles until an interrupt.
    Halted,
    /// The opcode is not yet implemented. A development aid: the exerciser
    /// harness reports *which* opcode failed instead of the emulator panicking.
    Illegal(u8),
}

/// The result of a single [`Cpu::step`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StepResult {
    /// T-states (clock cycles) consumed by this step.
    pub cycles: u32,
    /// How the step concluded.
    pub stop: Stop,
}

/// A Z80 CPU (a superset that also executes 8080 code).
#[derive(Debug, Clone)]
pub struct Cpu {
    /// The programmer-visible register file.
    pub regs: Registers,
    /// Interrupt enable flip-flop 1 (gates maskable interrupts).
    pub iff1: bool,
    /// Interrupt enable flip-flop 2 (saved copy of `iff1`).
    pub iff2: bool,
    /// Interrupt mode (0, 1, or 2).
    pub im: u8,
    /// Set for one instruction after `EI`: a maskable interrupt is not accepted
    /// in the shadow of `EI` (so the classic `EI; RET` returns before any
    /// pending interrupt is taken). See [`Cpu::can_accept_interrupt`].
    pub ei_delay: bool,
    /// Whether the CPU is halted, awaiting an interrupt.
    pub halted: bool,
    /// Total T-states executed since construction/reset.
    pub cycles: u64,
}

impl Cpu {
    /// Create a CPU in the post-reset state (`PC = 0`, interrupts disabled).
    pub fn new() -> Self {
        Self {
            regs: Registers::default(),
            iff1: false,
            iff2: false,
            im: 0,
            ei_delay: false,
            halted: false,
            cycles: 0,
        }
    }

    /// Reset to power-on state: `PC`, `I`, `R` cleared, interrupts disabled,
    /// interrupt mode 0. Other registers are left undefined on real hardware;
    /// here they retain their values.
    pub fn reset(&mut self) {
        self.regs.pc = 0;
        self.regs.i = 0;
        self.regs.r = 0;
        self.iff1 = false;
        self.iff2 = false;
        self.im = 0;
        self.ei_delay = false;
        self.halted = false;
    }

    // --- fetch helpers ---------------------------------------------------

    /// Fetch the opcode byte at `PC` (an M1 cycle): advance `PC` and tick the
    /// refresh register.
    pub(crate) fn fetch(&mut self, bus: &mut impl Bus) -> u8 {
        let opcode = bus.read(self.regs.pc);
        self.regs.pc = self.regs.pc.wrapping_add(1);
        self.regs.inc_r();
        opcode
    }

    /// Read an operand byte at `PC` and advance `PC`. Operand fetches are not
    /// M1 cycles, so they do **not** tick the refresh register.
    pub(crate) fn next_byte(&mut self, bus: &mut impl Bus) -> u8 {
        let v = bus.read(self.regs.pc);
        self.regs.pc = self.regs.pc.wrapping_add(1);
        v
    }

    /// Read a little-endian operand word at `PC` and advance `PC` by two.
    pub(crate) fn next_word(&mut self, bus: &mut impl Bus) -> u16 {
        let lo = self.next_byte(bus);
        let hi = self.next_byte(bus);
        u16::from(lo) | (u16::from(hi) << 8)
    }

    pub(crate) fn done(&mut self, cycles: u32, stop: Stop) -> StepResult {
        self.cycles += u64::from(cycles);
        StepResult { cycles, stop }
    }

    // --- register-table access ------------------------------------------

    /// Read `r[idx]` (0=B,1=C,2=D,3=E,4=H,5=L,6=(HL),7=A).
    pub(crate) fn reg_get(&mut self, idx: u8, bus: &mut impl Bus) -> u8 {
        match idx {
            0 => self.regs.b,
            1 => self.regs.c,
            2 => self.regs.d,
            3 => self.regs.e,
            4 => self.regs.h,
            5 => self.regs.l,
            6 => bus.read(self.regs.hl()),
            _ => self.regs.a,
        }
    }

    /// Write `r[idx]`.
    pub(crate) fn reg_set(&mut self, idx: u8, val: u8, bus: &mut impl Bus) {
        match idx {
            0 => self.regs.b = val,
            1 => self.regs.c = val,
            2 => self.regs.d = val,
            3 => self.regs.e = val,
            4 => self.regs.h = val,
            5 => self.regs.l = val,
            6 => bus.write(self.regs.hl(), val),
            _ => self.regs.a = val,
        }
    }

    /// Read `rp[idx]` (0=BC,1=DE,2=HL,3=SP).
    pub(crate) fn rp_get(&self, idx: u8) -> u16 {
        match idx {
            0 => self.regs.bc(),
            1 => self.regs.de(),
            2 => self.regs.hl(),
            _ => self.regs.sp,
        }
    }

    /// Write `rp[idx]`.
    pub(crate) fn rp_set(&mut self, idx: u8, val: u16) {
        match idx {
            0 => self.regs.set_bc(val),
            1 => self.regs.set_de(val),
            2 => self.regs.set_hl(val),
            _ => self.regs.sp = val,
        }
    }

    /// Read `rp2[idx]` (0=BC,1=DE,2=HL,3=AF), used by PUSH/POP.
    pub(crate) fn rp2_get(&self, idx: u8) -> u16 {
        match idx {
            0 => self.regs.bc(),
            1 => self.regs.de(),
            2 => self.regs.hl(),
            _ => self.regs.af(),
        }
    }

    /// Write `rp2[idx]`.
    pub(crate) fn rp2_set(&mut self, idx: u8, val: u16) {
        match idx {
            0 => self.regs.set_bc(val),
            1 => self.regs.set_de(val),
            2 => self.regs.set_hl(val),
            _ => self.regs.set_af(val),
        }
    }

    /// Evaluate condition `cc[idx]` (NZ,Z,NC,C,PO,PE,P,M).
    pub(crate) fn cond(&self, idx: u8) -> bool {
        match idx {
            0 => !self.regs.flag(flag::Z),
            1 => self.regs.flag(flag::Z),
            2 => !self.regs.flag(flag::C),
            3 => self.regs.flag(flag::C),
            4 => !self.regs.flag(flag::PV),
            5 => self.regs.flag(flag::PV),
            6 => !self.regs.flag(flag::S),
            _ => self.regs.flag(flag::S),
        }
    }

    // --- stack / control-flow helpers -----------------------------------

    pub(crate) fn push16(&mut self, bus: &mut impl Bus, val: u16) {
        self.regs.sp = self.regs.sp.wrapping_sub(1);
        bus.write(self.regs.sp, (val >> 8) as u8);
        self.regs.sp = self.regs.sp.wrapping_sub(1);
        bus.write(self.regs.sp, val as u8);
    }

    pub(crate) fn pop16(&mut self, bus: &mut impl Bus) -> u16 {
        let lo = bus.read(self.regs.sp);
        self.regs.sp = self.regs.sp.wrapping_add(1);
        let hi = bus.read(self.regs.sp);
        self.regs.sp = self.regs.sp.wrapping_add(1);
        u16::from(lo) | (u16::from(hi) << 8)
    }

    /// Relative jump: `PC += d` (signed).
    pub(crate) fn jr(&mut self, d: u8) {
        self.regs.pc = self.regs.pc.wrapping_add((d as i8) as u16);
    }

    /// Dispatch the `alu[op]` operation on the accumulator with `val`.
    pub(crate) fn alu(&mut self, op: u8, val: u8) {
        match op {
            0 => self.regs.a = self.add8(self.regs.a, val, false), // ADD
            1 => self.regs.a = self.add8(self.regs.a, val, self.regs.flag(flag::C)), // ADC
            2 => self.regs.a = self.sub8(self.regs.a, val, false), // SUB
            3 => self.regs.a = self.sub8(self.regs.a, val, self.regs.flag(flag::C)), // SBC
            4 => self.and8(val),
            5 => self.xor8(val),
            6 => self.or8(val),
            _ => self.cp8(val), // CP
        }
    }

    // --- the decode ------------------------------------------------------

    /// Execute one instruction and return T-states consumed plus how it ended.
    ///
    /// While halted, the CPU refreshes memory and burns 4 T-states per step (an
    /// internal `NOP`) until an interrupt clears `halted`.
    pub fn step(&mut self, bus: &mut impl Bus) -> StepResult {
        if self.halted {
            self.regs.inc_r();
            return self.done(4, Stop::Halted);
        }

        // The one-instruction `EI` shadow ends as this instruction begins; a
        // machine samples interrupts before calling step(), so by clearing here
        // the shadow covers exactly the single instruction after `EI`.
        self.ei_delay = false;

        let opcode = self.fetch(bus);
        self.execute(opcode, bus)
    }

    /// Decode and execute a single already-fetched opcode. Split out from
    /// [`Cpu::step`] so the `DD`/`FD` index handler can delegate any opcode it
    /// does not need to treat specially straight back to the normal decode.
    pub(crate) fn execute(&mut self, opcode: u8, bus: &mut impl Bus) -> StepResult {
        let x = opcode >> 6;
        let y = (opcode >> 3) & 7;
        let z = opcode & 7;
        let p = y >> 1;
        let q = y & 1;

        match x {
            0 => self.exec_x0(bus, y, z, p, q),
            1 => self.exec_x1_load(bus, y, z),
            2 => {
                // alu[y] A, r[z]
                let v = self.reg_get(z, bus);
                self.alu(y, v);
                self.done(if z == HL_MEM { 7 } else { 4 }, Stop::Normal)
            }
            _ => self.exec_x3(bus, y, z, p, q),
        }
    }

    /// `x = 0`: the miscellaneous group (control flow, 16-bit loads, indirect
    /// loads, INC/DEC, immediate loads, accumulator rotates and flag ops).
    fn exec_x0(&mut self, bus: &mut impl Bus, y: u8, z: u8, p: u8, q: u8) -> StepResult {
        match z {
            0 => match y {
                0 => self.done(4, Stop::Normal), // NOP
                1 => {
                    self.regs.ex_af();
                    self.done(4, Stop::Normal)
                }
                2 => {
                    // DJNZ d
                    let d = self.next_byte(bus);
                    self.regs.b = self.regs.b.wrapping_sub(1);
                    if self.regs.b != 0 {
                        self.jr(d);
                        self.done(13, Stop::Normal)
                    } else {
                        self.done(8, Stop::Normal)
                    }
                }
                3 => {
                    // JR d
                    let d = self.next_byte(bus);
                    self.jr(d);
                    self.done(12, Stop::Normal)
                }
                _ => {
                    // JR cc[y-4], d
                    let d = self.next_byte(bus);
                    if self.cond(y - 4) {
                        self.jr(d);
                        self.done(12, Stop::Normal)
                    } else {
                        self.done(7, Stop::Normal)
                    }
                }
            },
            1 => {
                if q == 0 {
                    // LD rp[p], nn
                    let nn = self.next_word(bus);
                    self.rp_set(p, nn);
                    self.done(10, Stop::Normal)
                } else {
                    // ADD HL, rp[p]
                    let res = self.add16(self.regs.hl(), self.rp_get(p));
                    self.regs.set_hl(res);
                    self.done(11, Stop::Normal)
                }
            }
            2 => self.exec_x0_z2(bus, p, q),
            3 => {
                // INC/DEC rp[p] (16-bit, no flags)
                let v = self.rp_get(p);
                let v = if q == 0 {
                    v.wrapping_add(1)
                } else {
                    v.wrapping_sub(1)
                };
                self.rp_set(p, v);
                self.done(6, Stop::Normal)
            }
            4 => {
                // INC r[y]
                let v = self.reg_get(y, bus);
                let r = self.inc8(v);
                self.reg_set(y, r, bus);
                self.done(if y == HL_MEM { 11 } else { 4 }, Stop::Normal)
            }
            5 => {
                // DEC r[y]
                let v = self.reg_get(y, bus);
                let r = self.dec8(v);
                self.reg_set(y, r, bus);
                self.done(if y == HL_MEM { 11 } else { 4 }, Stop::Normal)
            }
            6 => {
                // LD r[y], n
                let n = self.next_byte(bus);
                self.reg_set(y, n, bus);
                self.done(if y == HL_MEM { 10 } else { 7 }, Stop::Normal)
            }
            _ => {
                // z == 7: accumulator rotates and flag ops
                match y {
                    0 => self.rlca(),
                    1 => self.rrca(),
                    2 => self.rla(),
                    3 => self.rra(),
                    4 => self.daa(),
                    5 => self.cpl(),
                    6 => self.scf(),
                    _ => self.ccf(),
                }
                self.done(4, Stop::Normal)
            }
        }
    }

    /// `x = 0, z = 2`: indirect loads through BC/DE and direct `(nn)` loads.
    fn exec_x0_z2(&mut self, bus: &mut impl Bus, p: u8, q: u8) -> StepResult {
        if q == 0 {
            match p {
                0 => {
                    bus.write(self.regs.bc(), self.regs.a); // LD (BC),A
                    self.done(7, Stop::Normal)
                }
                1 => {
                    bus.write(self.regs.de(), self.regs.a); // LD (DE),A
                    self.done(7, Stop::Normal)
                }
                2 => {
                    let nn = self.next_word(bus); // LD (nn),HL
                    bus.write16(nn, self.regs.hl());
                    self.done(16, Stop::Normal)
                }
                _ => {
                    let nn = self.next_word(bus); // LD (nn),A
                    bus.write(nn, self.regs.a);
                    self.done(13, Stop::Normal)
                }
            }
        } else {
            match p {
                0 => {
                    self.regs.a = bus.read(self.regs.bc()); // LD A,(BC)
                    self.done(7, Stop::Normal)
                }
                1 => {
                    self.regs.a = bus.read(self.regs.de()); // LD A,(DE)
                    self.done(7, Stop::Normal)
                }
                2 => {
                    let nn = self.next_word(bus); // LD HL,(nn)
                    let v = bus.read16(nn);
                    self.regs.set_hl(v);
                    self.done(16, Stop::Normal)
                }
                _ => {
                    let nn = self.next_word(bus); // LD A,(nn)
                    self.regs.a = bus.read(nn);
                    self.done(13, Stop::Normal)
                }
            }
        }
    }

    /// `x = 1`: `LD r[y], r[z]` (with `HALT` at `y == z == 6`).
    fn exec_x1_load(&mut self, bus: &mut impl Bus, y: u8, z: u8) -> StepResult {
        if y == HL_MEM && z == HL_MEM {
            self.halted = true; // HALT
            return self.done(4, Stop::Halted);
        }
        let v = self.reg_get(z, bus);
        self.reg_set(y, v, bus);
        let cycles = if y == HL_MEM || z == HL_MEM { 7 } else { 4 };
        self.done(cycles, Stop::Normal)
    }

    /// `x = 3`: returns, stack ops, jumps, calls, I/O, immediate ALU, RST, and
    /// the `CB`/`DD`/`ED`/`FD` prefix entry points.
    fn exec_x3(&mut self, bus: &mut impl Bus, y: u8, z: u8, p: u8, q: u8) -> StepResult {
        match z {
            0 => {
                // RET cc[y]
                if self.cond(y) {
                    let addr = self.pop16(bus);
                    self.regs.pc = addr;
                    self.done(11, Stop::Normal)
                } else {
                    self.done(5, Stop::Normal)
                }
            }
            1 => {
                if q == 0 {
                    // POP rp2[p]
                    let v = self.pop16(bus);
                    self.rp2_set(p, v);
                    self.done(10, Stop::Normal)
                } else {
                    match p {
                        0 => {
                            // RET
                            let addr = self.pop16(bus);
                            self.regs.pc = addr;
                            self.done(10, Stop::Normal)
                        }
                        1 => {
                            self.regs.exx();
                            self.done(4, Stop::Normal)
                        }
                        2 => {
                            self.regs.pc = self.regs.hl(); // JP (HL)
                            self.done(4, Stop::Normal)
                        }
                        _ => {
                            self.regs.sp = self.regs.hl(); // LD SP,HL
                            self.done(6, Stop::Normal)
                        }
                    }
                }
            }
            2 => {
                // JP cc[y], nn
                let nn = self.next_word(bus);
                if self.cond(y) {
                    self.regs.pc = nn;
                }
                self.done(10, Stop::Normal)
            }
            3 => self.exec_x3_z3(bus, y),
            4 => {
                // CALL cc[y], nn
                let nn = self.next_word(bus);
                if self.cond(y) {
                    self.push16(bus, self.regs.pc);
                    self.regs.pc = nn;
                    self.done(17, Stop::Normal)
                } else {
                    self.done(10, Stop::Normal)
                }
            }
            5 => {
                if q == 0 {
                    // PUSH rp2[p]
                    self.push16(bus, self.rp2_get(p));
                    self.done(11, Stop::Normal)
                } else if p == 0 {
                    // CALL nn
                    let nn = self.next_word(bus);
                    self.push16(bus, self.regs.pc);
                    self.regs.pc = nn;
                    self.done(17, Stop::Normal)
                } else {
                    match p {
                        1 => self.exec_index(bus, Index::Ix), // DD: IX-indexed
                        2 => self.exec_ed(bus),               // ED-prefixed extended ops
                        _ => self.exec_index(bus, Index::Iy), // FD: IY-indexed
                    }
                }
            }
            6 => {
                // alu[y] A, n
                let n = self.next_byte(bus);
                self.alu(y, n);
                self.done(7, Stop::Normal)
            }
            _ => {
                // RST y*8
                self.push16(bus, self.regs.pc);
                self.regs.pc = u16::from(y) * 8;
                self.done(11, Stop::Normal)
            }
        }
    }

    /// `x = 3, z = 3`: the assorted jump / I/O / exchange / interrupt ops.
    fn exec_x3_z3(&mut self, bus: &mut impl Bus, y: u8) -> StepResult {
        match y {
            0 => {
                let nn = self.next_word(bus); // JP nn
                self.regs.pc = nn;
                self.done(10, Stop::Normal)
            }
            1 => self.exec_cb(bus), // CB-prefixed bit/rotate/shift ops
            2 => {
                // OUT (n),A — address bus: A on high byte, n on low byte.
                let n = self.next_byte(bus);
                let port = (u16::from(self.regs.a) << 8) | u16::from(n);
                bus.output(port, self.regs.a);
                self.done(11, Stop::Normal)
            }
            3 => {
                // IN A,(n)
                let n = self.next_byte(bus);
                let port = (u16::from(self.regs.a) << 8) | u16::from(n);
                self.regs.a = bus.input(port);
                self.done(11, Stop::Normal)
            }
            4 => {
                // EX (SP),HL
                let sp = self.regs.sp;
                let mem = bus.read16(sp);
                bus.write16(sp, self.regs.hl());
                self.regs.set_hl(mem);
                self.done(19, Stop::Normal)
            }
            5 => {
                // EX DE,HL
                let de = self.regs.de();
                let hl = self.regs.hl();
                self.regs.set_de(hl);
                self.regs.set_hl(de);
                self.done(4, Stop::Normal)
            }
            6 => {
                // DI
                self.iff1 = false;
                self.iff2 = false;
                self.ei_delay = false;
                self.done(4, Stop::Normal)
            }
            _ => {
                // EI — interrupts enabled, but not sampled until after the next
                // instruction (the `ei_delay` shadow).
                self.iff1 = true;
                self.iff2 = true;
                self.ei_delay = true;
                self.done(4, Stop::Normal)
            }
        }
    }
}

impl Default for Cpu {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::PlainBus;

    fn run(program: &[u8]) -> (Cpu, PlainBus) {
        let mut cpu = Cpu::new();
        let mut bus = PlainBus::new();
        bus.load(0x0000, program);
        let mut guard = 0;
        loop {
            let r = cpu.step(&mut bus);
            guard += 1;
            assert!(guard < 10_000, "runaway program");
            if r.stop == Stop::Halted {
                break;
            }
            if let Stop::Illegal(op) = r.stop {
                panic!("illegal opcode {op:#04x} at pc {:#06x}", cpu.regs.pc);
            }
        }
        (cpu, bus)
    }

    #[test]
    fn nop_advances_pc_and_counts_cycles() {
        let mut cpu = Cpu::new();
        let mut bus = PlainBus::new();
        bus.load(0x0000, &[0x00, 0x76]);
        let r = cpu.step(&mut bus);
        assert_eq!(r.stop, Stop::Normal);
        assert_eq!(r.cycles, 4);
        assert_eq!(cpu.regs.pc, 0x0001);
    }

    #[test]
    fn halt_sets_halted_and_spins() {
        let mut cpu = Cpu::new();
        let mut bus = PlainBus::new();
        bus.load(0x0000, &[0x76]);
        let r = cpu.step(&mut bus);
        assert_eq!(r.stop, Stop::Halted);
        assert!(cpu.halted);
        let r2 = cpu.step(&mut bus);
        assert_eq!(r2.stop, Stop::Halted);
        assert_eq!(cpu.regs.pc, 0x0001);
    }

    #[test]
    fn all_prefixes_are_handled() {
        // Every prefix (CB, DD, ED, FD) now decodes rather than reporting an
        // illegal opcode. A bare prefix followed by NOP must execute normally.
        for prefix in [0xCB, 0xDD, 0xED, 0xFD] {
            let mut cpu = Cpu::new();
            let mut bus = PlainBus::new();
            bus.load(0x0000, &[prefix, 0x00]);
            assert!(
                !matches!(cpu.step(&mut bus).stop, Stop::Illegal(_)),
                "prefix {prefix:#04x} reported illegal"
            );
        }
    }

    #[test]
    fn reset_clears_control_state() {
        let mut cpu = Cpu::new();
        cpu.regs.pc = 0x1234;
        cpu.iff1 = true;
        cpu.halted = true;
        cpu.reset();
        assert_eq!(cpu.regs.pc, 0);
        assert!(!cpu.iff1);
        assert!(!cpu.halted);
    }

    #[test]
    fn ld_immediate_and_add() {
        // LD A,0x14 ; LD B,0x2E ; ADD A,B ; HALT
        let (cpu, _) = run(&[0x3E, 0x14, 0x06, 0x2E, 0x80, 0x76]);
        assert_eq!(cpu.regs.a, 0x42);
        assert_eq!(cpu.regs.b, 0x2E);
    }

    #[test]
    fn ld16_indirect_roundtrip() {
        // LD HL,0x1234 ; LD (0x40),HL ; LD HL,0 ; LD HL,(0x40) ; HALT
        let (cpu, bus) = run(&[
            0x21, 0x34, 0x12, // LD HL,0x1234
            0x22, 0x40, 0x00, // LD (0x0040),HL
            0x21, 0x00, 0x00, // LD HL,0x0000
            0x2A, 0x40, 0x00, // LD HL,(0x0040)
            0x76, // HALT
        ]);
        assert_eq!(cpu.regs.hl(), 0x1234);
        let mut bus = bus;
        assert_eq!(bus.read16(0x0040), 0x1234);
    }

    #[test]
    fn call_and_ret() {
        // ORG 0: LD SP,0x100 ; CALL 0x08 ; HALT ; (pad) ; ORG8: LD A,0x99 ; RET
        let mut prog = vec![0u8; 0x10];
        prog[0x00] = 0x31;
        prog[0x01] = 0x00;
        prog[0x02] = 0x01; // LD SP,0x0100
        prog[0x03] = 0xCD;
        prog[0x04] = 0x08;
        prog[0x05] = 0x00; // CALL 0x0008
        prog[0x06] = 0x76; // HALT
        prog[0x08] = 0x3E;
        prog[0x09] = 0x99; // LD A,0x99
        prog[0x0A] = 0xC9; // RET
        let (cpu, _) = run(&prog);
        assert_eq!(cpu.regs.a, 0x99);
        assert_eq!(cpu.regs.pc, 0x0007); // returned past CALL, halted after
    }

    #[test]
    fn conditional_jump_loop() {
        // Count B down from 3 to 0 with DEC B / JR NZ.
        // LD B,3 ; (loop) DEC B ; JR NZ,-3 ; HALT
        let (cpu, _) = run(&[0x06, 0x03, 0x05, 0x20, 0xFD, 0x76]);
        assert_eq!(cpu.regs.b, 0x00);
        assert!(cpu.regs.flag(flag::Z));
    }

    #[test]
    fn push_pop_roundtrip() {
        // LD SP,0x100 ; LD HL,0xBEEF ; PUSH HL ; POP BC ; HALT
        let (cpu, _) = run(&[
            0x31, 0x00, 0x01, // LD SP,0x100
            0x21, 0xEF, 0xBE, // LD HL,0xBEEF
            0xE5, // PUSH HL
            0xC1, // POP BC
            0x76, // HALT
        ]);
        assert_eq!(cpu.regs.bc(), 0xBEEF);
    }
}
