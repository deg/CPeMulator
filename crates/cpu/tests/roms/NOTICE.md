# Vendored: Z80 instruction-set exercisers (test fixtures)

These `.com` files are **third-party test programs**, used purely as input data
for the CPU validation harness in `../exercisers.rs`. They are loaded into the
emulator's memory and executed as guest Z80 programs; they are **not** compiled
or linked into any cpemu code. They are test fixtures, in the same sense as a
sample input file.

## Files

| File | What it is | Source (pinned) |
|------|------------|-----------------|
| `prelim.com` | Frank Cringle's *preliminary* Z80 test — small and fast; a good CI smoke test. | [`superzazu/z80`](https://github.com/superzazu/z80) `roms/` |
| `zexdoc.com` | The Z80 *documented* instruction-set exerciser. Checks every documented instruction's result and flags via CRCs. **This is cpemu's v1 CPU correctness gate.** | [`anotherlin/z80emu`](https://github.com/anotherlin/z80emu) `testfiles/` |
| `zexall.com` | The Z80 *all* exerciser — additionally checks undocumented flag behavior (incl. the internal WZ/MEMPTR register). A known-hard **stretch goal**, not a v1 requirement. | [`anotherlin/z80emu`](https://github.com/anotherlin/z80emu) `testfiles/` |

The exact upstream commit SHAs these were taken from are recorded in
`.upstream-shas`.

## License

The exercisers originate with **Frank D. Cringle's `zexall`/`zexdoc` (1994)** and
are licensed under the **GNU General Public License v2 (or later)** (later edits
by J.G. Harston). The full license statement appears in the original `.src`
files in the upstream repositories.

Because these programs are executed as data by the emulator and are not linked
into or distributed as part of the cpemu binary, their GPL terms do not extend to
cpemu's own (MIT-licensed) source. They are included here solely so the test
suite can validate the CPU.
