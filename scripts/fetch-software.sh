#!/bin/sh
# Fetch a curated set of classic CP/M software into ./software/ for use with
# cpemu (e.g. `cargo run -- software/MBASIC.COM`).
#
# These are commercial-heritage binaries preserved at retroarchive.org. They are
# NOT committed to this repository (software/ is gitignored); this script just
# downloads them locally for you to run. Sources are credited per item.
set -e

DEST="$(cd "$(dirname "$0")/.." && pwd)/software"
ZIPS="$DEST/_zips"
mkdir -p "$ZIPS"

base="http://www.retroarchive.org/cpm"

# name | url | (files extracted into software/)
fetch() {
  url="$1"
  zip="$ZIPS/$(basename "$url")"
  echo "==> $url"
  curl -fsS -o "$zip" "$url"
  unzip -o -j -C -d "$DEST" "$zip" \
    '*.com' '*.COM' '*.dat' '*.DAT' '*.bas' '*.BAS' '*.hlp' '*.HLP' >/dev/null || true
}

fetch "$base/os/STDCPM22.ZIP"     # STAT, PIP, ED, DUMP, SUBMIT, ASM, LOAD, DDT...
fetch "$base/lang/mbasic.zip"     # Microsoft MBASIC 5.29
fetch "$base/games/zork123_80.zip" # Zork I, II, III (+ .DAT data files)
fetch "$base/lang/TP_301A.ZIP"    # Turbo Pascal 3.01a
fetch "$base/misc/LADDER.ZIP"     # Ladder (a Donkey-Kong-ish game)

echo
echo "Done. CP/M programs in $DEST:"
ls -1 "$DEST" | grep -viE '^_zips$' | sort
echo
echo "Run one with, e.g.:  cargo run -- software/MBASIC.COM"
