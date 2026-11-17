# Moving files between the host and CP/M

How to get files into and out of CPeMulator, as of 2026-11-17. In short: the
desktop version (the CLI) can get files *into* CP/M, but has no built-in way to
get them back out. The browser version at <https://cpemulator.degel.com/> does
both. The serial/printer devices aren't connected to host files in either.

This started as the answer to a user's question; `host-file-io.docx` and
`host-file-io.pdf` beside this file are that answer as sent on 2026-09-23, kept
as a record. This file is the one kept current.

Sources: `crates/cpemu/src/main.rs`, `crates/cpemu/src/bios.rs`,
`crates/cpemu/src/disk.rs`, `crates/cpemu/src/session.rs`,
`crates/web/src/lib.rs`, `web/www/main.js`.

---

## 1. Importing files (desktop)

Any file named on the command line is copied onto drive A: before boot:

```bash
cargo run -- notes.txt prog.bas                  # onto a fresh temporary A:
cargo run -- --disk=work.dsk notes.txt           # onto a saved image
```

`make play` does the same for everything in `software/`, so dropping a file
there works too.

Some details:

- **`^Z` end marker.** Files ending in `.TXT`, `.SUB`, `.DOC`, `.PRN` or `.ASC`
  get a `^Z` added (`disk::terminate_text_file`). Without it, `TYPE`, `ED` and
  `SUBMIT` read past the end of the text into garbage. Other extensions are
  copied byte for byte, so a `.BAS` or `.C` source file gets no `^Z`.
- **Line endings are not converted.** CP/M text expects CR/LF, and a Unix file
  with bare LF endings goes in as-is. Some CP/M programs will handle that badly,
  so run `unix2dos` on it first to be safe.
- **Name clash (a bug in the CLI).** The CLI imports with `Disk::add_file`, which
  doesn't check for a file that already exists. Importing `FOO.TXT` onto a
  `--disk` image that already has one creates a second directory entry with the
  same name instead of replacing it. (`Disk::put_file`, which the browser uses,
  replaces it.)
- **Names CP/M can't type are refused**: spaces, the characters the command
  processor treats as separators or wildcards (`< > . , ; : = ? * [ ]`), and
  anything outside printable ASCII. The CLI reports the file and skips it.
- **Everything goes to A:, user area 0.** Other drives can't be targeted from
  the command line.

## 2. In the browser

The page's **Your disk** panel moves files both ways. See
[browser.md](browser.md) for how it works.

**In:**

- **Add files**, or drop files anywhere on the page: programs, data files, or
  `.zip` archives, which are unpacked onto A: with their folders flattened
  (old DOS-era PKZIP archives included). The same `^Z` rule applies, a file
  replaces one of the same name, and names CP/M can't type are refused with a
  message. Adding files warm-boots CP/M so it sees them, and asks first if a
  program is running, since that ends it.
- **Get software** fetches the desktop version's software library; see
  [browser.md](browser.md).
- **Load disk** replaces drive A: with a `.dsk` image, of either size below.

**Out:**

- **Click a file** in the Your disk list to save a copy of it to your computer.
  CP/M records a file's length only to the nearest 128-byte record, so the copy
  is padded to a multiple of 128 bytes (a text file with `^Z`s, the way CP/M
  wrote it).
- **Save disk** saves all of drive A: as `cpemulator-A.dsk`.

**Between the two versions:** the images are the same format. A disk saved from
the browser (the 8 MB format) mounts in the CLI with
`cpemu --disk=cpemulator-A.dsk`, and a CLI image loads in the browser with Load
disk.

The disk is also kept in the browser between visits, but browsers may clear
their storage on their own, so the saved `.dsk` is the real copy.

## 3. Can `.dsk` files be modified from the host?

Yes. They're plain, headerless images: 128-byte sectors in order, no skew (the
BIOS's `SECTRAN` is identity), and a standard CP/M 2.2 directory. The emulator
chooses the format from the exact file size, so **the size must never change**.

| Format | Geometry | Size |
| ------ | -------- | ---- |
| 8 MB hard disk (default, and the browser's) | 64 sectors/track, 2 KB blocks, 4080 blocks, 1024 directory entries, 2 system tracks | 8,372,224 bytes (exactly 1022 tracks) |
| `--floppy` | 26 sectors/track, 1 KB blocks, 243 blocks, 64 directory entries, 2 system tracks | 255,488 bytes |

The desktop version has no host-side tool for reading or writing images. In the
browser, the Your disk panel is one (it uses the disk code's `list_files` and
`read_file`). Outside the emulator, try cpmtools.

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
`cpemu-ntn`, deferred until a full filesystem is needed. With a genuine BDOS it
amounts to a miniature filesystem; see [roadmap.md](roadmap.md).

## 4. Serial I/O: printer, punch, reader

Not implemented yet, in either version. The BIOS handles these devices like this
(the LIST, PUNCH, READER and LISTST vectors in `CpmBios`'s trap handler,
`bios.rs`):

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
