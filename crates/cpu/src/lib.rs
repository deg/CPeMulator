//! `cpemu-cpu` — the Z80/8080 CPU core for the cpemu CP/M emulator.
//!
//! The Z80 is implemented as a superset that also runs 8080 code. The core is
//! kept dependency-light so it can be validated in isolation against the
//! standard exercisers (zexdoc is the v1 correctness gate).
//!
//! This crate is currently a skeleton; the CPU implementation lands under the
//! "Z80/8080 CPU core" epic (registers/flags/decode, the 8080 base set, the
//! Z80 prefixes, interrupts, and the disassembler).

#![forbid(unsafe_code)]

/// Crate version string, for banners and diagnostics.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crate_builds() {
        assert!(!VERSION.is_empty());
    }
}
