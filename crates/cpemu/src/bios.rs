// SPDX-License-Identifier: MIT
// Copyright (c) 2026 David Goldfarb <deg@degel.com>

//! The HLE BIOS: the Rust side of every BIOS call, plus the cold/warm boot.
//!
//! This is the concrete [`BiosTrap`] the [`Executor`] calls when the CPU lands
//! on a BIOS vector. It implements the 17 standard CP/M 2.2 BIOS entry points —
//! console, list/punch/reader, and disk — and the boot sequence that loads the
//! genuine CCP+BDOS from the disk's system tracks and hands control to the CCP.
//!
//! The BIOS data structures BDOS reads (the Disk Parameter Header and Block, the
//! directory buffer and allocation/checksum vectors) are written into guest RAM
//! just above the BIOS jump table by [`CpmBios::install`]; [`MachineConfig`]
//! fixes where everything lives.

use crate::config::MachineConfig;
use crate::disk::{Disk, SECTOR_SIZE};
use crate::executor::{BiosTrap, Executor, TrapAction};
use crate::machine::Machine;
use crate::memory::Memory;
use cpemu_cpu::Cpu;

/// The console device the BIOS talks to. Implementations provide the host side
/// (a real terminal, or a scripted buffer for tests).
pub trait Console {
    /// Read one character. Blocks until input is available for an interactive
    /// terminal; returns `None` at end of input (which the BIOS treats as a
    /// request to stop).
    fn read(&mut self) -> Option<u8>;
    /// Whether a character is ready to read without blocking (CP/M `CONST`).
    fn status(&mut self) -> bool;
    /// Write one character to the console.
    fn write(&mut self, byte: u8);

    /// Whether the user has asked to quit the emulator (e.g. the input stream
    /// closed). The run loop checks this every step, so it works even when the
    /// running program busy-polls `CONST` and never calls `CONIN`. Default: never.
    fn wants_quit(&self) -> bool {
        false
    }

    /// Whether the input source is permanently exhausted — no further input will
    /// ever arrive. CONIN returns `^Z` (and the BIOS flags EOF) in this state. An
    /// interactive terminal is at EOF only once its stream closes; a scripted
    /// console is at EOF as soon as its queue empties. Default: never.
    fn at_eof(&self) -> bool {
        false
    }

    /// Whether the user has asked to break into the debug monitor (the break
    /// key). Polled by the run loop and **cleared** by this call, so it fires
    /// once per press. Default: never.
    fn wants_break(&mut self) -> bool {
        false
    }
}

/// A scripted console for headless runs and tests: a queue of input bytes and a
/// captured output buffer.
#[derive(Default)]
pub struct ScriptedConsole {
    /// Pending input, consumed front-to-back.
    pub input: std::collections::VecDeque<u8>,
    /// Everything the guest has written.
    pub output: Vec<u8>,
}

impl ScriptedConsole {
    /// Create a console pre-loaded with `input` (e.g. a typed command line).
    pub fn with_input(input: &str) -> Self {
        Self {
            input: input.bytes().collect(),
            output: Vec::new(),
        }
    }
    /// The captured output as a lossy string.
    pub fn output_string(&self) -> String {
        String::from_utf8_lossy(&self.output).into_owned()
    }
}

impl Console for ScriptedConsole {
    fn read(&mut self) -> Option<u8> {
        self.input.pop_front()
    }
    fn status(&mut self) -> bool {
        !self.input.is_empty()
    }
    fn write(&mut self, byte: u8) {
        self.output.push(byte);
    }
    fn at_eof(&self) -> bool {
        self.input.is_empty()
    }
}

/// The HLE BIOS state.
pub struct CpmBios<C: Console> {
    config: MachineConfig,
    /// The host console.
    pub console: C,
    /// Drives A..P. Slot 0 (A) is always mounted; the rest are optional. Each
    /// disk carries its own format, so drives may differ in size.
    disks: [Option<Disk>; 16],
    /// Address of each mounted drive's Disk Parameter Header in guest RAM (0 for
    /// an unmounted drive). Filled in by [`install`](CpmBios::install).
    dph: [u16; 16],
    // Current disk I/O state, set by SELDSK/SETTRK/SETSEC/SETDMA.
    track: usize,
    sector: usize,
    dma: u16,
    drive: u8,
    /// Set when the console reaches end of input (CONIN got `None`). A headless
    /// driver uses this to know the guest is waiting for input that won't come.
    pub eof: bool,
}

