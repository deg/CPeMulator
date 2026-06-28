// SPDX-License-Identifier: MIT
// Copyright (c) 2026 David Goldfarb <deg@degel.com>

//! CPU validation against the standard Z80 exercisers.
//!
//! The exercisers (`prelim`, `zexdoc`, `zexall`) are vintage CP/M `.COM`
//! programs that exhaustively test the CPU and print their results to the
//! console. To run them we provide the minimum CP/M environment they expect:
//!
//! * the program is loaded at `0x0100` (the CP/M "TPA") and run from there;
//! * `CALL 5` (the BDOS entry) is trapped and serviced here — function 2 prints
//!   the character in `E`, function 9 prints the `$`-terminated string at `DE`;
//! * returning to `0x0000` (a CP/M warm boot) means the program has finished.
//!
//! We capture everything the program prints and check it for the failure marker.
//! This validates the CPU end-to-end without needing any of the rest of the
//! emulator — exactly what the standalone `cpemu-cpu` crate is for.
//!
//! `prelim` is small and runs in CI. `zexdoc` and `zexall` execute billions of
//! T-states, so they are marked `#[ignore]`; run them in release with:
//!
//! ```text
//! cargo test -p cpemu-cpu --release --test exercisers -- --ignored --nocapture
//! ```

use cpemu_cpu::{Bus, Cpu, Stop};

const TPA: u16 = 0x0100; // where CP/M loads a .COM program
const BDOS: u16 = 0x0005; // BDOS entry point
const WARM_BOOT: u16 = 0x0000;

/// A 64 KB RAM bus with a captured console, standing in for a CP/M machine.
struct CpmBus {
    mem: Vec<u8>,
    console: String,
}

impl CpmBus {
    fn new() -> Self {
        let mut mem = vec![0u8; 0x1_0000];
        // Present a believable BDOS entry: `JP 0xFE00`, so a program that reads
        // the address at 0x0006 sees a sane top-of-TPA. We trap CALL 5 by PC
        // before these bytes ever execute, so the target only needs to exist.
        mem[BDOS as usize] = 0xC3;
        mem[BDOS as usize + 1] = 0x00;
        mem[BDOS as usize + 2] = 0xFE;
        mem[0xFE00] = 0xC9; // RET, just in case
        Self {
            mem,
            console: String::new(),
        }
    }

    fn load(&mut self, addr: u16, bytes: &[u8]) {
        self.mem[addr as usize..addr as usize + bytes.len()].copy_from_slice(bytes);
    }
}

impl Bus for CpmBus {
    fn read(&mut self, addr: u16) -> u8 {
        self.mem[addr as usize]
    }
    fn write(&mut self, addr: u16, val: u8) {
        self.mem[addr as usize] = val;
    }
    fn input(&mut self, _port: u16) -> u8 {
        0xFF
    }
    fn output(&mut self, _port: u16, _val: u8) {}
}

/// Service a trapped BDOS call from the current CPU/bus state.
fn bdos_call(cpu: &Cpu, bus: &mut CpmBus) {
    match cpu.regs.c {
        2 => bus.console.push(cpu.regs.e as char), // console output
        9 => {
            // Print the `$`-terminated string at DE.
            let mut addr = cpu.regs.de();
            loop {
                let ch = bus.read(addr);
                if ch == b'$' {
                    break;
                }
                bus.console.push(ch as char);
                addr = addr.wrapping_add(1);
            }
        }
        _ => {}
    }
}

/// Run a CP/M `.COM` exerciser to completion and return its console output.
fn run_exerciser(rom: &[u8]) -> String {
    let mut bus = CpmBus::new();
    bus.load(TPA, rom);

    let mut cpu = Cpu::new();
    cpu.regs.pc = TPA;
    // Stack just below the BDOS area; pre-seed a return to the warm-boot vector
    // so a final `RET` from the program is detected as completion.
    cpu.regs.sp = 0xFFFE;
    bus.write(0xFFFE, 0x00);
    bus.write(0xFFFF, 0x00);

    let mut guard: u64 = 0;
    loop {
        match cpu.regs.pc {
            WARM_BOOT => break,
            BDOS => {
                if cpu.regs.c == 0 {
                    break; // BDOS function 0: system reset
                }
                bdos_call(&cpu, &mut bus);
                // Simulate the RET that would return from BDOS.
                let lo = bus.read(cpu.regs.sp);
                let hi = bus.read(cpu.regs.sp.wrapping_add(1));
                cpu.regs.sp = cpu.regs.sp.wrapping_add(2);
                cpu.regs.pc = (u16::from(hi) << 8) | u16::from(lo);
                continue;
            }
            _ => {}
        }

        let r = cpu.step(&mut bus);
        guard += 1;
        assert!(
            guard < 50_000_000_000,
            "exerciser did not terminate; console so far:\n{}",
            bus.console
        );
        if let Stop::Illegal(op) = r.stop {
            panic!(
                "illegal opcode {op:#04X} at pc {:#06X}\nconsole so far:\n{}",
                cpu.regs.pc, bus.console
            );
        }
    }
    bus.console
}

#[test]
fn prelim() {
    let out = run_exerciser(include_bytes!("roms/prelim.com"));
    println!("{out}");
    // The preliminary test prints this line only if every check passed.
    assert!(
        out.contains("Preliminary tests complete"),
        "prelim did not report success:\n{out}"
    );
}

#[test]
#[ignore = "runs billions of T-states; use: cargo test -p cpemu-cpu --release --test exercisers -- --ignored"]
fn zexdoc() {
    let out = run_exerciser(include_bytes!("roms/zexdoc.com"));
    println!("{out}");
    assert!(
        !out.to_ascii_uppercase().contains("ERROR"),
        "zexdoc reported failures:\n{out}"
    );
    assert!(
        out.to_ascii_lowercase().contains("complete"),
        "zexdoc did not run to completion:\n{out}"
    );
}

#[test]
#[ignore = "stretch goal (undocumented flags/WZ); use: cargo test -p cpemu-cpu --release --test exercisers -- --ignored"]
fn zexall() {
    let out = run_exerciser(include_bytes!("roms/zexall.com"));
    println!("{out}");
    assert!(
        !out.to_ascii_uppercase().contains("ERROR"),
        "zexall reported failures:\n{out}"
    );
}
