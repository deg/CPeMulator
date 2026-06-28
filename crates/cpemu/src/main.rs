//! `cpemu` — CLI entry point. Boots the genuine CP/M 2.2 on the virtual machine
//! with the host terminal as the console.

use cpemu::bios::cold_boot;
use cpemu::config::MachineConfig;
use cpemu::console::TerminalConsole;
use cpemu::disk::Disk;
use cpemu::executor::RunExit;

/// The genuine CP/M 2.2 CCP and BDOS, assembled at our memory-map origins.
const CCP: &[u8] = include_bytes!("../../../third-party/cpm22/ccp.bin");
const BDOS: &[u8] = include_bytes!("../../../third-party/cpm22/bdos.bin");

fn main() {
    println!("cpemu — CP/M 2.2. Booting drive A:   (press Ctrl-\\ to quit)\n");

    let console = match TerminalConsole::new() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("cpemu: needs an interactive terminal (could not enter raw mode: {e})");
            std::process::exit(1);
        }
    };

    let disk = Disk::make_system_disk(CCP, BDOS);
    let mut exec = cold_boot(MachineConfig::virtual_default(), disk, console);

    let exit = loop {
        if exec.bios.eof {
            break "input closed";
        }
        match exec.step() {
            None => {}
            Some(RunExit::Halted) => break "halted",
            Some(RunExit::Stopped) => break "stopped",
            Some(RunExit::Illegal(op)) => {
                // Restore the terminal before printing the diagnostic.
                drop(exec);
                eprintln!("\r\ncpemu: illegal opcode {op:#04X}");
                std::process::exit(1);
            }
        }
    };

    drop(exec); // restores cooked terminal mode
    println!("\r\ncpemu: {exit}.");
}
