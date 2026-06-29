// SPDX-License-Identifier: MIT
// Copyright (c) 2026 David Goldfarb <deg@degel.com>

//! `cpemu` — CLI entry point. Boots the genuine CP/M 2.2 on the virtual machine
//! with the host terminal as the console.

use cpemu::bios::{Console, CpmBios, cold_boot_bios, work_area_bytes};
use cpemu::config::MachineConfig;
use cpemu::console::TerminalConsole;
use cpemu::debugger::{Action, Debugger};
use cpemu::disk::{self, Disk, DiskFormat};
use cpemu::monitor::{Monitor, StopReason};
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
    --disk=[X:]PATH
                Mount a .dsk image as drive X (A..P; default A) and save changes
                back to it on exit. Repeatable, e.g. --disk=game.dsk
                --disk=B:work.dsk. If PATH does not exist a fresh disk is created
                (drive A bootable, others blank). An existing image's size picks
                its format; up to two 8 MB disks (or more smaller ones) fit.
    --help, -h  Show this help and exit.

KEYS (while running):
    Ctrl-\\      Break into the debug monitor (? for help; g resumes, q quits).
    Ctrl-C      CP/M warm boot — back to the A> prompt (when the program lets it).

SCRIPTS:
    scripts/fetch-software.sh   Download a library of classic CP/M software.
    scripts/load-all.sh         Boot with that whole library on drive A:.

Inside CP/M, type PROGRAMS.TXT (TYPE PROGRAMS.TXT) for a guide to what's loaded.";

/// Parse a `--disk` value: an optional `X:` drive-letter prefix (A..P) followed
/// by a path. No prefix means drive A. `--disk=B:work.dsk` -> (1, "work.dsk").
fn parse_disk_spec(val: &str) -> (u8, String) {
    let b = val.as_bytes();
    if b.len() > 2 && b[1] == b':' {
        let c = b[0].to_ascii_uppercase();
        if (b'A'..=b'P').contains(&c) {
            return (c - b'A', val[2..].to_string());
        }
    }
    (0, val.to_string())
}

/// Drive number to its CP/M letter (0 -> 'A').
fn drive_letter(drive: u8) -> char {
    (b'A' + drive) as char
}

/// Extensions that name CP/M *text* files, which end at a `^Z` marker. A raw host
/// file lacks it, so `TYPE` (and SUBMIT, ED, …) read past the content into the
/// disk's zero-padding as garbage. We append `^Z` to these on import; everything
/// else — `.COM`, `.DAT`, and other binaries — must stay byte-exact, so the rule
/// is a strict extension allow-list, not a guess.
const TEXT_EXTENSIONS: &[&str] = &["SUB", "TXT", "DOC", "PRN", "ASC"];

