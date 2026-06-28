//! `cpemu` — CLI entry point. Boots the genuine CP/M 2.2 on the virtual machine
//! with the host terminal as the console.

use cpemu::bios::cold_boot;
use cpemu::config::MachineConfig;
use cpemu::console::TerminalConsole;
use cpemu::disk::Disk;
use cpemu::executor::RunExit;
use cpemu::terminal::TerminalMode;

/// The genuine CP/M 2.2 CCP and BDOS, assembled at our memory-map origins.
const CCP: &[u8] = include_bytes!("../../../third-party/cpm22/ccp.bin");
const BDOS: &[u8] = include_bytes!("../../../third-party/cpm22/bdos.bin");

fn main() {
    println!("cpemu — CP/M 2.2.   (press Ctrl-\\ to quit)");

    // `--raw` disables terminal translation; other arguments are host files to
    // place on drive A: before booting, e.g. `cargo run -- STAT.COM ZORK1.COM`.
    let mut mode = TerminalMode::Adm3a;
    let mut disk = Disk::make_system_disk(CCP, BDOS);
    for arg in std::env::args().skip(1) {
        if arg == "--raw" {
            mode = TerminalMode::Raw;
            continue;
        }
        let name = std::path::Path::new(&arg)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        match std::fs::read(&arg) {
            Ok(bytes) => match disk.add_file(&name, &bytes) {
                Ok(()) => println!("  loaded {} onto A: ({} bytes)", name, bytes.len()),
                Err(e) => eprintln!("  cannot add {arg}: {e:?}"),
            },
            Err(e) => eprintln!("  cannot read {arg}: {e}"),
        }
    }
    println!("Booting drive A:\n");

    let console = match TerminalConsole::new(mode) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("cpemu: needs an interactive terminal (could not enter raw mode: {e})");
            std::process::exit(1);
        }
    };

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
