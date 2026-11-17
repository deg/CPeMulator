// SPDX-License-Identifier: MIT
// Copyright (c) 2026 David Goldfarb <deg@degel.com>

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
// The host-terminal console and the speed throttle are native-only: the console
// needs crossterm (raw mode) and a reader thread, and the throttle needs
// `std::time::Instant` and `thread::sleep`, which panic on wasm32. The browser
// front end has its own console and paces itself per animation frame.
#[cfg(not(target_arch = "wasm32"))]
pub mod console;
pub mod cpm22;
pub mod debugger;
pub mod disk;
pub mod executor;
pub mod keys;
pub mod machine;
pub mod memory;
pub mod monitor;
pub mod session;
pub mod terminal;
#[cfg(not(target_arch = "wasm32"))]
pub mod throttle;
