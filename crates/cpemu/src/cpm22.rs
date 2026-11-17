// SPDX-License-Identifier: MIT
// Copyright (c) 2026 David Goldfarb <deg@degel.com>

//! The genuine Digital Research CP/M 2.2 operating system, as bytes.
//!
//! The CCP (command processor) and BDOS (kernel) are assembled from the vendored
//! source in `third-party/cpm22` at this emulator's memory-map origins (CCP at
//! `0xE400`, BDOS at `0xEC00`; see [`crate::config`]) and committed, so a normal
//! build needs no assembler (`make cpm-binaries` regenerates them). They are
//! compiled into the library so every front end — the CLI, the examples, the
//! browser build — boots the same system without reading any file at run time.
//! [`crate::disk::Disk::make_system_disk`] writes them onto a disk's system
//! tracks, from where the BIOS boots them.

/// The CP/M 2.2 CCP, assembled for `0xE400` (2048 bytes).
pub const CCP: &[u8] = include_bytes!("../../../third-party/cpm22/ccp.bin");
/// The CP/M 2.2 BDOS, assembled for `0xEC00` (3584 bytes).
pub const BDOS: &[u8] = include_bytes!("../../../third-party/cpm22/bdos.bin");
