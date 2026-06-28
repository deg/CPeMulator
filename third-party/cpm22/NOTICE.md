# Vendored: CP/M 2.2 CCP + BDOS source

This directory contains **third-party** source for the Digital Research CP/M 2.2
command processor (CCP) and kernel (BDOS), vendored into cpemu so the emulator
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

## Why source, not a prebuilt binary (yet)

The CCP/BDOS assemble at an **origin that depends on the target machine's memory
size** (e.g. the upstream example builds a 44K system with CCP@`0x9400`,
BDOS@`0x9C00`). A committed binary would bake in a memory map we have not yet
designed. The binary-vs-assemble decision and the chosen origin are deferred to
the **HLE BIOS & authentic boot** epic (`cpemu-pal`), where the memory layout is
fixed; the assembler (Macro Assembler AS / `asl`) will be added to tooling then.
