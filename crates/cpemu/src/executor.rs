//! The executor: drives the CPU and services BIOS traps.
//!
//! The executor owns the three things a running CP/M system needs and that must
//! be kept as separate objects: the [`Cpu`], the [`Machine`] (which the CPU
//! borrows as its `Bus`), and the [`BiosTrap`] handler. Because they are
//! distinct fields, the executor can hand the handler `&mut Cpu` and
//! `&mut Memory` together — which a handler embedded in the `Machine` could
//! never get, since the CPU already holds the `Machine` during a step.
//!
//! Each iteration, the executor checks whether the CPU's PC has landed on a BIOS
//! trap. If so it calls the handler and acts on the result; otherwise it runs
//! one CPU instruction. This is high-level emulation (HLE) of the BIOS: the
//! guest never executes BIOS code, it just *jumps to where the BIOS would be*
//! and Rust does the work.

use crate::machine::Machine;
use crate::memory::Memory;
use cpemu_cpu::{Cpu, Stop};

/// What a BIOS handler wants the executor to do after servicing a trap.
pub enum TrapAction {
    /// Return to the caller (pop the return address). The common case:
    /// `CONIN`/`CONOUT` and the disk calls are ordinary subroutines.
    Return,
    /// Transfer control to `addr` without returning. The boot vectors
    /// (`BOOT`/`WBOOT`) do this — they are jumped to and never come back.
    Jump(u16),
    /// Stop the machine (e.g. a system-reset request). The executor returns
    /// [`RunExit::Stopped`].
    Stop,
}

/// Services a trapped BIOS call. The concrete implementation (with the real
/// console and disk behavior) is built in the HLE BIOS epic; the executor only
/// depends on this trait.
pub trait BiosTrap {
    /// Handle BIOS function `vector`, with full access to the CPU registers and
    /// to guest memory (for example to copy a console character out of `C`, or
    /// to DMA a disk sector into RAM).
    fn call(&mut self, vector: u8, cpu: &mut Cpu, mem: &mut Memory) -> TrapAction;
}

/// Why [`Executor::run`] returned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunExit {
    /// A BIOS handler asked the machine to stop.
    Stopped,
    /// The CPU executed `HALT` (no interrupt source is running to wake it).
    Halted,
    /// The CPU hit an opcode the core does not implement (should not happen on
    /// a correct core; surfaced rather than panicking).
    Illegal(u8),
}

/// Drives a CPU, a machine, and a BIOS handler together.
pub struct Executor<T: BiosTrap> {
    /// The CPU.
    pub cpu: Cpu,
    /// The machine the CPU runs on (its `Bus`).
    pub machine: Machine,
    /// The BIOS trap handler.
    pub bios: T,
}

impl<T: BiosTrap> Executor<T> {
    /// Create an executor with a freshly reset CPU.
    pub fn new(machine: Machine, bios: T) -> Self {
        Self {
            cpu: Cpu::new(),
            machine,
            bios,
        }
    }

    /// Advance the machine by one unit of work: either service a BIOS trap or
    /// execute one instruction. Returns `Some(exit)` when the machine should
    /// stop, or `None` to keep going.
    pub fn step(&mut self) -> Option<RunExit> {
        if let Some(vector) = self.machine.trap_vector(self.cpu.regs.pc) {
            return match self.bios.call(vector, &mut self.cpu, &mut self.machine.mem) {
                TrapAction::Return => {
                    self.ret();
                    None
                }
                TrapAction::Jump(addr) => {
                    self.cpu.regs.pc = addr;
                    None
                }
                TrapAction::Stop => Some(RunExit::Stopped),
            };
        }

        match self.cpu.step(&mut self.machine).stop {
            Stop::Normal => None,
            Stop::Halted => Some(RunExit::Halted),
            Stop::Illegal(op) => Some(RunExit::Illegal(op)),
        }
    }

    /// Run until the machine stops, returning why.
    pub fn run(&mut self) -> RunExit {
        loop {
            if let Some(exit) = self.step() {
                return exit;
            }
        }
    }

    /// Emulate a `RET`: pop the return address off the stack into PC.
    fn ret(&mut self) {
        let sp = self.cpu.regs.sp;
        let addr = self.machine.mem.read16(sp);
        self.cpu.regs.sp = sp.wrapping_add(2);
        self.cpu.regs.pc = addr;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::TrapRegion;

    const TRAPS: TrapRegion = TrapRegion {
        base: 0x0200,
        stride: 3,
        count: 4,
    };

    /// A trap handler that records calls and maps each vector to an action.
    #[derive(Default)]
    struct DummyBios {
        return_calls: u32,
        last_vector: Option<u8>,
    }

    impl BiosTrap for DummyBios {
        fn call(&mut self, vector: u8, _cpu: &mut Cpu, _mem: &mut Memory) -> TrapAction {
            self.last_vector = Some(vector);
            match vector {
                0 => {
                    self.return_calls += 1;
                    TrapAction::Return
                }
                1 => TrapAction::Jump(0x0300),
                _ => TrapAction::Stop,
            }
        }
    }

    fn exec_with(program_at_100: &[u8]) -> Executor<DummyBios> {
        let mut machine = Machine::new(TRAPS);
        machine.mem.load(0x0100, program_at_100);
        let mut e = Executor::new(machine, DummyBios::default());
        e.cpu.regs.pc = 0x0100;
        e
    }

    #[test]
    fn return_action_resumes_after_call() {
        // LD SP,0x0FF0 ; CALL 0x0200 (vector 0) ; LD A,0x42 ; HALT
        let mut e = exec_with(&[
            0x31, 0xF0, 0x0F, // LD SP,0x0FF0
            0xCD, 0x00, 0x02, // CALL 0x0200  -> trap vector 0 -> Return
            0x3E, 0x42, // LD A,0x42 (only runs if control returned)
            0x76, // HALT
        ]);
        assert_eq!(e.run(), RunExit::Halted);
        assert_eq!(e.bios.return_calls, 1);
        assert_eq!(e.cpu.regs.a, 0x42); // returned and executed past the CALL
        assert_eq!(e.cpu.regs.sp, 0x0FF0); // stack balanced by the emulated RET
    }

    #[test]
    fn jump_action_transfers_control_without_returning() {
        // LD SP,0x0FF0 ; CALL 0x0203 (vector 1 -> Jump 0x0300) ; (skipped) HALT
        let mut e = exec_with(&[
            0x31, 0xF0, 0x0F, // LD SP,0x0FF0
            0xCD, 0x03, 0x02, // CALL 0x0203 -> trap vector 1 -> Jump 0x0300
            0x76, // HALT (should be skipped)
        ]);
        e.machine.mem.load(0x0300, &[0x3E, 0x99, 0x76]); // LD A,0x99 ; HALT
        assert_eq!(e.run(), RunExit::Halted);
        assert_eq!(e.cpu.regs.a, 0x99); // landed at the jump target
        assert_eq!(e.bios.last_vector, Some(1));
        // The jump did not pop the stack: SP still reflects the CALL's push.
        assert_eq!(e.cpu.regs.sp, 0x0FEE);
    }

    #[test]
    fn stop_action_ends_the_run() {
        // LD SP,0x0FF0 ; CALL 0x0206 (vector 2 -> Stop)
        let mut e = exec_with(&[
            0x31, 0xF0, 0x0F, // LD SP,0x0FF0
            0xCD, 0x06, 0x02, // CALL 0x0206 -> trap vector 2 -> Stop
        ]);
        assert_eq!(e.run(), RunExit::Stopped);
        assert_eq!(e.bios.last_vector, Some(2));
    }
}
