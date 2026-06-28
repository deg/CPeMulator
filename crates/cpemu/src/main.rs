// SPDX-License-Identifier: MIT
// Copyright (c) 2026 David Goldfarb <deg@degel.com>

//! `cpemu` — CLI entry point. Boots the genuine CP/M 2.2 on the virtual machine
//! with the host terminal as the console.

use cpemu::bios::{Console, cold_boot};
use cpemu::config::MachineConfig;
use cpemu::console::TerminalConsole;
use cpemu::disk::{self, Disk, DiskFormat};
use cpemu::executor::RunExit;
use cpemu::terminal::TerminalMode;
use cpemu::throttle::Throttle;
use std::path::Path;

/// The genuine CP/M 2.2 CCP and BDOS, assembled at our memory-map origins.
const CCP: &[u8] = include_bytes!("../../../third-party/cpm22/ccp.bin");
const BDOS: &[u8] = include_bytes!("../../../third-party/cpm22/bdos.bin");

const HELP: &str = "\
cpemu — a from-scratch Z80 CP/M 2.2 emulator

USAGE:
    cargo run -- [OPTIONS] [FILES...]
    cpemu [OPTIONS] [FILES...]

Boots genuine CP/M 2.2 on a virtual Z80. Any FILES are copied onto drive A:
before booting, so you can DIR and run them (e.g. `cargo run -- ZORK1.COM`).

OPTIONS:
    --mhz=N     Set the CPU clock in MHz (default 4). Lower it if a game runs
                too fast, e.g. --mhz=2.
    --fast      Run unthrottled (as fast as the host allows).
    --raw       Disable ADM-3A terminal translation (pass output through).
    --upper     Fold typed input to UPPERCASE (for old programs that expect
                capital letters).
    --no-map-delete
                Keep CP/M's authentic RUBOUT: the Delete key deletes a char but
                re-echoes it. By default Delete is mapped to Backspace so it
                erases cleanly.
    --floppy    Use a small 8\" floppy (~241 KB) instead of the default 8 MB
                disk.
    --disk=PATH Mount a .dsk disk image as drive A and save changes back to it
                on exit. If PATH does not exist, a fresh disk is created there.
                An existing image's size selects its format (--floppy ignored).
    --help, -h  Show this help and exit.

KEYS (while running):
    Ctrl-\\      Quit the emulator (always works).
    Ctrl-C      CP/M warm boot — back to the A> prompt (when the program lets it).

SCRIPTS:
    scripts/fetch-software.sh   Download a library of classic CP/M software.
    scripts/load-all.sh         Boot with that whole library on drive A:.

Inside CP/M, type PROGRAMS.TXT (TYPE PROGRAMS.TXT) for a guide to what's loaded.";

/// CP/M marks the end of a text file with `^Z` (0x1A). Tools that read to that
/// marker treat a raw host file's trailing zero-padding as garbage — most sharply
/// the CCP's SUBMIT mechanism, which then writes an empty `$$$.SUB` and chains
/// nothing. Host binaries (`.COM`, `.DAT`, …) must stay byte-exact, so we only
/// append `^Z` to `.SUB` batch files, which are always text and fail silently
/// without it. (A `.SUB` authored in-world with `ED` is already terminated.)
fn terminate_if_submit(name: &str, mut bytes: Vec<u8>) -> Vec<u8> {
    const EOF: u8 = 0x1A;
    if name.to_ascii_uppercase().ends_with(".SUB") && bytes.last() != Some(&EOF) {
        bytes.push(EOF);
    }
    bytes
}