/// Append a `^Z` end-of-text marker to a host file if its extension names a CP/M
/// text type and it isn't already terminated. (A file authored in-world with `ED`
/// already has one.) Binaries are returned untouched.
fn terminate_text_file(name: &str, mut bytes: Vec<u8>) -> Vec<u8> {
    const EOF: u8 = 0x1A;
    let ext = name.rsplit('.').next().unwrap_or("").to_ascii_uppercase();
    let is_text = name.contains('.') && TEXT_EXTENSIONS.contains(&ext.as_str());
    if is_text && bytes.last() != Some(&EOF) {
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
    let mut disk_mounts: Vec<(u8, String)> = Vec::new();
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
            _ if arg.starts_with("--disk=") => disk_mounts.push(parse_disk_spec(&arg[7..])),
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

    println!("cpemu — CP/M 2.2.   (Ctrl-\\ for the debug monitor)");

    // Resolve each --disk mount: load an existing image (its size picks the
    // format) or create a fresh disk (drive A bootable, other drives blank).
    // Drive A defaults to a fresh bootable disk if not named.
    let mut disk_a = Disk::make_system_disk_with(format, CCP, BDOS);
    let mut extras: Vec<(u8, Disk)> = Vec::new();
    for (drive, path) in &disk_mounts {
        let exists = Path::new(path).exists();
        let disk = if exists {
            match Disk::load_image(Path::new(path)) {
                Ok(d) => d,
                Err(e) => {
                    eprintln!("cpemu: cannot mount {path}: {e}");
                    std::process::exit(1);
                }
            }
        } else if *drive == 0 {
            Disk::make_system_disk_with(format, CCP, BDOS)
        } else {
            Disk::blank_with(format)
        };
        println!(
            "  {} {}: {} {path}",
            if exists { "mounted" } else { "created" },
            drive_letter(*drive),
            if exists { "<-" } else { "->" },
        );
        if *drive == 0 {
            disk_a = disk;
        } else {
            extras.push((*drive, disk));
        }
    }
    for path in &files {
        let name = std::path::Path::new(path)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        match std::fs::read(path) {
            Ok(bytes) => {
                let bytes = terminate_text_file(&name, bytes);
                match disk_a.add_file(&name, &bytes) {
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

    // Check the mounted drives' BIOS structures fit above the jump table before
    // touching the terminal, so the error prints in cooked mode.
    let config = MachineConfig::virtual_default();
    let needed = work_area_bytes(std::iter::once(&disk_a).chain(extras.iter().map(|(_, d)| d)));
    let capacity = config.memory_size - (config.bios_base as usize + 0x0100);
    if needed > capacity {
        eprintln!(
            "cpemu: the mounted drives need {needed} bytes of BIOS work area but only \
             {capacity} are available — mount fewer or smaller disks."
        );
        std::process::exit(1);
    }

    let console = match TerminalConsole::new(mode, upper, map_delete) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("cpemu: needs an interactive terminal (could not enter raw mode: {e})");
            std::process::exit(1);
        }
    };

    let mut bios = CpmBios::new(config, console, disk_a);
    for (drive, disk) in extras {
        bios.mount(drive, disk);
    }
    let mut mon = Monitor::new(cold_boot_bios(config, bios));
    let mut dbg = Debugger::new();
    let conin = config.bios_vector(3);

    // Run the CP/M session, dropping into the debug monitor on the break key
    // (Ctrl-\) or a breakpoint, until the machine stops or the user quits.
    let outcome = loop {
        match run_until_pause(&mut mon, conin, &mut throttle) {
            Pause::Eof => break Outcome::Eof,
            Pause::Halted => break Outcome::Halted,
            Pause::Stopped => break Outcome::Stopped,
            Pause::Illegal(op) => break Outcome::Illegal(op),
            Pause::Break => {
                let banner = Debugger::banner(mon.pc());
                if monitor_repl(&mut mon, &mut dbg, &banner) == Action::Quit {
                    break Outcome::Quit;
                }
            }
            Pause::Breakpoint(addr) => {
                let banner = format!("-- breakpoint at {addr:04X} --");
                if monitor_repl(&mut mon, &mut dbg, &banner) == Action::Quit {
                    break Outcome::Quit;
                }
            }
        }
    };

    // Snapshot each path-backed drive before dropping the monitor, so images save
    // in cooked-terminal mode after the drop. A crash (illegal opcode) saves
    // nothing, so it can't overwrite an image with corrupt state.
    let saves: Vec<(String, Vec<u8>)> = if matches!(outcome, Outcome::Illegal(_)) {
        Vec::new()
    } else {
        disk_mounts
            .iter()
            .filter_map(|(drive, path)| {
                mon.exec
                    .bios
                    .disk_at(*drive)
                    .map(|d| (path.clone(), d.image().to_vec()))
            })
            .collect()
    };
    drop(mon); // restores cooked terminal mode

    let exit = match outcome {
        Outcome::Quit => "quit",
        Outcome::Eof => "input closed",
        Outcome::Halted => "halted",
        Outcome::Stopped => "stopped",
        Outcome::Illegal(op) => {
            eprintln!("\r\ncpemu: illegal opcode {op:#04X}");
            std::process::exit(1);
        }
    };
    println!("\r\ncpemu: {exit}.");
    for (path, bytes) in saves {
        match disk::write_image(Path::new(&path), &bytes) {
            Ok(()) => println!("cpemu: saved disk image to {path}"),
            Err(e) => eprintln!("cpemu: could not save {path}: {e}"),
        }
    }
}

/// Why the run loop paused.
enum Pause {
    /// The user pressed the break key (Ctrl-\).
    Break,
    /// Execution reached a breakpoint at this address.
    Breakpoint(u16),
    Halted,
    Stopped,
    Illegal(u8),
    /// The input stream closed.
    Eof,
}

/// How the session ended.
enum Outcome {
    Quit,
    Eof,
    Halted,
    Stopped,
    Illegal(u8),
}

/// Step the CP/M session until it should pause: the break key, a breakpoint, a
/// machine stop, or end of input. Idle-sleeps while parked at CONIN so an idle
/// prompt doesn't busy-wait the host CPU.
fn run_until_pause<C: Console>(
    mon: &mut Monitor<CpmBios<C>>,
    conin: u16,
    throttle: &mut Option<Throttle>,
) -> Pause {
    // Don't immediately re-trigger a breakpoint we're resuming from: the first
    // instruction always runs before breakpoints are checked again.
    let mut skip_bp = true;
    loop {
        if mon.exec.bios.console.wants_break() {
            return Pause::Break;
        }
        if mon.exec.bios.console.wants_quit() || mon.exec.bios.eof {
            return Pause::Eof;
        }
        if !skip_bp && mon.has_breakpoint(mon.pc()) {
            return Pause::Breakpoint(mon.pc());
        }
        skip_bp = false;
        if mon.pc() == conin && !mon.exec.bios.console.status() {
            std::thread::sleep(std::time::Duration::from_millis(2));
            continue;
        }
        match mon.step() {
            None => {}
            Some(StopReason::Halted) => return Pause::Halted,
            Some(StopReason::Stopped) => return Pause::Stopped,
            Some(StopReason::Illegal(op)) => return Pause::Illegal(op),
            // step() never reports these (only run() does); ignore for completeness.
            Some(StopReason::Breakpoint(_) | StopReason::StepLimit) => {}
        }
        if let Some(t) = throttle {
            t.sync(mon.exec.cpu.cycles);
        }
    }
}

/// The interactive debug-monitor prompt: show the entry banner and registers,
/// then read and run commands until the user resumes (`g`) or quits (`q`).
fn monitor_repl<C: Console>(
    mon: &mut Monitor<CpmBios<C>>,
    dbg: &mut Debugger,
    entry: &str,
) -> Action {
    write_console(&mut mon.exec.bios.console, &format!("\r\n{entry}\r\n"));
    let regs = dbg.command(mon, "r").0;
    write_console(&mut mon.exec.bios.console, &regs);
    loop {
        write_console(&mut mon.exec.bios.console, "- ");
        let Some(line) = read_line(&mut mon.exec.bios.console) else {
            return Action::Quit; // the input stream closed
        };
        let (out, action) = dbg.command(mon, line.trim());
        write_console(&mut mon.exec.bios.console, &out);
        if action != Action::Stay {
            return action;
        }
    }
}

/// Write a string to the console byte by byte (through the terminal translation).
fn write_console<C: Console>(con: &mut C, s: &str) {
    for b in s.bytes() {
        con.write(b);
    }
}

/// Read a line from the console in raw mode, echoing input and handling
/// backspace. Returns `None` if the input stream closes.
fn read_line<C: Console>(con: &mut C) -> Option<String> {
    let mut line = String::new();
    loop {
        let b = con.read()?;
        match b {
            b'\r' | b'\n' => {
                con.write(b'\r');
                con.write(b'\n');
                return Some(line);
            }
            0x08 | 0x7F if !line.is_empty() => {
                line.pop();
                con.write(0x08);
                con.write(b' ');
                con.write(0x08);
            }
            0x20..=0x7E => {
                con.write(b);
                line.push(b as char);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_disk_spec, terminate_text_file};

    #[test]
    fn disk_spec_parses_drive_prefix() {
        assert_eq!(parse_disk_spec("foo.dsk"), (0, "foo.dsk".to_string())); // no prefix -> A
        assert_eq!(parse_disk_spec("A:foo.dsk"), (0, "foo.dsk".to_string()));
        assert_eq!(parse_disk_spec("B:work.dsk"), (1, "work.dsk".to_string()));
        assert_eq!(parse_disk_spec("p:last.dsk"), (15, "last.dsk".to_string())); // lowercase ok
        // A path that merely contains a colon but no single-letter prefix is a path.
        assert_eq!(parse_disk_spec("dir:name"), (0, "dir:name".to_string()));
        assert_eq!(parse_disk_spec("Z:bad.dsk"), (0, "Z:bad.dsk".to_string())); // Z is past P
    }

    #[test]
    fn text_files_get_a_ctrl_z() {
        // .SUB and .TXT (and other text types) without ^Z gain one, so SUBMIT/TYPE
        // read the content and not the disk's zero-padding.
        assert_eq!(
            terminate_text_file("B.SUB", b"STAT\r\n".to_vec()),
            b"STAT\r\n\x1a"
        );
        assert_eq!(terminate_text_file("READ.TXT", b"hi".to_vec()), b"hi\x1a");
        // Already terminated -> left unchanged (no double ^Z).
        assert_eq!(terminate_text_file("B.SUB", b"X\x1a".to_vec()), b"X\x1a");
        // Match is case-insensitive on the extension.
        assert_eq!(terminate_text_file("notes.txt", b"X".to_vec()), b"X\x1a");
        // Binaries and unknown/dotless names are left byte-exact.
        assert_eq!(terminate_text_file("FOO.COM", vec![1, 2, 3]), vec![1, 2, 3]);
        assert_eq!(terminate_text_file("ZORK1.DAT", vec![5, 6]), vec![5, 6]);
        assert_eq!(terminate_text_file("READSUB", vec![7]), vec![7]); // no dot
    }
}