impl<C: Console> CpmBios<C> {
    /// Create a BIOS with `disk` mounted as drive A. Mount more drives with
    /// [`mount`](CpmBios::mount) before calling [`install`](CpmBios::install).
    pub fn new(config: MachineConfig, console: C, disk: Disk) -> Self {
        let mut disks: [Option<Disk>; 16] = std::array::from_fn(|_| None);
        disks[0] = Some(disk);
        Self {
            config,
            console,
            disks,
            dph: [0; 16],
            track: 0,
            sector: 0,
            dma: 0x0080,
            drive: 0,
            eof: false,
        }
    }

    /// Mount `disk` as `drive` (0 = A .. 15 = P), replacing any existing mount.
    pub fn mount(&mut self, drive: u8, disk: Disk) {
        self.disks[drive as usize] = Some(disk);
    }

    /// The drive-A disk, for inspection or saving back to an image.
    pub fn disk(&self) -> &Disk {
        self.disks[0].as_ref().expect("drive A is always mounted")
    }

    /// The disk mounted as `drive`, if any.
    pub fn disk_at(&self, drive: u8) -> Option<&Disk> {
        self.disks.get(drive as usize).and_then(|d| d.as_ref())
    }

    /// Bytes of BIOS work area (DPH/DPB/dir buffer/alloc & checksum vectors) the
    /// currently mounted drives need.
    pub fn work_area_bytes(&self) -> usize {
        work_area_bytes(self.disks.iter().flatten())
    }

    /// Bytes available for the BIOS work area (from just above the jump table to
    /// the top of memory).
    pub fn work_area_capacity(&self) -> usize {
        self.config.memory_size - (self.config.bios_base as usize + 0x0100)
    }

    /// Write the static BIOS structures into guest memory: the jump table and,
    /// for each mounted drive, a Disk Parameter Header and Block plus the
    /// allocation/checksum vectors BDOS keeps. Call once, before running.
    ///
    /// Panics if the mounted drives need more work area than there is room for
    /// above the jump table; callers should check [`work_area_bytes`] against
    /// [`work_area_capacity`] first.
    ///
    /// [`work_area_bytes`]: CpmBios::work_area_bytes
    /// [`work_area_capacity`]: CpmBios::work_area_capacity
    pub fn install(&mut self, mem: &mut Memory) {
        // The BIOS jump table. Each vector is `JP <its own address>`. We trap by
        // PC, so the bytes here are never executed when BDOS calls a vector —
        // but some programs (notably MBASIC) build their console I/O by reading
        // a vector's `JP` *operand* to find the handler address, then calling
        // that. Pointing each operand at the vector itself means the address
        // they extract also traps into Rust. An empty (zero) table makes such a
        // program call address 0 (a warm boot) instead of the console.
        for n in 0..crate::config::BIOS_VECTORS {
            let addr = self.config.bios_vector(n);
            mem.write(addr, 0xC3); // JP
            mem.write16(addr + 1, addr);
        }

        // Lay out the per-drive structures with a simple bump allocator starting
        // just above the jump table. The 128-byte directory buffer is shared by
        // all drives (BDOS uses one drive at a time); each drive gets its own
        // DPH, DPB, allocation vector, and (for removable media) checksum vector.
        let base = self.config.bios_base as u32 + 0x0100;
        let ceiling = self.config.memory_size as u32;
        let mut ptr = base;

        let dirbuf = ptr as u16;
        ptr += 128;

        for drive in 0..16usize {
            self.dph[drive] = 0;
            let Some(disk) = &self.disks[drive] else {
                continue;
            };
            let fmt = *disk.format();
            let alv_len = fmt.alloc_vector_len() as u32;
            // CKS word in the DPB: bytes of checksum vector (0 for a fixed disk).
            let cks = u16::from_le_bytes([fmt.dpb[11], fmt.dpb[12]]) as u32;

            let dph = ptr as u16;
            let dpb = (ptr + 16) as u16;
            let alv = (ptr + 16 + 15) as u16;
            let csv = (ptr + 16 + 15 + alv_len) as u16;
            ptr += 16 + 15 + alv_len + cks;
            assert!(
                ptr <= ceiling,
                "BIOS work area overflow mounting drive {drive}: needs {} bytes, have {}",
                self.work_area_bytes(),
                self.work_area_capacity()
            );

            for (i, &b) in fmt.dpb.iter().enumerate() {
                mem.write(dpb + i as u16, b);
            }
            // Disk Parameter Header: XLT=0 (identity translation), 3 scratch
            // words (left zero), then the pointers BDOS follows.
            mem.write16(dph, 0x0000);
            mem.write16(dph + 8, dirbuf);
            mem.write16(dph + 10, dpb);
            mem.write16(dph + 12, csv);
            mem.write16(dph + 14, alv);
            self.dph[drive] = dph;
        }
    }

