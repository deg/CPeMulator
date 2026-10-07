# Moving files between the host and CP/M

How to get files into and out of CPeMulator, as of 2026-09-23. In short:
CPeMulator can get files *into* CP/M. It has no built-in way to get them back
out, and the serial/printer devices aren't connected to host files yet.

Sources: `crates/cpemu/src/main.rs`, `crates/cpemu/src/bios.rs`,
`crates/cpemu/src/disk.rs`.

---

## 1. Importing text files

Any file named on the command line is copied onto drive A: before boot:

```bash
cargo run -- notes.txt prog.bas                  # onto a fresh temporary A:
cargo run -- --disk=work.dsk notes.txt           # onto a saved image
```

`make play` does the same for everything in `software/`, so dropping a file
there works too.

Some details:

- **`^Z` end marker.** Files ending in `.TXT`, `.SUB`, `.DOC`, `.PRN` or `.ASC`
  get a `^Z` added (`main.rs:100`). Without it, `TYPE`, `ED` and `SUBMIT` read
  past the end of the text into garbage. Other extensions are copied byte for
  byte, so a `.BAS` or `.C` source file gets no `^Z`.
- **Line endings are not converted.** CP/M text expects CR/LF, and a Unix file
  with bare LF endings goes in as-is. Some CP/M programs will handle that badly,
  so run `unix2dos` on it first to be safe.
- **Name clash (a bug).** `add_file` doesn't check for a file that already
  exists. Importing `FOO.TXT` onto a `--disk` image that already has one creates
  a second directory entry with the same name instead of replacing it.
- **Everything goes to A:, user area 0.** Other drives can't be targeted from
  the command line.

## 2. Can `.dsk` files be modified from the host?

Yes. They're plain, headerless images: 128-byte sectors in order, no skew (the
BIOS's `SECTRAN` is identity), and a standard CP/M 2.2 directory. The emulator
chooses the format from the exact file size, so **the size must never change**.

| Format | Geometry | Size |
| ------ | -------- | ---- |
| 8 MB hard disk (default) | 64 sectors/track, 2 KB blocks, 4080 blocks, 1024 directory entries, 2 system tracks | 8,372,224 bytes (exactly 1022 tracks) |
| `--floppy` | 26 sectors/track, 1 KB blocks, 243 blocks, 64 directory entries, 2 system tracks | 255,488 bytes |

The emulator has no host-side tool for reading or writing images. The disk code
has a `read_file` function, but only the tests use it.

### Using cpmtools (untested)

On paper, [cpmtools](http://www.moria.de/~michael/cpmtools/) (`cpmls`, `cpmcp`,
`cpmrm`) should handle the 8 MB format with a custom diskdef. **This has not
been tested.**

```
diskdef cpemu8mb
  seclen 128
  tracks 1022
  sectrk 64
  blocksize 2048
  maxdir 1024
  skew 0
  boottrk 2
  os 2.2
end
```

Then, for example:

```bash
cpmls -f cpemu8mb work.dsk                  # list files
cpmcp -f cpemu8mb work.dsk 0:REPORT.TXT .   # copy a file out to the host
cpmcp -f cpemu8mb work.dsk notes.txt 0:     # copy a file in
```

### Cautions

- **The floppy format is not a standard 8-inch image.** The standard `ibm-3740`
  definition uses skew 6 and a 256,256-byte image. Ours has no skew and is 768
  bytes short, so the stock definition won't read it.
- **Don't edit an image while the emulator has it mounted.** The emulator writes
  its whole in-memory copy back on exit, which silently overwrites any host-side
  changes.

Mapping a host directory straight to a CP/M drive is on the roadmap as epic
`cpemu-ntn`, but deferred. It's hard with a genuine BDOS; see
`dev-docs/roadmap.md` in the repository.

## 3. Serial I/O: printer, punch, reader

Not implemented yet. The BIOS handles these devices like this
(`bios.rs:350-391`):

| Device | Current behavior |
|---|---|
| **LIST** (printer) | Goes to the console, mixed in with screen output. `LISTST` always reports ready. |
| **PUNCH** | Output is discarded. |
| **READER** | Always returns `^Z` (end of tape). |

So `PIP LST:=FILE.TXT` or `^P` echo prints to the screen, and
`PIP FILE.TXT=RDR:` gets an empty file.

### Possible future work

These traps already run in Rust, so connecting them to host files would be a
small change. It could be flags such as:

- `--list=out.txt` sends printer output to a host file.
- `--punch=out.bin` sends punch output to a host file.
- `--reader=in.txt` feeds a host file to the reader, returning `^Z` at end of
  file.

Each would have a config-file equivalent. This would also give a way to get text
out (`PIP PUN:=FILE.TXT`) without touching the disk images.
