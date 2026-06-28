// SPDX-License-Identifier: MIT
// Copyright (c) 2026 David Goldfarb <deg@degel.com>

//! A debug monitor: breakpoints, single-step, and continue over a running
//! [`Executor`].
//!
//! This is the **execution-control core** of the debugger. It wraps an executor
//! and adds the three primitives a monitor is built on:
//!
//! * [`step`](Monitor::step) — run exactly one instruction (or one BIOS trap).
//! * [`run`](Monitor::run) — continue until a breakpoint, a stop, or a budget.
//! * breakpoint management — [`add_breakpoint`](Monitor::add_breakpoint) etc.
//!
//! Register/memory inspection and editing, disassembly, and instruction tracing
//! build on top of this; they live in the inspect/disassembly task. The executor
//! is exposed as [`exec`](Monitor::exec) so those layers (and tests) can read CPU
//! and memory state directly in the meantime.

use crate::executor::{BiosTrap, Executor, RunExit};
use std::collections::BTreeSet;

/// Why the monitor stopped advancing the machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    /// Execution reached a breakpoint at this address. The instruction there has
    /// **not** run yet — the next [`step`](Monitor::step) or [`run`](Monitor::run)
    /// resumes by executing it.
    Breakpoint(u16),
    /// The CPU executed `HALT`.
    Halted,
    /// A BIOS handler asked the machine to stop.
    Stopped,
    /// The CPU hit an opcode the core does not implement.
    Illegal(u8),
    /// [`run`](Monitor::run) reached its step budget without otherwise stopping.
    StepLimit,
}

/// A monitor wrapping an [`Executor`], adding breakpoint/step/continue control.
pub struct Monitor<T: BiosTrap> {
    /// The machine being debugged. Public so inspection layers and tests can read
    /// CPU registers and memory directly.
    pub exec: Executor<T>,
    breakpoints: BTreeSet<u16>,
}

impl<T: BiosTrap> Monitor<T> {
    /// Wrap an executor in a monitor with no breakpoints set.
    pub fn new(exec: Executor<T>) -> Self {
        Self {
            exec,
            breakpoints: BTreeSet::new(),
        }
    }

    /// The current program counter (the address of the next instruction to run).
    pub fn pc(&self) -> u16 {
        self.exec.cpu.regs.pc
    }

    // --- breakpoints -------------------------------------------------------

    /// Set a breakpoint at `addr`. Returns `true` if it was newly added.
    pub fn add_breakpoint(&mut self, addr: u16) -> bool {
        self.breakpoints.insert(addr)
    }

    /// Clear the breakpoint at `addr`. Returns `true` if one was there.
    pub fn remove_breakpoint(&mut self, addr: u16) -> bool {
        self.breakpoints.remove(&addr)
    }

    /// Remove all breakpoints.
    pub fn clear_breakpoints(&mut self) {
        self.breakpoints.clear();
    }

    /// Whether a breakpoint is set at `addr`.
    pub fn has_breakpoint(&self, addr: u16) -> bool {
        self.breakpoints.contains(&addr)
    }

    /// The breakpoint addresses, in ascending order.
    pub fn breakpoints(&self) -> impl Iterator<Item = u16> + '_ {
        self.breakpoints.iter().copied()
    }

    // --- execution control -------------------------------------------------

    /// Execute exactly one instruction, or service one BIOS trap. Returns a
    /// terminal [`StopReason`] if the CPU halted, was stopped, or hit an illegal
    /// opcode; otherwise `None`. Breakpoints do not apply to single-stepping —
    /// stepping onto (or off) a breakpoint is always allowed.
    pub fn step(&mut self) -> Option<StopReason> {
        terminal(self.exec.step())
    }

    /// Continue execution until a breakpoint is reached, the machine stops, or
    /// `max_steps` instructions have run (the budget guards against runaway loops
    /// with no breakpoint).
    ///
    /// The instruction at the current PC is always executed first, so continuing
    /// while sitting *on* a breakpoint makes progress instead of immediately
    /// re-triggering it. A breakpoint stops execution *before* its instruction
    /// runs, so [`pc`](Monitor::pc) equals the breakpoint address on return.
    pub fn run(&mut self, max_steps: u64) -> StopReason {
        for i in 0..max_steps {
            if i > 0 && self.breakpoints.contains(&self.exec.cpu.regs.pc) {
                return StopReason::Breakpoint(self.exec.cpu.regs.pc);
            }
            if let Some(reason) = terminal(self.exec.step()) {
                return reason;
            }
        }
        StopReason::StepLimit
    }
}