    /// Load the CCP+BDOS image off drive A's system tracks into memory at the
    /// CCP base. The image is exactly the gap between the CCP base and the BIOS.
    fn load_system(&self, mem: &mut Memory) {
        let len = (self.config.bios_base - self.config.ccp_base) as usize;
        mem.load(self.config.ccp_base, &self.disk().system_area()[..len]);
    }

    /// Set up page zero: the warm-boot jump at 0x0000 and the BDOS-entry jump at
    /// 0x0005 (which is also how BDOS locates the BIOS).
    fn setup_page_zero(&self, mem: &mut Memory) {
        mem.write(0x0000, 0xC3); // JP WBOOT
        mem.write16(0x0001, self.config.bios_vector(1));
        mem.write(0x0003, 0x00); // IOBYTE
        mem.write(0x0004, 0x00); // current drive / user
        mem.write(0x0005, 0xC3); // JP BDOS
        mem.write16(0x0006, self.config.bdos_entry());
    }

    /// Cold/warm boot: (re)load the system, set up page zero, pass the drive to
    /// the CCP in `C`, and jump to it. WBOOT reloads CCP in case a program
    /// clobbered it; the only difference is which drive we log in.
    fn enter_ccp(&mut self, cpu: &mut Cpu, mem: &mut Memory, drive: u8) -> TrapAction {
        self.load_system(mem);
        self.setup_page_zero(mem);
        self.drive = drive;
        self.dma = 0x0080;
        cpu.regs.c = drive; // C = (user << 4) | drive, user 0
        TrapAction::Jump(self.config.ccp_base)
    }

    /// CONIN: block for a character; on end of input, flag EOF and return ^Z.
    fn conin(&mut self) -> u8 {
        match self.console.read() {
            Some(c) => c,
            None => {
                self.eof = true;
                0x1A // ^Z, CP/M end-of-file
            }
        }
    }

    /// READ: transfer the current sector of the selected drive into the DMA
    /// buffer. Returns A=1 (error) if no disk is mounted there.
    fn disk_read(&mut self, cpu: &mut Cpu, mem: &mut Memory) -> TrapAction {
        match &self.disks[self.drive as usize] {
            Some(disk) => {
                let sector = disk.read_sector(self.track, self.sector).to_vec();
                mem.load(self.dma, &sector);
                cpu.regs.a = 0; // success
            }
            None => cpu.regs.a = 1, // no such drive
        }
        TrapAction::Return
    }

    /// WRITE: transfer the DMA buffer into the current sector of the selected
    /// drive. Returns A=1 (error) if no disk is mounted there.
    fn disk_write(&mut self, cpu: &mut Cpu, mem: &mut Memory) -> TrapAction {
        let mut buf = [0u8; SECTOR_SIZE];
        for (i, b) in buf.iter_mut().enumerate() {
            *b = mem.read(self.dma.wrapping_add(i as u16));
        }
        match &mut self.disks[self.drive as usize] {
            Some(disk) => {
                disk.write_sector(self.track, self.sector, &buf);
                cpu.regs.a = 0;
            }
            None => cpu.regs.a = 1, // no such drive
        }
        TrapAction::Return
    }
}

