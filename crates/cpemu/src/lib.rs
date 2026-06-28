//! `cpemu` — the system layer of the CP/M emulator.
//!
//! This crate builds the virtual machine *around* the [`cpemu_cpu`] CPU core:
//! the 64 KB of [`memory`], the [`machine`] that presents memory and I/O to the
//! CPU as a `Bus` and drives execution, and (later) the HLE BIOS, disk
//! subsystem, console, and monitor. The accompanying binary (`main.rs`) is the
//! CLI front end.
//!
//! See `dev-docs/architecture.md` for how these layers fit together.

#![forbid(unsafe_code)]

pub mod bios;
pub mod config;
pub mod console;
pub mod disk;
pub mod executor;
pub mod machine;
pub mod memory;
pub mod terminal;
pub mod throttle;
