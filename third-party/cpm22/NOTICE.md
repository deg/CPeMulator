# Vendored: CP/M 2.2 CCP + BDOS source

This directory contains **third-party** source for the Digital Research CP/M 2.2
command processor (CCP) and kernel (BDOS), vendored into CPeMulator so the emulator
can boot a genuine CP/M 2.2 system.

## What's here

| File          | Origin |
|---------------|--------|
| `ccp.asm`     | CP/M 2.2 CCP source, reformatted for Macro Assembler AS |
| `bdos.asm`    | CP/M 2.2 BDOS source, reformatted for Macro Assembler AS |
| `LICENSE.txt` | Digital Research / DRDOS Inc. redistribution grant |
| `README.md`   | Upstream project README |
| `.upstream-sha` | Exact upstream commit these files were taken from |

## Provenance

Source: [`brouhaha/cpm22`](https://github.com/brouhaha/cpm22) by Eric Smith,
pinned at the commit recorded in `.upstream-sha`. That project reformatted the
original Digital Research `os2ccp.asm` / `os3bdos.asm` for cross-assembly with
[Macro Assembler AS](http://john.ccac.rwth-aachen.de:8000/as/) and **verified
that it assembles to the exact binary present on several real CP/M 2.2 disks**,
except for the six-byte serial numbers embedded in the CCP and BDOS.

## License

CP/M and its derivatives are redistributable. Per `LICENSE.txt`, Bryan Sparks
— President of DRDOS Inc., successor in interest to Digital Research — granted
(2022-07-07): *"a right to use, distribute, modify, enhance, and otherwise make
available in a nonexclusive manner CP/M and its derivatives."* See `LICENSE.txt`
for the verbatim text; canonical copy at <http://www.cpm.z80.de/license.html>.

## The assembled binaries (`ccp.bin`, `bdos.bin`)

| File | Origin | Size |
|------|--------|------|
| `ccp.bin`  | `0xE400` | 2048 bytes (`0x800`) |
| `bdos.bin` | `0xEC00` | 3584 bytes (`0xE00`) |

These are the genuine CCP and BDOS, assembled from the source here at the origins
of CPeMulator's memory map (defined in `crates/cpemu/src/config.rs`). They are
committed so that normal builds and CI never need an assembler.

**How they were generated** (reproducible via `make cpm-binaries`):

- Assembler: **Macro Assembler AS** (`asl`), V1.42 Beta, built from
  `http://john.ccac.rwth-aachen.de:8000/ftp/as/source/c_version/asl-current.tar.gz`
  with `make binaries` (the `Makefile.def-arm-osx` sample on macOS; no texlive
  needed — that is only for the docs). Install `asl` and `p2bin` on your `PATH`.
- Commands (also encoded in the `cpm-binaries` Makefile target):
  ```sh
  asl -D origin=0e400h -o ccp.p  -L -OLIST ccp.lst  ccp.asm
  p2bin -l '$00' -r '$e400-$ebff' ccp.p  ccp.bin     # CCP  @ 0xE400, 0x800 bytes
  asl -D origin=0ec00h -o bdos.p -L -OLIST bdos.lst bdos.asm
  p2bin -l '$00' -r '$ec00-$f9ff' bdos.p bdos.bin    # BDOS @ 0xEC00, 0xE00 bytes
  ```

The `-r` range pads each image to its full region; the underlying code is
slightly smaller (CCP 1887 bytes, BDOS 3453 bytes). To re-target a different
memory map, change the origins here and in `config.rs` together.
