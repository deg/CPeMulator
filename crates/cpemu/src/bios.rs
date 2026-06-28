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
use crate::disk::{DPB_8INCH, Disk, SECTOR_SIZE};
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
}

/// The HLE BIOS state.
pub struct CpmBios<C: Console> {
    config: MachineConfig,
    /// The host console.
    pub console: C,
    /// Drive A's disk image.
    disk: Disk,
    // Current disk I/O state, set by SETTRK/SETSEC/SETDMA.
    track: usize,
    sector: usize,
    dma: u16,
    drive: u8,
    /// Set when the console reaches end of input (CONIN got `None`). A headless
    /// driver uses this to know the guest is waiting for input that won't come.
    pub eof: bool,
}

impl<C: Console> CpmBios<C> {
    /// Create a BIOS for the given machine, console, and (drive A) disk.
    pub fn new(config: MachineConfig, console: C, disk: Disk) -> Self {
        Self {
            config,
            console,
            disk,
            track: 0,
            sector: 0,
            dma: 0x0080,
            drive: 0,
            eof: false,
        }
    }

    /// Address of the Disk Parameter Header (just above the jump table).
    fn dph_addr(&self) -> u16 {
        self.config.bios_base.wrapping_add(0x0100)
    }

    /// Write the static BIOS data structures (DPH → DPB, plus the directory
    /// buffer and allocation/checksum vectors) into guest memory. Call once,
    /// before running.
    pub fn install(&self, mem: &mut Memory) {
        let dph = self.dph_addr();
        let dpb = dph + 0x10;
        let dirbuf = dph + 0x20;
        let alv = dph + 0xA0;
        let csv = dph + 0xC0;

        for (i, &b) in DPB_8INCH.iter().enumerate() {
            mem.write(dpb + i as u16, b);
        }
        // Disk Parameter Header: XLT=0 (no skew), 3 scratch words, then the
        // pointers BDOS follows.
        mem.write16(dph, 0x0000); // XLT — identity translation
        mem.write16(dph + 8, dirbuf);
        mem.write16(dph + 10, dpb);
        mem.write16(dph + 12, csv);
        mem.write16(dph + 14, alv);
    }

    /// Load the CCP+BDOS image off the disk's system tracks into memory at the
    /// CCP base. The image is exactly the gap between the CCP base and the BIOS.
    fn load_system(&self, mem: &mut Memory) {
        let len = (self.config.bios_base - self.config.ccp_base) as usize;
        mem.load(self.config.ccp_base, &self.disk.system_area()[..len]);
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

    /// READ: transfer the current sector from disk into the DMA buffer.
    fn disk_read(&mut self, cpu: &mut Cpu, mem: &mut Memory) -> TrapAction {
        let sector = self.disk.read_sector(self.track, self.sector).to_vec();
        mem.load(self.dma, &sector);
        cpu.regs.a = 0; // success
        TrapAction::Return
    }

    /// WRITE: transfer the DMA buffer into the current sector on disk.
    fn disk_write(&mut self, cpu: &mut Cpu, mem: &mut Memory) -> TrapAction {
        let mut buf = [0u8; SECTOR_SIZE];
        for (i, b) in buf.iter_mut().enumerate() {
            *b = mem.read(self.dma.wrapping_add(i as u16));
        }
        self.disk.write_sector(self.track, self.sector, &buf);
        cpu.regs.a = 0;
        TrapAction::Return
    }
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
                cpu.regs.a = self.conin(); // CONIN
                TrapAction::Return
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
                // SELDSK: C = drive. Only drive 0 (A:) exists; return its DPH,
                // or HL=0 for any other drive.
                if cpu.regs.c == 0 {
                    self.drive = 0;
                    cpu.regs.set_hl(self.dph_addr());
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

/// Cold-boot a CP/M system: build the machine, install the BIOS structures, and
/// point the CPU at the BOOT vector so the first step loads and enters the CCP.
pub fn cold_boot<C: Console>(
    config: MachineConfig,
    disk: Disk,
    console: C,
) -> Executor<CpmBios<C>> {
    let machine = Machine::from_config(&config);
    let bios = CpmBios::new(config, console, disk);
    let mut exec = Executor::new(machine, bios);
    exec.bios.install(&mut exec.machine.mem);
    exec.cpu.regs.pc = config.bios_base; // the BOOT vector
    exec
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::executor::RunExit;

    const CCP: &[u8] = include_bytes!("../../../third-party/cpm22/ccp.bin");
    const BDOS: &[u8] = include_bytes!("../../../third-party/cpm22/bdos.bin");

    /// Boot the genuine CP/M with a scripted command line and run until the
    /// guest asks for input that will not come (EOF) or a step budget is spent.
    fn boot_and_run(command: &str) -> String {
        boot_disk(Disk::make_system_disk(CCP, BDOS), command)
    }

    /// Boot a specific disk with a scripted command line.
    fn boot_disk(disk: Disk, command: &str) -> String {
        let console = ScriptedConsole::with_input(command);
        let mut exec = cold_boot(MachineConfig::virtual_default(), disk, console);

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
}
