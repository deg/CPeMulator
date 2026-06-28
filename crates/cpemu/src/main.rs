//! `cpemu` — CLI entry point for the CP/M emulator.
//!
//! This is a skeleton entry point. The runtime (virtual machine, HLE BIOS,
//! disk subsystem, console, and monitor) is built out under their respective
//! epics. For now it only proves the workspace wires together.

fn main() {
    println!(
        "cpemu {} (cpu core {}) — skeleton; emulator not yet implemented.",
        env!("CARGO_PKG_VERSION"),
        cpemu_cpu::VERSION,
    );
}
