//! The CPU itself: machine state and the fetch/decode/execute step.
//!
//! This task (`cpemu-yd5.1`) establishes the execution framework — register and
//! interrupt state, T-state accounting, and an *observable* step outcome — plus
//! enough of the decode loop (`NOP`, `HALT`) to exercise it. The full 8080 and
//! Z80 instruction sets land in the following tasks, filling in the opcode
//! match where unimplemented opcodes currently report [`Stop::Illegal`].

use crate::bus::Bus;
use crate::registers::Registers;

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
        self.halted = false;
    }

    /// Fetch the opcode byte at `PC`, advance `PC`, and tick the refresh
    /// register (every M1 opcode fetch increments `R`).
    fn fetch(&mut self, bus: &mut impl Bus) -> u8 {
        let opcode = bus.read(self.regs.pc);
        self.regs.pc = self.regs.pc.wrapping_add(1);
        self.regs.inc_r();
        opcode
    }

    /// Record consumed cycles and return a normal step result.
    fn done(&mut self, cycles: u32, stop: Stop) -> StepResult {
        self.cycles += u64::from(cycles);
        StepResult { cycles, stop }
    }

    /// Execute one instruction and return how many T-states it took.
    ///
    /// While halted, the CPU continues to refresh memory and burn 4 T-states
    /// per step (an internal `NOP`) until an interrupt clears `halted`.
    pub fn step(&mut self, bus: &mut impl Bus) -> StepResult {
        if self.halted {
            self.regs.inc_r();
            return self.done(4, Stop::Halted);
        }

        let opcode = self.fetch(bus);
        match opcode {
            0x00 => self.done(4, Stop::Normal), // NOP
            0x76 => {
                // HALT
                self.halted = true;
                self.done(4, Stop::Halted)
            }
            other => {
                // Not yet implemented. PC has already advanced past the opcode;
                // report it so the harness can pinpoint the gap. No panic.
                self.done(4, Stop::Illegal(other))
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

    #[test]
    fn nop_advances_pc_and_counts_cycles() {
        let mut cpu = Cpu::new();
        let mut bus = PlainBus::new();
        bus.load(0x0000, &[0x00, 0x00]); // NOP, NOP
        let r = cpu.step(&mut bus);
        assert_eq!(r.stop, Stop::Normal);
        assert_eq!(r.cycles, 4);
        assert_eq!(cpu.regs.pc, 0x0001);
        assert_eq!(cpu.cycles, 4);
        assert_eq!(cpu.regs.r, 0x01); // one fetch
    }

    #[test]
    fn halt_sets_halted_and_spins() {
        let mut cpu = Cpu::new();
        let mut bus = PlainBus::new();
        bus.load(0x0000, &[0x76]); // HALT
        let r = cpu.step(&mut bus);
        assert_eq!(r.stop, Stop::Halted);
        assert!(cpu.halted);
        assert_eq!(cpu.regs.pc, 0x0001);
        // Subsequent steps keep the CPU halted without advancing PC.
        let r2 = cpu.step(&mut bus);
        assert_eq!(r2.stop, Stop::Halted);
        assert_eq!(cpu.regs.pc, 0x0001);
    }

    #[test]
    fn unimplemented_opcode_is_reported_not_panicked() {
        let mut cpu = Cpu::new();
        let mut bus = PlainBus::new();
        bus.load(0x0000, &[0xED]); // prefix not implemented yet
        let r = cpu.step(&mut bus);
        assert_eq!(r.stop, Stop::Illegal(0xED));
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
}
