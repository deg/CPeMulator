#!/bin/sh
# Boot cpemu with a full load-out of fetched software on drive A:.
#
# A single 8" CP/M floppy holds ~241 KB, and the fetched software totals ~460 KB
# (the three Zork data files alone are ~250 KB), so not everything fits at once.
# This loads a curated set that does: the core CP/M utilities, MBASIC, Turbo
# Pascal, Ladder, and Zork I. Any flags you pass (e.g. --mhz=2 or --raw) are
# forwarded to the emulator.
#
# To play a different Zork:  cargo run -- software/ZORK2.COM software/ZORK2.DAT
set -e

DIR="$(cd "$(dirname "$0")/.." && pwd)/software"
[ -d "$DIR" ] || { echo "No software/ dir — run scripts/fetch-software.sh first."; exit 1; }

# Priority order (small essentials first); the emulator reports anything that
# doesn't fit, so the important tools always make it.
PROGS="STAT.COM PIP.COM ED.COM DDT.COM DUMP.COM ASM.COM LOAD.COM SUBMIT.COM XSUB.COM \
       MBASIC.COM TURBO.COM LADDER.COM LADDER.DAT ZORK1.COM ZORK1.DAT"

files=""
missing=""
for p in $PROGS; do
  if [ -f "$DIR/$p" ]; then
    files="$files $DIR/$p"
  else
    missing="$missing $p"
  fi
done
[ -n "$missing" ] && echo "note: not found in software/:$missing"

# shellcheck disable=SC2086
exec cargo run --release -p cpemu -- "$@" $files