/// Bytes of BIOS work area a set of mounted disks needs: a shared 128-byte
/// directory buffer plus, per drive, a DPH (16), DPB (15), allocation vector,
/// and checksum vector (from the DPB's CKS field; 0 for fixed disks).
pub fn work_area_bytes<'a>(disks: impl Iterator<Item = &'a Disk>) -> usize {
    let mut n = 128; // shared directory buffer
    for d in disks {
        let fmt = d.format();
        let cks = u16::from_le_bytes([fmt.dpb[11], fmt.dpb[12]]) as usize;
        n += 16 + 15 + fmt.alloc_vector_len() + cks;
    }
    n
}

impl<C: Console> BiosTrap for CpmBios<C> {
    fn call(&mut self, vector: u8, cpu: &mut Cpu, mem: &mut Memory) -> TrapAction {
        match vector {
            0 => self.enter_ccp(cpu, mem, 0),          // BOOT
            1 => self.enter_ccp(cpu, mem, self.drive), // WBOOT
            2 => {
                // CONST: 0xFF if a character is ready, else 0x00.
                cpu.regs.a = if self.console.status() { 0xFF } else { 0x00 };
                TrapAction::Return
            }
            3 => {
                // CONIN. If a key is waiting, return it. If the stream is truly
                // closed, return ^Z and flag EOF (how headless runs stop). Else
                // no key yet but more may come: re-trap CONIN so the run loop
                // keeps control (to poll the break/quit keys and idle-sleep)
                // instead of blocking the host thread here. The CALL return
                // address stays on the stack across re-traps; the eventual
                // Return pops it.
                if self.console.status() {
                    cpu.regs.a = self.conin();
                    TrapAction::Return
                } else if self.console.at_eof() {
                    self.eof = true;
                    cpu.regs.a = 0x1A; // ^Z
                    TrapAction::Return
                } else {
                    TrapAction::Jump(self.config.bios_vector(3))
                }
            }
            4 | 5 => {
                self.console.write(cpu.regs.c); // CONOUT / LIST
                TrapAction::Return
            }
            6 => TrapAction::Return, // PUNCH: discard
            7 => {
                cpu.regs.a = 0x1A; // READER: end of tape
                TrapAction::Return
            }
            8 => {
                self.track = 0; // HOME
                TrapAction::Return
            }
            9 => {
                // SELDSK: C = drive (0=A). If a disk is mounted there, select it
                // and return its DPH; otherwise HL=0, which BDOS reports as a
                // select error.
                let d = cpu.regs.c as usize;
                if d < 16 && self.disks[d].is_some() {
                    self.drive = d as u8;
                    cpu.regs.set_hl(self.dph[d]);
                } else {
                    cpu.regs.set_hl(0);
                }
                TrapAction::Return
            }
            10 => {
                self.track = cpu.regs.bc() as usize; // SETTRK
                TrapAction::Return
            }
            11 => {
                self.sector = cpu.regs.bc() as usize; // SETSEC
                TrapAction::Return
            }
            12 => {
                self.dma = cpu.regs.bc(); // SETDMA
                TrapAction::Return
            }
            13 => self.disk_read(cpu, mem),  // READ
            14 => self.disk_write(cpu, mem), // WRITE
            15 => {
                cpu.regs.a = 0xFF; // LISTST: ready
                TrapAction::Return
            }
            16 => {
                // SECTRAN: identity (XLT is 0). HL = BC.
                cpu.regs.set_hl(cpu.regs.bc());
                TrapAction::Return
            }
            _ => TrapAction::Return,
        }
    }
}

/// Cold-boot a CP/M system with just drive A: build the machine, install the
/// BIOS structures, and point the CPU at the BOOT vector so the first step loads
/// and enters the CCP.
pub fn cold_boot<C: Console>(
    config: MachineConfig,
    disk: Disk,
    console: C,
) -> Executor<CpmBios<C>> {
    cold_boot_bios(config, CpmBios::new(config, console, disk))
}

