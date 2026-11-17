# Vendored: the CP/M 2.2 utility programs

These are Digital Research's own CP/M 2.2 transient programs, unmodified. The
browser build puts them on its default drive A: so a visitor has real tools to
try (bead cpemu-kb9). Nothing else in this repository is copied onto a disk by
default; other software stays the user's to fetch or upload.

## Source

`cpm22-b.zip` from the Unofficial CP/M Web Site, which DRI's successor licensed
to distribute CP/M: <http://www.cpm.z80.de/download/cpm22-b.zip>, downloaded
2026-11-17 (zip SHA-256
`88735cca77e786505f53a63746ddbc5ca221eda79fd822fde5dc9f00a5bfba6f`). Its
`READ.ME` says it holds the files from the CP/M 2.2 distribution disk for a
"XEROX 1800" system. Seven of the nine programs are byte-identical to those in
retroarchive.org's `STDCPM22.ZIP` (which `scripts/fetch-software.sh` uses); DDT
and DUMP are different builds.

| File | Bytes | SHA-256 | What it is |
|------|------:|---------|------------|
| `PIP.COM` | 7424 | `7f9e12a92e2bcfd814b5b680a2f7d5c2a2c50c9a5ef94a6891dcaa3527f08ec2` | Peripheral Interchange Program: copy files |
| `STAT.COM` | 5248 | `614d0b1d66466177e5b2bf585251d53d1e24eda34ada136b4ae178ab5944dc73` | disk and file statistics, free space |
| `ED.COM` | 6656 | `0397b96b6d48ba92a0b41cdf974bf8ad31e4d49783f1ff7eed9e701b6a8870db` | the line editor |
| `ASM.COM` | 8192 | `ef403388a04f18d735984fe497f9fa5dbb48f114b52dab323e33e82073133c2c` | 8080 assembler (`.ASM` to `.HEX`) |
| `LOAD.COM` | 1792 | `c885d061a5dcb3830ab1c29a13e34d3c0560cc3faf569bc55b195c0330a9cbd6` | `.HEX` to `.COM` |
| `DDT.COM` | 4864 | `bc4b0f085f8d1e37caf2a13850c70afa07acb626b22b1a7cb63368b3334dd448` | Dynamic Debugging Tool |
| `DUMP.COM` | 512 | `9a99911c0fe0aaec22fdec61b1b2b03dd012ea12d20cc17f21c6f6d9c5399fe3` | hex dump of a file |
| `SUBMIT.COM` | 1280 | `4c3fec22ebca595b03c279eb21b19aa4b2dab139741ed29172ca6fa4c43e7066` | run a batch file of commands |
| `XSUB.COM` | 768 | `817a16b595ea6de8df0dd5e808f89a49e85dea546673283d759d01e960598bed` | feed SUBMIT input to programs |
| `DUMP.ASM` | 4224 | `789e9f260df49e4d77d0ad5477de5429d36c677347ad6b9c875727722c6ea85a` | source of DUMP, DRI's sample program |

Left out from the same zip: `CPM.SYS` (a system image for that machine),
`DSKMAINT.COM` (its disk formatter, which drives its hardware), and the BIOS
skeleton sources (`BIOS.ASM`, `DEBLOCK.ASM`, `DISKDEF.LIB`), which document
porting CP/M rather than using it.

## License

The same grant as the CCP and BDOS beside this directory; see
[`../LICENSE.txt`](../LICENSE.txt) and [`../NOTICE.md`](../NOTICE.md). Bryan
Sparks, President of DRDOS Inc., successor in interest to Digital Research,
granted (2022-07-07) "a right to use, distribute, modify, enhance, and otherwise
make available in a nonexclusive manner CP/M and its derivatives."
