#!/bin/sh
# Fetch classic CP/M software into ./software/ for use with cpemu
# (e.g. `cargo run -- software/MBASIC.COM`, or `scripts/load-all.sh`).
#
# These are commercial-heritage binaries preserved at retroarchive.org. They are
# NOT committed to this repository (software/ is gitignored); this script only
# downloads them locally. Downloads use timeouts and tolerate failures (the
# archive is occasionally slow), so a stalled item is skipped, not fatal.

DEST="$(cd "$(dirname "$0")/.." && pwd)/software"
ZIPS="$DEST/_zips"
mkdir -p "$ZIPS"
base="http://www.retroarchive.org/cpm"

# Extract every file flat into software/ (apps like WordStar/dBASE/Multiplan need
# their full set of overlays, message files, and terminal-config files alongside
# the .COM; doc files are harmless extra). Skip only bulky manuals.
fetch() {
  url="$1"
  zip="$ZIPS/$(basename "$url")"
  printf '==> %-28s ' "$(basename "$url")"
  # Reuse a previously-downloaded zip (re-running is fast and avoids the flaky
  # archive); otherwise download it.
  if [ -s "$zip" ] || curl -fsS --connect-timeout 10 --max-time 90 -o "$zip" "$url" 2>/dev/null; then
    unzip -o -j -C -d "$DEST" "$zip" -x '*.pdf' '*.PDF' >/dev/null 2>&1
    echo "ok"
  else
    echo "SKIPPED (download failed/timed out)"
  fi
}

# --- operating system + utilities ---
fetch "$base/os/STDCPM22.ZIP"       # STAT, PIP, ED, DUMP, SUBMIT, ASM, LOAD, DDT...

# --- languages ---
fetch "$base/lang/mbasic.zip"       # Microsoft MBASIC 5.29
fetch "$base/lang/TP_301A.ZIP"      # Turbo Pascal 3.01a

# --- games ---
fetch "$base/games/zork123_80.zip"  # Zork I, II, III (+ data files)
fetch "$base/misc/LADDER.ZIP"       # Ladder
fetch "$base/games/sargon.zip"      # Sargon chess
fetch "$base/games/nemesis.zip"     # Nemesis (dungeon adventure)
fetch "$base/games/manygame.zip"    # a collection of small games

# --- iconic productivity (full-screen: best with a matching terminal) ---
fetch "$base/text/Ws4_mbee.zip"     # WordStar 4 (Microbee — ADM-3A-compatible)
fetch "$base/business/Suprcalc.zip" # SuperCalc spreadsheet
fetch "$base/business/MULTPLAN.ZIP" # Microsoft Multiplan
fetch "$base/dbase/dbaseii.zip"     # dBASE II database

# Write the committed guide (scripts/programs.txt) onto the disk as PROGRAMS.TXT,
# converting to CR/LF and adding a trailing ^Z so CP/M's TYPE displays it cleanly.
# (The guide's source is version-controlled; software/PROGRAMS.TXT is generated.)
guide="$(cd "$(dirname "$0")" && pwd)/programs.txt"
if [ -f "$guide" ]; then
  awk '{ printf "%s\r\n", $0 }' "$guide" > "$DEST/PROGRAMS.TXT"
  printf '\032' >> "$DEST/PROGRAMS.TXT"   # ^Z end-of-text marker for CP/M
fi

echo
echo "Programs in $DEST:"
ls -1 "$DEST" | grep -viE '^_zips$' | sort | column 2>/dev/null || ls -1 "$DEST" | grep -viE '^_zips$' | sort
echo
echo "Boot with everything:  scripts/load-all.sh"