/// Cold-boot an already-built BIOS, with whatever drives it has mounted. Use
/// this when more than drive A is mounted (mount them, then boot).
pub fn cold_boot_bios<C: Console>(config: MachineConfig, bios: CpmBios<C>) -> Executor<CpmBios<C>> {
    let machine = Machine::from_config(&config);
    let mut exec = Executor::new(machine, bios);
    exec.bios.install(&mut exec.machine.mem);
    exec.cpu.regs.pc = config.bios_base; // the BOOT vector
    exec
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::executor::RunExit;

    use crate::cpm22::{BDOS, CCP};

    /// Boot the genuine CP/M with a scripted command line and run until the
    /// guest asks for input that will not come (EOF) or a step budget is spent.
    fn boot_and_run(command: &str) -> String {
        boot_disk(Disk::make_system_disk(CCP, BDOS), command)
    }

    /// Boot a specific disk with a scripted command line.
    fn boot_disk(disk: Disk, command: &str) -> String {
        let console = ScriptedConsole::with_input(command);
        drive_exec(cold_boot(MachineConfig::virtual_default(), disk, console))
    }

    /// Run an executor until the guest waits for input that won't come (EOF) or a
    /// step budget is spent, returning the captured console output.
    fn drive_exec(mut exec: Executor<CpmBios<ScriptedConsole>>) -> String {
        let mut steps: u64 = 0;
        while !exec.bios.eof && steps < 50_000_000 {
            if let Some(exit) = exec.step() {
                match exit {
                    RunExit::Illegal(op) => panic!(
                        "illegal opcode {op:#04X} at pc {:#06X}\noutput:\n{}",
                        exec.cpu.regs.pc,
                        exec.bios.console.output_string()
                    ),
                    // HALT or an explicit stop ends the run.
                    RunExit::Halted | RunExit::Stopped => break,
                }
            }
            steps += 1;
        }
        exec.bios.console.output_string()
    }

    #[test]
    fn boots_to_prompt() {
        let out = boot_and_run("\r");
        assert!(out.contains("A>"), "expected an A> prompt, got:\n{out}");
    }

    #[test]
    fn dir_on_empty_disk_says_no_file() {
        // This is the real test of the disk layer: DIR scans the directory.
        let out = boot_and_run("DIR\r");
        assert!(out.contains("A>"), "no prompt:\n{out}");
        assert!(
            out.to_uppercase().contains("NO FILE"),
            "DIR on an empty disk should say NO FILE, got:\n{out}"
        );
    }

    /// A tiny CP/M `.COM`: print "HELLO" via BDOS function 9, then return to the
    /// CCP. Loaded at 0x0100; the string is at 0x0109.
    const HELLO_COM: &[u8] = &[
        0x11, 0x09, 0x01, // LD DE,0x0109
        0x0E, 0x09, // LD C,9 (BDOS print string)
        0xCD, 0x05, 0x00, // CALL 5 (BDOS)
        0xC9, // RET (back to CCP)
        b'H', b'E', b'L', b'L', b'O', 0x0D, 0x0A, b'$',
    ];

    #[test]
    fn dir_lists_an_added_file() {
        let mut disk = Disk::make_system_disk(CCP, BDOS);
        disk.add_file("HELLO.COM", HELLO_COM).unwrap();
        let out = boot_disk(disk, "DIR\r");
        assert!(
            out.to_uppercase().contains("HELLO"),
            "DIR should list the added file:\n{out}"
        );
    }

    #[test]
    fn bios_jump_table_is_populated() {
        // Each vector must be `JP <its own address>` so programs that read a
        // vector's operand to find the handler get a trapping address.
        let cfg = MachineConfig::virtual_default();
        let mut m = crate::machine::Machine::from_config(&cfg);
        CpmBios::new(cfg, ScriptedConsole::default(), Disk::blank()).install(&mut m.mem);
        let conout = cfg.bios_vector(4); // CONOUT
        assert_eq!(m.mem.read(conout), 0xC3); // JP
        assert_eq!(m.mem.read16(conout + 1), conout); // operand points at itself
    }

    #[test]
    fn program_can_call_a_bios_vector_via_its_operand() {
        // Mimics MBASIC: read the CONOUT vector's JP operand from the jump table,
        // then CALL that address to print a character. Regresses the empty-jump-
        // table bug that made MBASIC call address 0 (warm boot) and print nothing.
        #[rustfmt::skip]
        let prog: &[u8] = &[
            0x2A, 0x01, 0x00,       // LD HL,(0x0001)   ; HL = WBOOT vector = bios_base+3
            0x11, 0x0A, 0x00,       // LD DE,0x000A
            0x19,                   // ADD HL,DE        ; HL = bios_base+13 (CONOUT JP operand)
            0x7E,                   // LD A,(HL)        ; low byte of operand
            0x23,                   // INC HL
            0x66,                   // LD H,(HL)        ; high byte
            0x6F,                   // LD L,A           ; HL = CONOUT vector address
            0x22, 0x11, 0x01,       // LD (0x0111),HL   ; patch the CALL below
            0x0E, 0x58,             // LD C,'X'
            0xCD, 0x00, 0x00,       // CALL 0x0000      ; operand at 0x0111, patched above
            0xC9,                   // RET
        ];
        let mut disk = Disk::make_system_disk(CCP, BDOS);
        disk.add_file("VEC.COM", prog).unwrap();
        let out = boot_disk(disk, "VEC\r");
        assert!(out.contains('X'), "vector-via-operand call failed:\n{out}");
    }

    #[test]
    fn runs_a_program_loaded_from_disk() {
        // The full loop: host places a .COM, CP/M loads it off disk into the TPA
        // and runs it. Proves the directory, block allocation, BDOS file load,
        // and program execution all line up.
        let mut disk = Disk::make_system_disk(CCP, BDOS);
        disk.add_file("HELLO.COM", HELLO_COM).unwrap();
        let out = boot_disk(disk, "HELLO\r");
        assert!(
            out.contains("HELLO"),
            "program did not run/print from disk:\n{out}"
        );
    }

    #[test]
    fn quit_flag_stops_a_const_polling_program() {
        // Regression: a program that busy-polls CONST and never calls CONIN must
        // still be stoppable by the quit signal (Ctrl-\). Before the fix, the run
        // loop only noticed quit via CONIN, so such a program hung the emulator.
        use std::cell::Cell;
        use std::rc::Rc;

        struct QuitConsole {
            quit: Rc<Cell<bool>>,
        }
        impl Console for QuitConsole {
            fn read(&mut self) -> Option<u8> {
                None
            }
            fn status(&mut self) -> bool {
                false
            }
            fn write(&mut self, _: u8) {}
            fn wants_quit(&self) -> bool {
                self.quit.get()
            }
        }

        let cfg = MachineConfig::virtual_default();
        let quit = Rc::new(Cell::new(false));
        let bios = CpmBios::new(cfg, QuitConsole { quit: quit.clone() }, Disk::blank());
        let mut exec =
            crate::executor::Executor::new(crate::machine::Machine::from_config(&cfg), bios);
        exec.bios.install(&mut exec.machine.mem);

        // CALL CONST ; JR -5  (poll the console status forever; never read).
        let v = cfg.bios_vector(2);
        exec.machine
            .mem
            .load(0x0100, &[0xCD, v as u8, (v >> 8) as u8, 0x18, 0xFB]);
        exec.cpu.regs.pc = 0x0100;

        // Run loop as main does: it can only stop via the quit flag.
        let mut steps = 0u64;
        loop {
            if exec.bios.console.wants_quit() {
                break;
            }
            assert!(steps < 1_000_000, "quit flag was not honored (would hang)");
            exec.step();
            steps += 1;
            if steps == 5000 {
                quit.set(true); // simulate pressing the quit key
            }
        }
        assert!(steps >= 5000);
    }

    #[test]
    fn conin_retraps_while_idle_then_returns_the_byte() {
        // The interruptible-CONIN path: when no key is ready and the stream is not
        // at EOF, CONIN must re-trap itself (so the run loop keeps control) rather
        // than block or corrupt state; once a key is ready it returns it.
        struct Delayed {
            ready: bool,
        }
        impl Console for Delayed {
            fn read(&mut self) -> Option<u8> {
                Some(b'Q')
            }
            fn status(&mut self) -> bool {
                self.ready
            }
            fn write(&mut self, _: u8) {}
            // never at_eof (default)
        }
        let cfg = MachineConfig::virtual_default();
        let mut machine = Machine::from_config(&cfg);
        let mut bios = CpmBios::new(cfg, Delayed { ready: false }, Disk::blank());
        let mut cpu = Cpu::new();
        let conin = cfg.bios_vector(3);

        // Idle: re-trap back to the CONIN vector, A and EOF untouched.
        assert!(matches!(
            bios.call(3, &mut cpu, &mut machine.mem),
            TrapAction::Jump(a) if a == conin
        ));
        assert!(!bios.eof);

        // Key ready: return it.
        bios.console.ready = true;
        assert!(matches!(
            bios.call(3, &mut cpu, &mut machine.mem),
            TrapAction::Return
        ));
        assert_eq!(cpu.regs.a, b'Q');
        assert!(!bios.eof);
    }

    #[test]
    fn conin_returns_ctrl_z_at_eof() {
        // A scripted console with no input is at EOF: CONIN returns ^Z and flags
        // EOF (the signal headless runs use to stop) — it must not re-trap.
        let cfg = MachineConfig::virtual_default();
        let mut machine = Machine::from_config(&cfg);
        let mut bios = CpmBios::new(cfg, ScriptedConsole::default(), Disk::blank());
        let mut cpu = Cpu::new();
        assert!(matches!(
            bios.call(3, &mut cpu, &mut machine.mem),
            TrapAction::Return
        ));
        assert_eq!(cpu.regs.a, 0x1A);
        assert!(bios.eof);
    }

    #[test]
    fn dir_lists_files_on_a_second_drive() {
        // Mount a data disk as drive B and prove CP/M can select it and read its
        // directory — exercises SELDSK returning B's DPH and per-drive READ.
        let cfg = MachineConfig::virtual_default();
        let mut disk_b = Disk::blank();
        disk_b.add_file("ONB.TXT", b"on drive b\r\n\x1a").unwrap();
        let mut bios = CpmBios::new(
            cfg,
            ScriptedConsole::with_input("DIR B:\r"),
            Disk::make_system_disk(CCP, BDOS),
        );
        bios.mount(1, disk_b);
        let out = drive_exec(cold_boot_bios(cfg, bios));
        assert!(
            out.to_uppercase().contains("ONB"),
            "DIR B: should list drive B's file:\n{out}"
        );
    }

    #[test]
    fn work_area_budget_fits_two_hard_disks_not_three() {
        // The 8 MB allocation vector (510 bytes) is what bounds how many drives'
        // BIOS structures fit above the jump table: two 8 MB disks fit, a third
        // does not. Documents the limit the install() budget check enforces.
        let cfg = MachineConfig::virtual_default();
        let hard = || Disk::blank_with(crate::disk::DiskFormat::hard_8mb());
        let mut bios = CpmBios::new(cfg, ScriptedConsole::default(), hard());
        bios.mount(1, hard());
        assert!(
            bios.work_area_bytes() <= bios.work_area_capacity(),
            "two 8MB drives should fit ({} > {})",
            bios.work_area_bytes(),
            bios.work_area_capacity()
        );
        bios.mount(2, hard());
        assert!(
            bios.work_area_bytes() > bios.work_area_capacity(),
            "three 8MB drives should overflow the work area"
        );
    }

    #[test]
    fn monitor_breakpoint_stops_genuine_cpm_at_bdos_entry() {
        // The execution-control core (cpemu-6o2.1) driving the real CCP/BDOS: a
        // breakpoint at the BDOS entry must halt the machine there, since the CCP
        // calls BDOS almost immediately (to reset disks and print the prompt).
        use crate::monitor::{Monitor, StopReason};
        let cfg = MachineConfig::virtual_default();
        let exec = cold_boot(
            cfg,
            Disk::make_system_disk(CCP, BDOS),
            ScriptedConsole::with_input("\r"),
        );
        let mut mon = Monitor::new(exec);
        let bdos = cfg.bdos_entry();
        mon.add_breakpoint(bdos);
        assert_eq!(mon.run(50_000_000), StopReason::Breakpoint(bdos));
        assert_eq!(mon.pc(), bdos, "stopped exactly at the BDOS entry");
    }

    #[test]
    fn runs_a_program_from_an_8mb_disk() {
        // Boot the genuine CP/M on the 8 MB hard-disk format and run a program
        // off it — proving BDOS agrees with the new DPB/geometry end to end.
        let mut disk = Disk::make_system_disk_with(crate::disk::DiskFormat::hard_8mb(), CCP, BDOS);
        disk.add_file("HELLO.COM", HELLO_COM).unwrap();
        let out = boot_disk(disk, "HELLO\r");
        assert!(out.contains("HELLO"), "8MB disk program run failed:\n{out}");
    }
}
