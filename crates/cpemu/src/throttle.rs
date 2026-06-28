// SPDX-License-Identifier: MIT
// Copyright (c) 2026 David Goldfarb <deg@degel.com>

//! Speed throttle: pace emulation to a target CPU clock.
//!
//! The emulator executes instructions as fast as the host can, which is far
//! faster than a real Z80 — so software that times itself with CPU delay loops
//! (most action games, like Ladder) runs absurdly fast. The throttle slows the
//! machine to a chosen clock rate by comparing *emulated* time (the CPU's
//! accumulated T-states divided by the target frequency) against *real* elapsed
//! time, and sleeping when emulation gets ahead.
//!
//! Blocking on console input (the CPU is parked in `CONIN`) makes real time race
//! ahead of emulated time; rather than then sprinting to "catch up", the
//! throttle resyncs its baseline when it falls far behind. Programs that don't
//! burn cycles waiting (the utilities, MBASIC at a prompt) are unaffected — they
//! simply never get far enough ahead to be slept.

use std::time::{Duration, Instant};

/// If real time runs this far ahead of emulated time (e.g. after waiting for a
/// keypress), resync rather than running fast to catch up.
const RESYNC_THRESHOLD: Duration = Duration::from_millis(100);

/// Paces emulation to `hz` T-states per second.
pub struct Throttle {
    hz: u64,
    /// Re-check (and possibly sleep) only this many cycles apart, to keep the
    /// per-instruction overhead negligible.
    interval: u64,
    start: Instant,
    base_cycles: u64,
    next_check: u64,
}

impl Throttle {
    /// Create a throttle targeting `hz` cycles per second (e.g. `4_000_000`).
    pub fn new(hz: u64) -> Self {
        let hz = hz.max(1);
        Self {
            hz,
            interval: (hz / 1000).max(1), // check ~every millisecond of emulated time
            start: Instant::now(),
            base_cycles: 0,
            next_check: 0,
        }
    }

    /// Emulated time represented by `cycles - base_cycles` at the target clock.
    fn emulated(&self, cycles: u64) -> Duration {
        let elapsed = cycles.saturating_sub(self.base_cycles);
        let nanos =
            (u128::from(elapsed) * 1_000_000_000 / u128::from(self.hz)).min(u128::from(u64::MAX));
        Duration::from_nanos(nanos as u64)
    }

    /// Call with the CPU's total T-state count after each step. Sleeps if the
    /// emulation has run ahead of real time; resyncs if it has fallen far behind.
    pub fn sync(&mut self, cycles: u64) {
        if cycles < self.next_check {
            return;
        }
        self.next_check = cycles.saturating_add(self.interval);

        let emulated = self.emulated(cycles);
        let real = self.start.elapsed();
        if emulated > real {
            std::thread::sleep(emulated - real);
        } else if real - emulated > RESYNC_THRESHOLD {
            self.start = Instant::now();
            self.base_cycles = cycles;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emulated_time_matches_clock() {
        let t = Throttle::new(4_000_000); // 4 MHz
        // 4,000,000 cycles == 1 second.
        assert_eq!(t.emulated(4_000_000), Duration::from_secs(1));
        // 2,000,000 cycles == 0.5 s.
        assert_eq!(t.emulated(2_000_000), Duration::from_millis(500));
    }

    #[test]
    fn sync_does_not_block_when_behind() {
        // With an enormous target clock, emulated time is essentially zero, so
        // the throttle is always "behind" real time and must never sleep.
        let mut t = Throttle::new(1_000_000_000_000); // 1 THz
        let before = Instant::now();
        for c in (0..1_000_000).step_by(1000) {
            t.sync(c);
        }
        assert!(before.elapsed() < Duration::from_millis(50));
    }
}