fn main() {
    // Flags: terminal/CPU options plus host files to place on drive A: before
    // booting. See HELP (or `--help`).
    let mut mode = TerminalMode::Adm3a;
    let mut mhz = 4.0_f64;
    let mut throttled = true;
    let mut upper = false;
    let mut map_delete = true;
    let mut format = DiskFormat::hard_8mb();
    let mut disk_path: Option<String> = None;
    let mut files = Vec::new();
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--help" | "-h" => {
                println!("{HELP}");
                return;
            }
            "--raw" => mode = TerminalMode::Raw,
            "--fast" => throttled = false,
            "--upper" => upper = true,
            "--no-map-delete" => map_delete = false,
            "--floppy" => format = DiskFormat::floppy_8inch(),
            _ if arg.starts_with("--disk=") => disk_path = Some(arg[7..].to_string()),
            _ if arg.starts_with("--mhz=") => match arg[6..].parse::<f64>() {
                Ok(n) if n > 0.0 => mhz = n,
                _ => eprintln!("  ignoring bad --mhz value: {}", &arg[6..]),
            },
            _ if arg.starts_with("--") => {
                eprintln!("cpemu: unknown option {arg} (try --help)");
                std::process::exit(2);
            }
            _ => files.push(arg),
        }
    }

    println!("cpemu — CP/M 2.2.   (press Ctrl-\\ to quit)");

    // Mount an existing image (its size picks the format), or make a fresh
    // bootable disk — written back to --disk on exit if one was named.
    let mut disk = match disk_path.as_deref() {
        Some(p) if Path::new(p).exists() => match Disk::load_image(Path::new(p)) {
            Ok(d) => {
                println!("  mounted {p} ({} bytes)", d.image().len());
                d
            }
            Err(e) => {
                eprintln!("cpemu: cannot mount {p}: {e}");
                std::process::exit(1);
            }
        },
        _ => Disk::make_system_disk_with(format, CCP, BDOS),
    };
    for path in &files {
        let name = std::path::Path::new(path)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        match std::fs::read(path) {
            Ok(bytes) => {
                let bytes = terminate_if_submit(&name, bytes);
                match disk.add_file(&name, &bytes) {
                    Ok(()) => println!("  loaded {} onto A: ({} bytes)", name, bytes.len()),
                    Err(e) => eprintln!("  cannot add {path}: {e:?}"),
                }
            }
            Err(e) => eprintln!("  cannot read {path}: {e}"),
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

    let console = match TerminalConsole::new(mode, upper, map_delete) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("cpemu: needs an interactive terminal (could not enter raw mode: {e})");
            std::process::exit(1);
        }
    };

    let mut exec = cold_boot(MachineConfig::virtual_default(), disk, console);

    let exit = loop {
        if exec.bios.console.wants_quit() {
            break "quit";
        }
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

    // Snapshot the (possibly modified) disk before dropping the executor, so the
    // image can be written back in cooked-terminal mode after the drop. The
    // illegal-opcode path above exits without reaching here, so a crashed run
    // never overwrites the image.
    let image = disk_path
        .as_deref()
        .map(|_| exec.bios.disk().image().to_vec());
    drop(exec); // restores cooked terminal mode
    println!("\r\ncpemu: {exit}.");
    if let (Some(p), Some(bytes)) = (disk_path.as_deref(), image) {
        match disk::write_image(Path::new(p), &bytes) {
            Ok(()) => println!("cpemu: saved disk image to {p}"),
            Err(e) => eprintln!("cpemu: could not save {p}: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::terminate_if_submit;

    #[test]
    fn submit_files_get_a_ctrl_z() {
        // .SUB without ^Z gains one (so SUBMIT reads the commands, not padding).
        assert_eq!(
            terminate_if_submit("B.SUB", b"STAT\r\n".to_vec()),
            b"STAT\r\n\x1a"
        );
        // Already terminated -> left unchanged (no double ^Z).
        assert_eq!(terminate_if_submit("B.SUB", b"X\x1a".to_vec()), b"X\x1a");
        // Match is case-insensitive on the extension.
        assert_eq!(terminate_if_submit("b.sub", b"X".to_vec()), b"X\x1a");
        // Non-.SUB files (e.g. a binary) are left byte-exact.
        assert_eq!(terminate_if_submit("FOO.COM", vec![1, 2, 3]), vec![1, 2, 3]);
    }
}