/// Map an executor exit to a terminal stop reason (`None` means "kept running").
fn terminal(exit: Option<RunExit>) -> Option<StopReason> {
    exit.map(|e| match e {
        RunExit::Halted => StopReason::Halted,
        RunExit::Stopped => StopReason::Stopped,
        RunExit::Illegal(op) => StopReason::Illegal(op),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::executor::TrapAction;
    use crate::machine::{Machine, TrapRegion};
    use crate::memory::Memory;
    use cpemu_cpu::Cpu;

    /// A BIOS that traps nothing (no vectors): plain CPU execution.
    struct NoBios;
    impl BiosTrap for NoBios {
        fn call(&mut self, _v: u8, _cpu: &mut Cpu, _mem: &mut Memory) -> TrapAction {
            TrapAction::Return
        }
    }

    /// Build a monitor running `program` at 0x0100 with no BIOS traps.
    fn monitor_with(program: &[u8]) -> Monitor<NoBios> {
        // An empty trap region (count 0) so nothing is ever trapped.
        let traps = TrapRegion {
            base: 0xFFFF,
            stride: 1,
            count: 0,
        };
        let mut machine = Machine::new(traps);
        machine.mem.load(0x0100, program);
        let mut exec = Executor::new(machine, NoBios);
        exec.cpu.regs.pc = 0x0100;
        Monitor::new(exec)
    }

    // LD A,1 ; LD A,2 ; LD A,3 ; HALT  — three 2-byte loads then HALT.
    //   0x0100 LD A,1   0x0102 LD A,2   0x0104 LD A,3   0x0106 HALT
    const LOADS: &[u8] = &[0x3E, 0x01, 0x3E, 0x02, 0x3E, 0x03, 0x76];

    #[test]
    fn breakpoint_management() {
        let mut m = monitor_with(&[0x76]);
        assert!(m.add_breakpoint(0x0104));
        assert!(!m.add_breakpoint(0x0104)); // already present
        assert!(m.add_breakpoint(0x0102));
        assert!(m.has_breakpoint(0x0104));
        assert_eq!(m.breakpoints().collect::<Vec<_>>(), vec![0x0102, 0x0104]); // sorted
        assert!(m.remove_breakpoint(0x0104));
        assert!(!m.remove_breakpoint(0x0104));
        m.clear_breakpoints();
        assert_eq!(m.breakpoints().count(), 0);
    }

    #[test]
    fn run_stops_before_the_breakpoint_instruction() {
        let mut m = monitor_with(LOADS);
        m.add_breakpoint(0x0104); // the LD A,3
        assert_eq!(m.run(100), StopReason::Breakpoint(0x0104));
        assert_eq!(m.pc(), 0x0104); // stopped *before* running LD A,3
        assert_eq!(m.exec.cpu.regs.a, 2); // LD A,2 ran, LD A,3 did not
    }

    #[test]
    fn step_executes_one_instruction() {
        let mut m = monitor_with(LOADS);
        assert_eq!(m.step(), None);
        assert_eq!(m.pc(), 0x0102);
        assert_eq!(m.exec.cpu.regs.a, 1);
        assert_eq!(m.step(), None);
        assert_eq!(m.exec.cpu.regs.a, 2);
    }

    #[test]
    fn continue_from_a_breakpoint_makes_progress() {
        let mut m = monitor_with(LOADS);
        m.add_breakpoint(0x0104);
        assert_eq!(m.run(100), StopReason::Breakpoint(0x0104));
        // Sitting on the breakpoint: continuing must run past it, not re-trigger.
        assert_eq!(m.run(100), StopReason::Halted);
        assert_eq!(m.exec.cpu.regs.a, 3); // LD A,3 ran this time
    }

    #[test]
    fn run_reports_halt_with_no_breakpoint() {
        let mut m = monitor_with(LOADS);
        assert_eq!(m.run(100), StopReason::Halted);
        assert_eq!(m.exec.cpu.regs.a, 3);
    }

    #[test]
    fn run_hits_the_step_budget() {
        // JR -2 spins forever (jumps back onto itself); with no breakpoint the
        // budget is what stops us.
        let mut m = monitor_with(&[0x18, 0xFE]); // JR -2
        assert_eq!(m.run(1000), StopReason::StepLimit);
    }
}
