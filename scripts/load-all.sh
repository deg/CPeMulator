#!/bin/sh
# Boot cpemu with ALL the fetched software loaded onto drive A:.
#
# The default drive is now an 8 MB hard disk (the CP/M 2.2 maximum), which holds
# the entire ~460 KB software collection with room to spare. Any flags you pass
# (e.g. --mhz=2, --raw, --floppy) are forwarded to the emulator.
set -e

DIR="$(cd "$(dirname "$0")/.." && pwd)/software"
[ -d "$DIR" ] || { echo "No software/ dir — run scripts/fetch-software.sh first."; exit 1; }

files=""
for f in "$DIR"/*.COM "$DIR"/*.DAT; do
  [ -f "$f" ] && files="$files $f"
done
[ -n "$files" ] || { echo "No .COM/.DAT files in $DIR — run scripts/fetch-software.sh first."; exit 1; }

# shellcheck disable=SC2086
exec cargo run --release -p cpemu -- "$@" $files
