// SPDX-License-Identifier: MIT
// Copyright (c) 2026 David Goldfarb <deg@degel.com>

//! Interrupt delivery: maskable interrupts (modes 0, 1, 2) and the NMI.
//!
//! The CPU does not poll for interrupts itself — there is no interrupt *source*
//! inside the CPU. Instead the surrounding machine, when it has asserted an
//! interrupt line, calls [`Cpu::interrupt`] (maskable) or [`Cpu::nmi`]
//! (non-maskable). The machine should first check [`Cpu::can_accept_interrupt`]
//! for the maskable case. This keeps interrupt *policy* (when a device raises a
//! line) in the machine and interrupt *mechanism* (what the CPU does in
//! response) here.

use crate::bus::Bus;

impl crate::cpu::Cpu {
    /// Whether a maskable interrupt would be accepted right now: interrupts must
    /// be enabled (`IFF1`) and we must not be in the one-instruction shadow of an
    /// `EI`. A machine checks this before offering an interrupt.
    pub fn can_accept_interrupt(&self) -> bool {
        self.iff1 && !self.ei_delay
    }

    /// Common entry housekeeping when an interrupt (maskable or NMI) is taken:
    /// leave the halt state and tick the refresh register for the acknowledge
    /// cycle.
    fn enter_interrupt(&mut self) {
        self.halted = false;
        self.regs.inc_r();
    }

    /// Deliver a **maskable** interrupt, returning the T-states it consumed.
    ///
    /// `bus_value` is the byte the interrupting device places on the data bus.
    /// Its use depends on the interrupt mode:
    ///
    /// * **Mode 0** — the CPU executes `bus_value` as an instruction. In
    ///   practice this is almost always a single-byte `RST n`, which is what we
    ///   support; the byte is run through the normal decoder.
    /// * **Mode 1** — the device byte is ignored and the CPU does `RST 38h`.
    /// * **Mode 2** — a vectored interrupt: the 16-bit handler address is read
    ///   from `(I << 8) | (bus_value & 0xFE)` and called.
    ///
    /// The caller is responsible for only delivering when
    /// [`Cpu::can_accept_interrupt`] is true; this method also guards on `IFF1`
    /// and returns 0 if interrupts are disabled.
    pub fn interrupt(&mut self, bus: &mut impl Bus, bus_value: u8) -> u32 {
        if !self.iff1 {
            return 0;
        }
        self.enter_interrupt();
        self.iff1 = false;
        self.iff2 = false;

        match self.im {
            0 => {
                // Execute the bus value as an opcode (typically RST n).
                let r = self.execute(bus_value, bus);
                // The acknowledge adds two cycles over the executed instruction.
                self.cycles += 2;
                r.cycles + 2
            }
            1 => {
                self.push16(bus, self.regs.pc);
                self.regs.pc = 0x0038;
                self.cycles += 13;
                13
            }
            _ => {
                let vector = (u16::from(self.regs.i) << 8) | u16::from(bus_value & 0xFE);
                let addr = bus.read16(vector);
                self.push16(bus, self.regs.pc);
                self.regs.pc = addr;
                self.cycles += 19;
                19
            }
        }
    }

