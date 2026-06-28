//! `cpemu` — CLI entry point. Boots the genuine CP/M 2.2 on the virtual machine
//! with the host terminal as the console.

use cpemu::bios::cold_boot;
use cpemu::config::MachineConfig;
use cpemu::console::TerminalConsole;
use cpemu::disk::Disk;
use cpemu::executor::RunExit;
use cpemu::terminal::TerminalMode;
use cpemu::throttle::Throttle;

/// The genuine CP/M 2.2 CCP and BDOS, assembled at our memory-map origins.
const CCP: &[u8] = include_bytes!("../../../third-party/cpm22/ccp.bin");
const BDOS: &[u8] = include_bytes!("../../../third-party/cpm22/bdos.bin");

fn main() {
    println!("cpemu — CP/M 2.2.   (press Ctrl-\\ to quit)");

    // Flags: `--raw` disables terminal translation, `--mhz=N` sets the CPU clock
    // (default 4), `--fast` runs unthrottled. Other arguments are host files to
    // place on drive A: before booting, e.g. `cargo run -- STAT.COM ZORK1.COM`.
    let mut mode = TerminalMode::Adm3a;
    let mut mhz = 4.0_f64;
    let mut throttled = true;
    let mut disk = Disk::make_system_disk(CCP, BDOS);
    for arg in std::env::args().skip(1) {
        if arg == "--raw" {
            mode = TerminalMode::Raw;
            continue;
        }
        if arg == "--fast" {
            throttled = false;
            continue;
        }
        if let Some(v) = arg.strip_prefix("--mhz=") {
            match v.parse::<f64>() {
                Ok(n) if n > 0.0 => mhz = n,
                _ => eprintln!("  ignoring bad --mhz value: {v}"),
            }
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
    let mut throttle = throttled.then(|| Throttle::new((mhz * 1_000_000.0) as u64));
    println!(
        "Booting drive A:  ({})\n",
        if throttle.is_some() {
            format!("{mhz} MHz")
        } else {
            "unthrottled".to_string()
        }
    );

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
        if let Some(t) = &mut throttle {
            t.sync(exec.cpu.cycles);
        }
    };

    drop(exec); // restores cooked terminal mode
    println!("\r\ncpemu: {exit}.");
}
