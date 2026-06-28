//! `cpemu-cpu` — the Z80/8080 CPU core for the cpemu CP/M emulator.
//!
//! The Z80 is implemented as a superset that also runs 8080 code. The core is
//! kept dependency-light so it can be validated in isolation against the
//! standard exercisers (zexdoc is the v1 correctness gate).
//!
//! The execution framework is in place ([`Cpu`], [`Bus`], [`Registers`], and an
//! observable [`StepResult`]); the full instruction set is filled in under the
//! "Z80/8080 CPU core" epic, replacing the [`Stop::Illegal`] opcodes.

#![forbid(unsafe_code)]

mod alu;
mod bus;
mod cb;
mod cpu;
mod ed;
mod index;
mod interrupt;
mod registers;

pub use bus::{Bus, PlainBus};
pub use cpu::{Cpu, StepResult, Stop};
pub use registers::{Registers, flag};

/// Crate version string, for banners and diagnostics.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