    /// Deliver a **non-maskable** interrupt, returning the T-states consumed.
    ///
    /// The NMI is always accepted. `IFF1` is copied into `IFF2` (so the prior
    /// enable state can be restored by `RETN`) and then cleared, and the CPU
    /// calls the fixed handler at `0x0066`.
    pub fn nmi(&mut self, bus: &mut impl Bus) -> u32 {
        self.enter_interrupt();
        self.iff2 = self.iff1;
        self.iff1 = false;
        self.push16(bus, self.regs.pc);
        self.regs.pc = 0x0066;
        self.cycles += 11;
        11
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::PlainBus;
    use crate::cpu::{Cpu, Stop};

    fn cpu_at(pc: u16) -> (Cpu, PlainBus) {
        let mut cpu = Cpu::new();
        cpu.regs.pc = pc;
        cpu.regs.sp = 0xFFFE;
        (cpu, PlainBus::new())
    }

    #[test]
    fn disabled_interrupt_is_ignored() {
        let (mut cpu, mut bus) = cpu_at(0x1000);
        cpu.iff1 = false;
        assert!(!cpu.can_accept_interrupt());
        let cycles = cpu.interrupt(&mut bus, 0xFF);
        assert_eq!(cycles, 0);
        assert_eq!(cpu.regs.pc, 0x1000); // unchanged
    }

    #[test]
    fn mode1_calls_0038() {
        let (mut cpu, mut bus) = cpu_at(0x1234);
        cpu.iff1 = true;
        cpu.im = 1;
        cpu.interrupt(&mut bus, 0x00);
        assert_eq!(cpu.regs.pc, 0x0038);
        assert!(!cpu.iff1); // interrupts disabled on accept
        // Return address pushed.
        assert_eq!(bus.read16(cpu.regs.sp), 0x1234);
    }

    #[test]
    fn mode2_vectors_through_table() {
        let (mut cpu, mut bus) = cpu_at(0x2000);
        cpu.iff1 = true;
        cpu.im = 2;
        cpu.regs.i = 0x80;
        // Vector for bus value 0x0E -> table at 0x800E holds 0x4000.
        bus.write(0x800E, 0x00);
        bus.write(0x800F, 0x40);
        cpu.interrupt(&mut bus, 0x0F); // low bit masked off
        assert_eq!(cpu.regs.pc, 0x4000);
        assert_eq!(bus.read16(cpu.regs.sp), 0x2000);
    }

    #[test]
    fn mode0_executes_rst() {
        let (mut cpu, mut bus) = cpu_at(0x3000);
        cpu.iff1 = true;
        cpu.im = 0;
        cpu.interrupt(&mut bus, 0xFF); // RST 38h
        assert_eq!(cpu.regs.pc, 0x0038);
        assert_eq!(bus.read16(cpu.regs.sp), 0x3000);
    }

    #[test]
    fn nmi_calls_0066_and_saves_iff() {
        let (mut cpu, mut bus) = cpu_at(0x5000);
        cpu.iff1 = true;
        cpu.nmi(&mut bus);
        assert_eq!(cpu.regs.pc, 0x0066);
        assert!(!cpu.iff1);
        assert!(cpu.iff2); // saved copy of the old IFF1
    }

    #[test]
    fn nmi_unhalts() {
        let (mut cpu, mut bus) = cpu_at(0x6000);
        cpu.halted = true;
        cpu.nmi(&mut bus);
        assert!(!cpu.halted);
    }

    #[test]
    fn ei_shadows_one_instruction() {
        // EI ; NOP ; NOP — interrupts must not be acceptable until after the NOP
        // immediately following EI.
        let mut cpu = Cpu::new();
        let mut bus = PlainBus::new();
        bus.load(0x0000, &[0xFB, 0x00, 0x00]); // EI ; NOP ; NOP
        assert!(!cpu.can_accept_interrupt()); // before EI

        cpu.step(&mut bus); // execute EI
        assert!(cpu.iff1);
        assert!(!cpu.can_accept_interrupt()); // still shadowed right after EI

        cpu.step(&mut bus); // execute the NOP after EI
        assert!(cpu.can_accept_interrupt()); // now acceptable
    }

    #[test]
    fn accepted_interrupt_resumes_after_halt() {
        // HALT then an interrupt: the CPU leaves halt and the return address is
        // the instruction after HALT.
        let mut cpu = Cpu::new();
        let mut bus = PlainBus::new();
        bus.load(0x0000, &[0x76]); // HALT at 0x0000
        cpu.regs.pc = 0x0000;
        cpu.regs.sp = 0xFFFE;
        cpu.iff1 = true;
        cpu.im = 1;
        assert_eq!(cpu.step(&mut bus).stop, Stop::Halted);
        assert!(cpu.halted);
        cpu.interrupt(&mut bus, 0x00);
        assert!(!cpu.halted);
        assert_eq!(cpu.regs.pc, 0x0038);
        assert_eq!(bus.read16(cpu.regs.sp), 0x0001); // resume past HALT
    }
}
